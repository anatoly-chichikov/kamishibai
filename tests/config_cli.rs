//! Offline CLI tests for `config` and the `new` language refusal — no network
//! beyond 127.0.0.1. `KAMISHIBAI_DATA` redirects preferences to a temp dir, so
//! the real user config is never read or written.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a binary invocation isolated to temp data and cache dirs.
fn cli(data: &Path, cache: &Path) -> Command {
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    command
        .env("KAMISHIBAI_DATA", data)
        .env("KAMISHIBAI_CACHE", cache)
        .env_remove("GEMINI_API_KEY");
    command
}

/// Run one isolated invocation to completion and capture its output.
fn run(data: &Path, cache: &Path, args: &[&str]) -> Output {
    cli(data, cache)
        .args(args)
        .output()
        .expect("the command must run")
}

/// A 127.0.0.1 listener that answers every request with one HTTP `status`, so
/// the key-validation probe sees a deterministic accept or reject.
fn gemini_answering(status: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the listener must bind");
    let port = listener
        .local_addr()
        .expect("the listener has an address")
        .port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut scratch = [0u8; 65536];
            let _ = stream.read(&mut scratch);
            let response =
                format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}");
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn a_word_session_without_a_saved_language_is_refused() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = run(data.path(), cache.path(), &["new", "--word", "chat"]);
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert_eq!(
        (output.status.code(), stderr.contains("config --known")),
        (Some(2), true),
        "a word session with no saved language must be refused with guidance toward config"
    );
}

#[test]
fn a_word_session_with_a_language_but_no_key_points_to_config_key() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    run(data.path(), cache.path(), &["config", "--known", "en"]);
    let output = run(data.path(), cache.path(), &["new", "--word", "chat"]);
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert_eq!(
        (output.status.code(), stderr.contains("config --key")),
        (Some(2), true),
        "a word session with a saved language but no key must be refused with guidance toward config --key"
    );
}

#[test]
fn config_saves_the_known_language_as_a_confirmed_choice() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    run(data.path(), cache.path(), &["config", "--known", "ru"]);
    let shown = run(data.path(), cache.path(), &["config", "--json"]);
    let document = String::from_utf8(shown.stdout).expect("stdout must be UTF-8");
    assert_eq!(
        document.trim(),
        r#"{"ok":true,"known":"RU","key_saved":false}"#,
        "config --json must report the saved language uppercased, without a derived confirmed flag"
    );
}

#[test]
fn config_refuses_an_unknown_language() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = run(data.path(), cache.path(), &["config", "--known", "zz"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown language code must be refused with the usage exit code"
    );
}

#[test]
fn config_saves_a_verified_key_without_echoing_its_value() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("200 OK");
    cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "secret-key"])
        .assert()
        .success();
    let shown = run(data.path(), cache.path(), &["config", "--json"]);
    let document = String::from_utf8(shown.stdout).expect("stdout must be UTF-8");
    assert!(
        document.contains("\"key_saved\":true") && !document.contains("secret-key"),
        "a verified key must be saved and reported without echoing its value, got {document}"
    );
}

#[test]
fn config_refuses_a_key_the_probe_rejects() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("401 Unauthorized");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "bad-key"])
        .output()
        .expect("the command must run");
    assert_eq!(
        output.status.code(),
        Some(2),
        "a key the Gemini probe rejects must be refused, never saved"
    );
}
