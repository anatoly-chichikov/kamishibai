//! Offline CLI tests for `config` and the `new` language refusal — no network
//! beyond 127.0.0.1. `KAMISHIBAI_DATA` redirects preferences to a temp dir, so
//! the real user config is never read or written.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;
use std::sync::{Arc, Barrier};

use assert_cmd::Command;
use serde_json::Value;
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
            let body = match status.split_once(' ').map(|(code, _)| code) {
                Some("200") => {
                    r#"{"models":[{"name":"models/gemini-3.6-flash","supportedGenerationMethods":["generateContent"]}]}"#
                }
                Some("400") => r#"{"error":{"status":"INVALID_ARGUMENT"}}"#,
                Some("401") => r#"{"error":{"status":"UNAUTHENTICATED"}}"#,
                Some("403") => r#"{"error":{"status":"PERMISSION_DENIED"}}"#,
                _ => "{}",
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Start one local intake response for a non-billable env-precedence test.
fn intake_answering() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the listener must bind");
    let port = listener
        .local_addr()
        .expect("the listener has an address")
        .port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the intake request must arrive");
        let mut scratch = [0u8; 65536];
        let _ = stream.read(&mut scratch);
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "{\"target_lang\":\"FR\",\"items\":[{\"term\":\"chat\",\"senses\":[{\"understanding\":\"a cat\",\"tag\":null}],\"selected\":0,\"ok\":true}]}"
                    }]
                }
            }]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("the intake response must be writable");
    });
    format!("http://127.0.0.1:{port}")
}

/// Parse the command's one JSON stdout document.
fn document(output: &Output) -> Value {
    serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be one JSON document")
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
fn a_missing_known_language_json_error_preserves_the_setup_hint() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = run(
        data.path(),
        cache.path(),
        &["new", "--word", "chat", "--json"],
    );
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("config --known EN --json")),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(2), true, Some(false)),
        "a missing-known JSON refusal must retain its actionable setup hint"
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
fn a_missing_key_json_error_preserves_the_setup_hint() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    run(data.path(), cache.path(), &["config", "--known", "en"]);
    let output = run(
        data.path(),
        cache.path(),
        &["new", "--word", "chat", "--json"],
    );
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("config --key - --json")),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(2), true, Some(false)),
        "a missing-key JSON refusal must retain its actionable setup hint"
    );
}

#[test]
fn config_saves_the_known_language_as_a_confirmed_choice() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    run(data.path(), cache.path(), &["config", "--known", "ru"]);
    let shown = run(data.path(), cache.path(), &["config", "--json"]);
    let document = document(&shown);
    assert_eq!(
        (
            document["known"].as_str(),
            document["key_saved"].as_bool(),
            document["credential_source"].as_str(),
            document["credential_present"].as_bool(),
            document["preferences_path"].as_str(),
        ),
        (
            Some("RU"),
            Some(false),
            Some("none"),
            Some(false),
            Some(
                data.path()
                    .join("kamishibai")
                    .join("preferences.json")
                    .to_str()
                    .expect("preferences path must be UTF-8")
            ),
        ),
        "config --json lost one of its additive setup fields"
    );
}

#[test]
fn config_without_a_saved_language_reports_known_as_null() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = run(data.path(), cache.path(), &["config", "--json"]);
    let document = document(&output);
    assert_eq!(
        document.as_object().and_then(|fields| fields.get("known")),
        Some(&Value::Null),
        "config --json omitted known instead of reporting an unconfigured language as null"
    );
}

#[test]
fn config_refuses_an_unknown_language() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = run(
        data.path(),
        cache.path(),
        &["config", "--known", "zz", "--json"],
    );
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["retryable"].as_bool(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("EN, ZH, ES, JA, FR, DE, RU, IT, PT, EL, NL")),
        ),
        (Some(2), Some("usage"), Some(false), true),
        "an unknown language refusal omitted its supported language codes"
    );
}

#[test]
fn config_saves_a_verified_key_without_echoing_its_value() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("200 OK");
    let saved = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "secret-key", "--json"])
        .output()
        .expect("the save command must run");
    let shown = run(data.path(), cache.path(), &["config", "--json"]);
    let saved_stdout = String::from_utf8(saved.stdout).expect("save stdout must be UTF-8");
    let saved_stderr = String::from_utf8(saved.stderr).expect("save stderr must be UTF-8");
    let shown_stdout = String::from_utf8(shown.stdout).expect("show stdout must be UTF-8");
    let names = std::fs::read_dir(data.path().join("kamishibai"))
        .expect("preferences directory must be readable")
        .map(|entry| {
            entry
                .expect("preferences entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        (
            saved.status.code(),
            shown_stdout.contains("\"key_saved\":true"),
            saved_stdout.contains("secret-key"),
            saved_stderr.contains("secret-key"),
            shown_stdout.contains("secret-key"),
            names.iter().any(|name| name.contains("secret-key")),
        ),
        (Some(0), true, false, false, false, false),
        "a verified key leaked through a process output or temporary filename"
    );
}

#[test]
fn config_refuses_a_key_the_probe_rejects() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("401 Unauthorized");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "bad-key", "--json"])
        .output()
        .expect("the command must run");
    assert_eq!(
        (
            output.status.code(),
            document(&output)["error"]["retryable"].as_bool(),
            document(&output)["error"]["hint"].is_string(),
        ),
        (Some(2), Some(false), true),
        "a rejected key must be a non-retryable usage error with a recovery hint"
    );
}

#[test]
fn malformed_header_keys_are_non_retryable_usage_errors() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let malformed = "malformed\nsecret-key";
    let output = cli(data.path(), cache.path())
        .args(["config", "--key", malformed, "--json"])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["retryable"].as_bool(),
            String::from_utf8_lossy(output.stdout.as_slice()).contains(malformed),
            String::from_utf8_lossy(output.stderr.as_slice()).contains(malformed),
        ),
        (Some(2), Some("usage"), Some(false), false, false),
        "a malformed header key was retryable, operational, or exposed"
    );
}

#[test]
fn generic_bad_requests_are_operational_not_invalid_keys() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("400 Bad Request");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "secret-key", "--json"])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(1), Some("operational"), Some(false)),
        "generic INVALID_ARGUMENT was misreported as an invalid API key"
    );
}

#[test]
fn rejected_key_validation_preserves_the_previous_saved_key() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let accepted = gemini_answering("200 OK");
    cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &accepted)
        .args(["config", "--key", "original-secret"])
        .output()
        .expect("the seed command must run");
    let rejected = gemini_answering("403 Forbidden");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &rejected)
        .args(["config", "--key", "replacement-secret", "--json"])
        .output()
        .expect("the rejected command must run");
    let stored = std::fs::read_to_string(data.path().join("kamishibai").join("preferences.json"))
        .expect("preferences must remain readable");
    assert_eq!(
        (
            output.status.code(),
            stored.contains("original-secret"),
            stored.contains("replacement-secret"),
            String::from_utf8_lossy(output.stdout.as_slice()).contains("replacement-secret"),
            String::from_utf8_lossy(output.stderr.as_slice()).contains("replacement-secret"),
        ),
        (Some(2), true, false, false, false),
        "rejected credential validation changed or exposed the saved key"
    );
}

#[test]
fn config_reports_environment_credentials_without_saving_them() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let output = cli(data.path(), cache.path())
        .env("GEMINI_API_KEY", "environment-secret")
        .args(["config", "--json"])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            document["key_saved"].as_bool(),
            document["credential_source"].as_str(),
            document["credential_present"].as_bool(),
            String::from_utf8_lossy(output.stdout.as_slice()).contains("environment-secret"),
        ),
        (Some(false), Some("environment"), Some(true), false),
        "config must report environment precedence without exposing or persisting its value"
    );
}

#[test]
fn transient_credential_failures_are_retryable_operational_errors() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("429 Too Many Requests");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "secret-key", "--json"])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(1), Some("operational"), Some(true)),
        "quota and transient provider failures must remain retryable operational errors"
    );
}

#[test]
fn unavailable_credential_models_are_not_misreported_as_bad_keys() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("404 Not Found");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "secret-key", "--json"])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(1), Some("operational"), Some(false)),
        "model availability failures must be non-retryable operational errors"
    );
}

#[test]
fn concurrent_config_updates_preserve_the_known_language_and_key() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("200 OK");
    let barrier = Arc::new(Barrier::new(2));
    std::thread::scope(|scope| {
        let data_path = data.path();
        let cache_path = cache.path();
        let known = Arc::clone(&barrier);
        scope.spawn(move || {
            known.wait();
            run(data_path, cache_path, &["config", "--known", "ru"])
        });
        let data_path = data.path();
        let cache_path = cache.path();
        let gemini = gemini.as_str();
        let key = Arc::clone(&barrier);
        scope.spawn(move || {
            key.wait();
            cli(data_path, cache_path)
                .env("KAMISHIBAI_GEMINI_URL", gemini)
                .args(["config", "--key", "concurrent-secret"])
                .output()
                .expect("the key command must run")
        });
    });
    let shown = run(data.path(), cache.path(), &["config", "--json"]);
    let document = document(&shown);
    assert_eq!(
        (
            document["known"].as_str(),
            document["key_saved"].as_bool(),
            document["credential_source"].as_str(),
        ),
        (Some("RU"), Some(true), Some("saved")),
        "concurrent config updates lost the language or the saved key"
    );
}

#[test]
fn corrupt_preferences_surface_the_path_and_recovery_hint() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let directory = data.path().join("kamishibai");
    std::fs::create_dir_all(directory.as_path()).expect("preferences directory must be created");
    let path = directory.join("preferences.json");
    std::fs::write(path.as_path(), "{not-json").expect("corrupt preferences must be written");
    let output = run(data.path(), cache.path(), &["config", "--json"]);
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(path.to_string_lossy().as_ref())),
            document["error"]["hint"].is_string(),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(1), true, true, Some(false)),
        "corrupt preferences must not collapse to first-run defaults"
    );
}

#[test]
fn environment_credentials_bypass_corrupt_saved_preferences() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let directory = data.path().join("kamishibai");
    std::fs::create_dir_all(directory.as_path()).expect("preferences directory must be created");
    std::fs::write(directory.join("preferences.json"), "{not-json")
        .expect("corrupt preferences must be written");
    let gemini = intake_answering();
    let output = cli(data.path(), cache.path())
        .env("GEMINI_API_KEY", "environment-secret")
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .env("KAMISHIBAI_OUTPUT", cache.path().join("out"))
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--learning",
            "FR",
            "--json",
        ])
        .output()
        .expect("the command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["pair"]["known"].as_str(),
            document["pair"]["learning"].as_str(),
        ),
        (Some(0), Some("RU"), Some("FR")),
        "an environment key must win without parsing corrupt saved preferences"
    );
}

#[cfg(windows)]
#[test]
fn key_persistence_fails_closed_without_windows_acl_enforcement() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("200 OK");
    let output = cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .env("PATH", "")
        .args(["config", "--key", "must-not-persist", "--json"])
        .output()
        .expect("the command must run");
    assert_eq!(
        (
            output.status.code(),
            data.path()
                .join("kamishibai")
                .join("preferences.json")
                .exists(),
            String::from_utf8_lossy(output.stdout.as_slice()).contains("must-not-persist"),
            String::from_utf8_lossy(output.stderr.as_slice()).contains("must-not-persist"),
        ),
        (Some(1), false, false, false),
        "a key was persisted or exposed without verified Windows ACL support"
    );
}

#[cfg(windows)]
#[test]
fn stamped_windows_preferences_read_without_acl_enforcement() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let gemini = gemini_answering("200 OK");
    cli(data.path(), cache.path())
        .env("KAMISHIBAI_GEMINI_URL", &gemini)
        .args(["config", "--key", "saved-secret"])
        .output()
        .expect("the seed command must run");
    let output = cli(data.path(), cache.path())
        .env("PATH", "")
        .args(["config", "--json"])
        .output()
        .expect("the read command must run");
    let document = document(&output);
    assert_eq!(
        (
            output.status.code(),
            document["key_saved"].as_bool(),
            document["credential_source"].as_str(),
        ),
        (Some(0), Some(true), Some("saved")),
        "current Windows preferences still require ACL enforcement on every read"
    );
}

#[cfg(windows)]
#[test]
fn legacy_windows_preferences_fail_closed_without_acl_enforcement() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let directory = data.path().join("kamishibai");
    std::fs::create_dir_all(directory.as_path()).expect("preferences directory must be created");
    let path = directory.join("preferences.json");
    std::fs::write(
        path.as_path(),
        r#"{"my_language":"en","my_language_confirmed":true,"api_key":"legacy-secret"}"#,
    )
    .expect("legacy preferences must be writable");
    let output = cli(data.path(), cache.path())
        .env("PATH", "")
        .args(["config", "--json"])
        .output()
        .expect("the read command must run");
    let stored = std::fs::read_to_string(path.as_path()).expect("preferences must remain readable");
    assert_eq!(
        (
            output.status.code(),
            String::from_utf8_lossy(output.stdout.as_slice()).contains("legacy-secret"),
            String::from_utf8_lossy(output.stderr.as_slice()).contains("legacy-secret"),
            stored.contains("windows_acl_version"),
        ),
        (Some(1), false, false, false),
        "legacy Windows preferences bypassed fail-closed ACL migration"
    );
}
