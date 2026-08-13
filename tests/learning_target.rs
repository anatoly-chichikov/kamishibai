//! Offline process tests for explicit and detected understanding targets.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

fn cli(data: &Path, cache: &Path, gemini: &str) -> Command {
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    command
        .env("KAMISHIBAI_DATA", data)
        .env("KAMISHIBAI_CACHE", cache)
        .env("KAMISHIBAI_GEMINI_URL", gemini)
        .env("GEMINI_API_KEY", "offline-dummy-key");
    command
}

fn gemini(target: &str) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the Gemini stub must bind");
    let port = listener
        .local_addr()
        .expect("the Gemini stub must have an address")
        .port();
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed_calls = calls.clone();
    let observed_requests = requests.clone();
    let target = String::from(target);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            observed_calls.fetch_add(1, Ordering::SeqCst);
            observed_requests
                .lock()
                .expect("request log must lock")
                .push(request(&mut stream));
            respond(&mut stream, target.as_str());
        }
    });
    (format!("http://127.0.0.1:{port}"), calls, requests)
}

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("request timeout must configure");
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 8192];
        let size = stream.read(&mut chunk).expect("request must read");
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..size]);
        let text = String::from_utf8_lossy(bytes.as_slice());
        let Some(header_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length = text[..header_end]
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length: ")
                    .or_else(|| line.strip_prefix("Content-Length: "))
            })
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + length {
            break;
        }
    }
    String::from_utf8(bytes).expect("request must be UTF-8")
}

fn respond(stream: &mut TcpStream, target: &str) {
    let intake = json!({
        "target_lang": target,
        "items": [{
            "term": "chat",
            "senses": [{"understanding": "Сущ. «кот», домашнее животное.", "tag": null}],
            "selected": 0,
            "ok": true
        }]
    });
    let body = json!({
        "candidates": [{
            "content": {"parts": [{"text": intake.to_string()}]}
        }]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("response must write");
}

fn empty(path: &Path) -> bool {
    fs::read_dir(path)
        .expect("temporary root must be readable")
        .next()
        .is_none()
}

/// Invalid explicit languages fail before credentials, network, cache, or sessions.
#[test]
fn invalid_learning_fails_before_any_external_or_persistent_work() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, _) = gemini("FR");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .env_remove("GEMINI_API_KEY")
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--learning",
            "ZZ",
            "--id",
            "rejected",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("invalid new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["exit"].as_u64(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("EN, ZH, ES, JA, FR, DE, KO, RU, IT, PT, HI, AR, TR, PL, UK, ID, VI, TH, EL, HE, NL")),
            calls.load(Ordering::SeqCst),
            empty(data.path()),
            empty(cache.path()),
        ),
        (Some(2), Some("usage"), Some(2), true, 0, true, true),
        "invalid --learning omitted supported codes or reached credentials, Gemini, cache, or session creation"
    );
}

/// Lowercase explicit input is canonicalised and sent as a mandatory target.
#[test]
fn lowercase_explicit_learning_controls_understanding_and_session_identity() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, requests) = gemini("FR");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "ru",
            "--learning",
            "fr",
            "--id",
            "lowercase",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("explicit new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    let request = requests
        .lock()
        .expect("request log must lock")
        .first()
        .cloned()
        .expect("Gemini request must be recorded");
    assert_eq!(
        (
            output.status.success(),
            calls.load(Ordering::SeqCst),
            document["pair"]["known"].as_str(),
            document["pair"]["learning"].as_str(),
            request.contains("The required target language is FR (French)"),
            cache.path().join("understanding/RU-FR").is_dir(),
            cache
                .path()
                .join("sessions/lowercase/session.json")
                .is_file(),
        ),
        (true, 1, Some("RU"), Some("FR"), true, true, true),
        "lowercase explicit learning was not canonicalised through prompt, cache, and session"
    );
}

/// A provider target mismatch cannot leave a relabelled session or cache entry.
#[test]
fn provider_target_mismatch_leaves_no_session_or_understanding_entry() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, _) = gemini("EN");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--learning",
            "FR",
            "--id",
            "mismatch",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("mismatched new command must run");
    assert_eq!(
        (
            output.status.code(),
            calls.load(Ordering::SeqCst),
            cache.path().join("sessions/mismatch/session.json").exists(),
            cache.path().join("understanding/RU-FR").exists(),
        ),
        (Some(1), 1, false, false),
        "provider target mismatch created a relabelled session or understanding cache"
    );
}

/// Omitting the target keeps the existing model-driven language detection.
#[test]
fn omitted_learning_keeps_autodetection() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, requests) = gemini("EN");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--id",
            "detected",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("autodetected new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    let request = requests
        .lock()
        .expect("request log must lock")
        .first()
        .cloned()
        .expect("Gemini request must be recorded");
    assert_eq!(
        (
            output.status.success(),
            calls.load(Ordering::SeqCst),
            document["pair"]["learning"].as_str(),
            request.contains("Choose exactly one dominant target language"),
        ),
        (true, 1, Some("EN"), true),
        "omitting --learning no longer uses the existing autodetection contract"
    );
}
