//! Offline end-to-end coverage of the first-time agent-only workflow.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use kamishibai::generation::artifact_cache::{ILLUSTRATION_FILE, SCENE_FILE, VOICE_FILE};
use kamishibai::generation::visual_revision;
use kamishibai::session::{CardCell, CardMeta, CardMetaCache, LanguagePair};
use tempfile::TempDir;

/// Isolated directories shared by every invocation in one agent workflow.
struct Profile {
    data: TempDir,
    cache: TempDir,
    output: TempDir,
}

impl Profile {
    /// Create one clean profile with explicit data, cache, and output roots.
    fn new() -> Self {
        Self {
            data: TempDir::new().expect("data tempdir must exist"),
            cache: TempDir::new().expect("cache tempdir must exist"),
            output: TempDir::new().expect("output tempdir must exist"),
        }
    }

    /// Build one binary invocation isolated from the user's real profile.
    fn cli(&self) -> Command {
        let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
        command
            .env("KAMISHIBAI_DATA", self.data.path())
            .env("KAMISHIBAI_CACHE", self.cache.path())
            .env("KAMISHIBAI_OUTPUT", self.output.path())
            .env_remove("GEMINI_API_KEY")
            .env_remove("KAMISHIBAI_GEMINI_URL");
        command
    }
}

/// Run one JSON command and return its status plus parsed document.
fn json(command: &mut Command) -> (Option<i32>, serde_json::Value) {
    let output = command.output().expect("agent command must run");
    (
        output.status.code(),
        serde_json::from_slice(&output.stdout).expect("agent stdout must be one JSON document"),
    )
}

/// Start one local Gemini intake endpoint and return its URL plus request count.
fn intake_gemini() -> (String, Arc<AtomicUsize>, Arc<Mutex<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("intake listener must bind");
    let port = listener
        .local_addr()
        .expect("intake listener must have an address")
        .port();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let request = Arc::new(Mutex::new(String::new()));
    let captured = request.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("intake request must arrive");
        let mut scratch = [0u8; 65536];
        let count = stream.read(&mut scratch).expect("intake request must read");
        *captured.lock().expect("captured intake must lock") =
            String::from_utf8_lossy(&scratch[..count]).into_owned();
        observed.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "{\"target_lang\":\"FR\",\"items\":[{\"term\":\"chat\",\"senses\":[{\"understanding\":\"Сущ. «кот», домашнее животное.\",\"tag\":null}],\"selected\":0,\"ok\":true}]}"
                    }]
                }
            }]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("intake response must write");
    });
    (format!("http://127.0.0.1:{port}"), calls, request)
}

/// Start one local model catalog for saved-key validation.
fn credential_gemini() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("credential listener must bind");
    let port = listener
        .local_addr()
        .expect("credential listener must have an address")
        .port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("credential request must arrive");
        let mut scratch = [0u8; 65536];
        let _ = stream.read(&mut scratch);
        let body = r#"{"models":[{"name":"models/gemini-3.6-flash","supportedGenerationMethods":["generateContent"]}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("credential response must write");
    });
    format!("http://127.0.0.1:{port}")
}

/// Seed a complete card so generation and publishing remain offline.
fn seed_card(cache: &Path, understanding: &str) {
    let pair = LanguagePair::new("FR", "RU");
    let meta = CardMeta::new(
        "ʃa",
        "lə ʃa dɔʁ",
        "кот",
        8,
        "Кот спит.",
        "Кот",
        "домашнее животное",
        "повседневное существительное",
        "Le chat dort.",
    );
    CardMetaCache::new(cache)
        .store("chat", understanding, &pair, &meta)
        .expect("card meta must seed");
    let root = CardCell::new(cache, &pair, "chat", understanding).cache();
    fs::write(
        root.filepath(VOICE_FILE).expect("voice path must resolve"),
        b"RIFFxxxxWAVE",
    )
    .expect("voice must seed");
    let visual = root
        .visual(visual_revision())
        .expect("visual cache must resolve");
    fs::write(
        visual
            .filepath(SCENE_FILE)
            .expect("scene path must resolve"),
        include_bytes!("fixtures/production-scene.json"),
    )
    .expect("scene must seed");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("docs")
            .join("hero")
            .join("hero.jpg"),
        visual
            .filepath(ILLUSTRATION_FILE)
            .expect("picture path must resolve"),
    )
    .expect("picture must seed");
}

/// Poll the only session until it reaches a terminal phase.
fn terminal(profile: &Profile) -> serde_json::Value {
    let started = Instant::now();
    loop {
        let (_, status) = json(profile.cli().args(["status", "--json"]));
        if matches!(
            status["phase"].as_str(),
            Some("published" | "partial" | "failed" | "interrupted" | "cancelled")
        ) {
            return status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "agent generation did not reach a terminal phase"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn clean_env_only_agent_reaches_published_results_without_the_tui() {
    let profile = Profile::new();
    let (initial_code, initial) = json(profile.cli().args(["config", "--json"]));
    let (known_code, known) = json(profile.cli().args(["config", "--known", "RU", "--json"]));
    let (gemini, calls, _) = intake_gemini();
    let (new_code, created) = json(
        profile
            .cli()
            .env("GEMINI_API_KEY", "offline-agent-key")
            .env("KAMISHIBAI_GEMINI_URL", &gemini)
            .args(["new", "--word", "chat", "--learning", "FR", "--json"]),
    );
    let understanding = created["candidates"]["items"][0]["senses"][0]["understanding"]
        .as_str()
        .expect("created candidate must carry its understanding");
    seed_card(profile.cache.path(), understanding);
    let (generate_code, generated) = json(
        profile
            .cli()
            .env("GEMINI_API_KEY", "offline-agent-key")
            .env("KAMISHIBAI_GEMINI_URL", &gemini)
            .args(["generate", "--json"]),
    );
    let status = terminal(&profile);
    let (result_code, result) = json(profile.cli().args(["result", "--json"]));
    let deck = PathBuf::from(
        result["paths"]["deck"]
            .as_str()
            .expect("result must carry the deck path"),
    );
    let report = PathBuf::from(
        result["paths"]["pdf"]
            .as_str()
            .expect("result must carry the PDF path"),
    );
    assert_eq!(
        (
            (
                initial_code,
                initial["credential_source"].as_str(),
                initial["credential_present"].as_bool(),
                known_code,
                known["known"].as_str(),
                new_code,
                created["pair"]["known"].as_str(),
                created["pair"]["learning"].as_str(),
            ),
            (
                generate_code,
                generated["phase"].as_str(),
                status["phase"].as_str(),
                result_code,
                deck.is_file(),
                report.is_file(),
                deck.parent(),
                report.parent(),
                calls.load(Ordering::SeqCst),
            ),
        ),
        (
            (
                Some(0),
                Some("none"),
                Some(false),
                Some(0),
                Some("RU"),
                Some(0),
                Some("RU"),
                Some("FR"),
            ),
            (
                Some(0),
                Some("generating"),
                Some("published"),
                Some(0),
                true,
                true,
                Some(profile.output.path()),
                Some(profile.output.path()),
                1,
            ),
        ),
        "the clean env-only agent workflow did not reach its published files"
    );
}

#[test]
fn saved_key_supports_a_headless_session_without_an_environment_secret() {
    let profile = Profile::new();
    let credential = credential_gemini();
    let (saved_code, saved) = json(
        profile
            .cli()
            .env("KAMISHIBAI_GEMINI_URL", &credential)
            .write_stdin("saved-agent-key")
            .args(["config", "--known", "RU", "--key", "-", "--json"]),
    );
    let (gemini, calls, request) = intake_gemini();
    let (new_code, created) = json(profile.cli().env("KAMISHIBAI_GEMINI_URL", &gemini).args([
        "new",
        "--word",
        "chat",
        "--learning",
        "FR",
        "--json",
    ]));
    let request = request.lock().expect("captured intake must lock");
    assert_eq!(
        (
            saved_code,
            saved["known"].as_str(),
            saved["key_saved"].as_bool(),
            saved["credential_source"].as_str(),
            new_code,
            created["pair"]["known"].as_str(),
            created["pair"]["learning"].as_str(),
            calls.load(Ordering::SeqCst),
            request.contains("saved-agent-key"),
        ),
        (
            Some(0),
            Some("RU"),
            Some(true),
            Some("saved"),
            Some(0),
            Some("RU"),
            Some("FR"),
            1,
            true,
        ),
        "the saved-key headless setup did not carry its verified credential into intake"
    );
}
