//! Offline end-to-end tests of the session flow — no Gemini, no network beyond
//! 127.0.0.1.
//!
//! The happy path drives `new --build` (which writes each card's meta into the
//! cache), seeds the remaining artifacts so every step is a cache hit, then
//! generates and inspects. The async ladder additionally points
//! `KAMISHIBAI_GEMINI_URL` at throwaway 127.0.0.1 listeners — one that never
//! answers, keeping a real detached worker alive deterministically, and one
//! answering HTTP 500, failing one card to prove a partial publish — so the
//! detached worker, status polling, cancel, interrupted, and partial contracts
//! are all exercised with real processes, real locks, and real HTTP.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use kamishibai::generation::artifact_cache::VISUAL_DIRECTORY;
use kamishibai::generation::visual_revision;
use kamishibai::session::{CardCell, LanguagePair};
use tempfile::TempDir;

const CARDS_JSON: &str = r#"{
  "entries": [
    {
      "term": "canard",
      "meaning": "a duck",
      "pronunciation": "ka.naʁ",
      "transcription": "lə ka.naʁ",
      "importance": 5,
      "source": {
        "sentence": "The duck swam across the pond.",
        "lang": "en",
        "highlight": "duck",
        "hint": "a water bird",
        "context": "animals and nature"
      },
      "target": { "sentence": "Le canard a nagé dans l'étang.", "lang": "fr" }
    }
  ]
}"#;

const TWO_CARDS_JSON: &str = r#"{
  "entries": [
    {
      "term": "canard",
      "meaning": "a duck",
      "pronunciation": "ka.naʁ",
      "transcription": "lə ka.naʁ",
      "importance": 5,
      "source": {
        "sentence": "The duck swam across the pond.",
        "lang": "en",
        "highlight": "duck",
        "hint": "a water bird",
        "context": "animals and nature"
      },
      "target": { "sentence": "Le canard a nagé dans l'étang.", "lang": "fr" }
    },
    {
      "term": "lanterne",
      "meaning": "a lantern",
      "pronunciation": "lɑ̃.tɛʁn",
      "transcription": "la lɑ̃.tɛʁn",
      "importance": 4,
      "source": {
        "sentence": "The lantern lit the cellar.",
        "lang": "en",
        "highlight": "lantern",
        "hint": "a portable light",
        "context": "household objects"
      },
      "target": { "sentence": "La lanterne éclairait la cave.", "lang": "fr" }
    }
  ]
}"#;

fn cli(cache: &Path) -> Command {
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    command
        .env("KAMISHIBAI_CACHE", cache)
        .env("GEMINI_API_KEY", "offline-dummy-key");
    command
}

fn cli_at(cache: &Path, gemini: &str) -> Command {
    let mut command = cli(cache);
    command.env("KAMISHIBAI_GEMINI_URL", gemini);
    command
}

/// Run `new --build` with a fixed id, leaving the session understood.
fn understood_session(cache: &Path, out: &Path, id: &str, json: &str) {
    let cards = cache.join("cards.json");
    fs::write(&cards, json).expect("the cards JSON must write");
    cli(cache)
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            id,
            "--out",
            out.to_str().expect("out path is utf8"),
        ])
        .assert()
        .success();
}

fn card_dirs(cache: &Path) -> Vec<PathBuf> {
    let mut cells = Vec::new();
    let pairs = cache.join("cards");
    for pair in fs::read_dir(&pairs)
        .expect("cards directory must exist")
        .flatten()
    {
        for cell in fs::read_dir(pair.path())
            .expect("pair directory must exist")
            .flatten()
        {
            if cell.path().is_dir() {
                cells.push(cell.path());
            }
        }
    }
    cells
}

fn first_card_dir(cache: &Path) -> PathBuf {
    card_dirs(cache)
        .into_iter()
        .next()
        .expect("`new --build` created no card folder under the cache")
}

/// Find the cached cell of one card by the term inside its stored meta.
fn cell_of(cache: &Path, term: &str) -> PathBuf {
    for cell in card_dirs(cache) {
        if let Ok(text) = fs::read_to_string(cell.join("meta.json"))
            && text.contains(term)
        {
            return cell;
        }
    }
    panic!("no cached cell mentions '{term}'");
}

fn fixture_jpeg() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/hero/hero.jpg");
    assert!(path.is_file(), "missing JPEG fixture for the offline test");
    path
}

/// Seed everything but the meta (which `new --build` wrote) into one cell.
fn seed_artifacts(cell: &Path) {
    fs::write(cell.join("audio.wav"), b"RIFFxxxxWAVE").expect("seed voice");
    seed_visual_artifacts(cell);
}

/// Seed the current visual-policy artifacts into one card cell.
fn seed_visual_artifacts(cell: &Path) {
    let visual = cell.join(VISUAL_DIRECTORY).join(visual_revision());
    fs::create_dir_all(&visual).expect("seed visual directory");
    fs::write(
        visual.join("scene.json"),
        include_bytes!("fixtures/production-scene.json"),
    )
    .expect("seed scene");
    fs::copy(fixture_jpeg(), visual.join("picture.jpg")).expect("seed picture");
}

/// Poll `status --json` until its `phase` field satisfies the predicate,
/// panicking with the last seen phase when the deadline expires (a harness
/// failure, not a verdict).
fn poll_quiet_status(
    cache: &Path,
    id: &str,
    deadline: Duration,
    until: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    loop {
        let output = cli(cache)
            .args(["status", id, "--json"])
            .output()
            .expect("status must run");
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null);
        let phase = value["phase"].as_str().unwrap_or("").to_string();
        if until(phase.as_str()) {
            return phase;
        }
        assert!(
            started.elapsed() < deadline,
            "status polling deadline expired; the phase never left '{phase}'"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Poll the full `status` text until it satisfies the predicate, panicking when
/// the deadline expires.
fn poll_full_status(
    cache: &Path,
    id: &str,
    deadline: Duration,
    until: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    loop {
        let output = cli(cache)
            .args(["status", id])
            .output()
            .expect("status must run");
        let text = String::from_utf8(output.stdout).expect("status output must be UTF-8");
        if until(text.as_str()) {
            return text;
        }
        assert!(
            started.elapsed() < deadline,
            "status polling deadline expired; the last status was:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Start a 127.0.0.1 listener that accepts connections and never answers, so a
/// worker generating against it stays alive for the client timeout window.
fn stalled_gemini() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the stalled listener must bind");
    let port = listener
        .local_addr()
        .expect("the stalled listener has an address")
        .port();
    std::thread::spawn(move || {
        let mut parked = Vec::new();
        for stream in listener.incoming().flatten() {
            parked.push(stream);
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Start one stalled endpoint that captures the worker's authenticated request.
fn observed_stalled_gemini() -> (String, Arc<Mutex<String>>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the observed listener must bind");
    let port = listener
        .local_addr()
        .expect("the observed listener has an address")
        .port();
    let request = Arc::new(Mutex::new(String::new()));
    let release = Arc::new(AtomicBool::new(false));
    let captured = request.clone();
    let permitted = release.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the worker request must arrive");
        let mut scratch = [0u8; 65536];
        let count = stream.read(&mut scratch).expect("worker request must read");
        *captured.lock().expect("captured worker request must lock") =
            String::from_utf8_lossy(&scratch[..count]).into_owned();
        while !permitted.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    (format!("http://127.0.0.1:{port}"), request, release)
}

/// Start a 127.0.0.1 listener that answers every request with HTTP 500, so any
/// generation against it fails fast through its retries.
fn failing_gemini() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the failing listener must bind");
    let port = listener
        .local_addr()
        .expect("the failing listener has an address")
        .port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut scratch = [0u8; 65536];
            let _ = stream.read(&mut scratch);
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
            );
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Start a metered stub that returns one valid bulk-correction response.
fn metered_bulk_correction_gemini() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the correction listener must bind");
    let port = listener
        .local_addr()
        .expect("the correction listener has an address")
        .port();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut scratch = [0u8; 65536];
            let _ = stream.read(&mut scratch);
            observed.fetch_add(1, Ordering::SeqCst);
            let body = bulk_correction_body();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}"), calls)
}

/// Start a correction stub that waits until the test commits the session plan.
fn blocked_bulk_correction_gemini() -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the correction listener must bind");
    let port = listener
        .local_addr()
        .expect("the correction listener has an address")
        .port();
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let observed = calls.clone();
    let permitted = release.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut scratch = [0u8; 65536];
            let _ = stream.read(&mut scratch);
            observed.fetch_add(1, Ordering::SeqCst);
            while !permitted.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
            let body = bulk_correction_body();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}"), calls, release)
}

fn bulk_correction_body() -> String {
    serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"senses\":[{\"understanding\":\"a newspaper hoax\",\"tag\":\"dated\"}],\"message\":null}"
                }]
            }
        }]
    })
    .to_string()
}

/// Attach one nonzero authoritative cost to a committed session and return exact snapshots.
fn costed_session_snapshots(cache: &Path, id: &str) -> (PathBuf, Vec<u8>, PathBuf, Vec<u8>) {
    let directory = cache.join("sessions").join(id);
    let session_path = directory.join("session.json");
    let mut session: serde_json::Value = serde_json::from_slice(
        fs::read(&session_path)
            .expect("the committed session must exist")
            .as_slice(),
    )
    .expect("the committed session must decode");
    session["drafts"][0]["costs"] = serde_json::json!({"meta": {"nanos": 73_000}});
    let session_bytes = serde_json::to_vec_pretty(&session).expect("the session must encode");
    fs::write(&session_path, &session_bytes).expect("the costed session must write");
    let journal_path = fs::read_dir(&directory)
        .expect("the session directory must list")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("costs-") && name.ends_with(".json"))
        })
        .expect("the committed session must have a cost journal");
    let mut journal: serde_json::Value = serde_json::from_slice(
        fs::read(&journal_path)
            .expect("the cost journal must exist")
            .as_slice(),
    )
    .expect("the cost journal must decode");
    journal["slots"][0] = serde_json::json!({"meta": {"nanos": 73_000}});
    let journal_bytes = serde_json::to_vec_pretty(&journal).expect("the journal must encode");
    fs::write(&journal_path, &journal_bytes).expect("the cost journal must write");
    (session_path, session_bytes, journal_path, journal_bytes)
}

#[derive(Debug, Default)]
struct GenerationCalls {
    meta: AtomicUsize,
    sound: AtomicUsize,
}

/// Start a metered stub whose delayed responses expose duplicate generators.
fn metered_generation_gemini() -> (String, Arc<GenerationCalls>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the audio listener must bind");
    let port = listener
        .local_addr()
        .expect("the audio listener has an address")
        .port();
    let calls = Arc::new(GenerationCalls::default());
    let observed = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let counted = observed.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut scratch = [0u8; 65536];
                let size = stream.read(&mut scratch).unwrap_or(0);
                let request = String::from_utf8_lossy(&scratch[..size]);
                let sound = request.contains("gemini-3.1-flash-tts-preview");
                if sound {
                    counted.sound.fetch_add(1, Ordering::SeqCst);
                } else {
                    counted.meta.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(500));
                let content = if sound {
                    serde_json::json!({"inlineData": {"data": "AAA="}})
                } else {
                    serde_json::json!({"text": serde_json::json!({
                        "pronunciation": "ka.naʁ",
                        "transcription": "lə ka.naʁ",
                        "meaning": "a duck",
                        "importance": 5,
                        "source_sentence": "The duck swam across the pond.",
                        "source_highlight": "duck",
                        "source_hint": "a water bird",
                        "source_context": "animals and nature",
                        "target_sentence": "Le canard a nagé dans l'étang.",
                        "labels": {
                            "register": "neutral",
                            "level": "b1",
                            "type": "statement",
                            "approx": []
                        }
                    }).to_string()})
                };
                let body = serde_json::json!({
                    "candidates": [{"content": {"parts": [content]}}],
                    "usageMetadata": {
                        "promptTokenCount": 10,
                        "candidatesTokenCount": 5,
                        "totalTokenCount": 15
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), calls)
}

/// Reap one child within a deadline and report only a successful exit.
fn finish_success(child: &mut std::process::Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            return status.success();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Wait for the detached worker to claim one session and record its pid.
#[cfg(unix)]
fn worker_pid(cache: &Path, id: &str) -> i64 {
    let path = cache.join("sessions").join(id).join("session.json");
    let started = Instant::now();
    loop {
        let text = fs::read_to_string(&path).expect("the session file must exist");
        let record: serde_json::Value =
            serde_json::from_str(text.as_str()).expect("the session file must be valid JSON");
        if let Some(pid) = record["worker"]["pid"].as_i64() {
            return pid;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the session must record a worker pid"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Seed a session whose detached worker provably stays alive: every artifact
/// except the voice is cached, so the worker blocks on one TTS request against
/// the stalled listener. Returns once `status` reports the worker alive.
#[cfg(unix)]
fn live_worker_session(cache: &Path, out: &Path, id: &str, gemini: &str) {
    understood_session(cache, out, id, CARDS_JSON);
    let cell = first_card_dir(cache);
    seed_visual_artifacts(&cell);
    cli_at(cache, gemini)
        .args(["generate", id])
        .assert()
        .success();
    poll_full_status(cache, id, Duration::from_secs(30), |text| {
        text.contains("building ")
    });
}

#[test]
fn a_fully_cached_build_session_runs_to_published_offline() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "offline", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "offline"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let phase = poll_quiet_status(cache.path(), "offline", Duration::from_secs(5), |_| true);
    assert_eq!(
        phase, "published",
        "a fully-cached build session must run to published with no Gemini calls"
    );
}

#[test]
fn curation_cannot_replace_a_committed_costed_plan() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "costed-plan", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "costed-plan"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let (session_path, session_bytes, journal_path, journal_bytes) =
        costed_session_snapshots(cache.path(), "costed-plan");
    let (gemini, calls) = metered_bulk_correction_gemini();
    let commands = [
        vec!["select", "costed-plan", "--card", "canard", "--sense", "1"],
        vec!["exclude", "costed-plan", "--card", "canard"],
        vec![
            "correct",
            "costed-plan",
            "--card",
            "canard",
            "--note",
            "add its dated newspaper sense",
        ],
    ];
    let mut refused = Vec::new();
    let mut unchanged = Vec::new();
    for command in commands {
        fs::write(&session_path, &session_bytes).expect("the session snapshot must restore");
        fs::write(&journal_path, &journal_bytes).expect("the journal snapshot must restore");
        let output = cli_at(cache.path(), gemini.as_str())
            .args(command)
            .output()
            .expect("the curation command must run");
        refused.push(output.status.code() == Some(2));
        unchanged.push(
            fs::read(&session_path).ok().as_ref() == Some(&session_bytes)
                && fs::read(&journal_path).ok().as_ref() == Some(&journal_bytes),
        );
    }
    assert_eq!(
        (refused, unchanged, calls.load(Ordering::SeqCst)),
        (vec![true, true, true], vec![true, true, true], 0),
        "post-commit curation changed the stable plan or costs, or called Gemini"
    );
}

#[test]
fn a_bulk_correction_cannot_clear_a_plan_committed_while_gemini_answers() {
    use std::process::Stdio;
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "correction-race", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    let (gemini, calls, release) = blocked_bulk_correction_gemini();
    let mut correction = std::process::Command::new(env!("CARGO_BIN_EXE_kamishibai"))
        .args([
            "correct",
            "correction-race",
            "--card",
            "canard",
            "--note",
            "add its dated newspaper sense",
        ])
        .env("KAMISHIBAI_CACHE", cache.path())
        .env("KAMISHIBAI_GEMINI_URL", gemini.as_str())
        .env("GEMINI_API_KEY", "offline-dummy-key")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the correction command must spawn");
    let started = Instant::now();
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the correction provider call must arrive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cli(cache.path())
        .args(["generate", "--wait", "correction-race"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let session_path = cache
        .path()
        .join("sessions")
        .join("correction-race")
        .join("session.json");
    let committed = fs::read(&session_path).expect("the committed record must read");
    release.store(true, Ordering::SeqCst);
    let status = correction.wait().expect("the correction command must exit");
    assert_eq!(
        (
            status.code(),
            calls.load(Ordering::SeqCst),
            fs::read(session_path).ok(),
        ),
        (Some(2), 1, Some(committed)),
        "a correction response won its race against a newly committed plan"
    );
}

#[test]
fn concurrent_sessions_share_one_meta_and_sound_request_with_exact_costs() {
    let cache = TempDir::new().expect("cache tempdir");
    let first_out = TempDir::new().expect("first output tempdir");
    let second_out = TempDir::new().expect("second output tempdir");
    understood_session(cache.path(), first_out.path(), "audio-first", CARDS_JSON);
    understood_session(cache.path(), second_out.path(), "audio-second", CARDS_JSON);
    let cell = first_card_dir(cache.path());
    fs::remove_file(cell.join("meta.json")).expect("the shared meta must be absent");
    seed_visual_artifacts(cell.as_path());
    let (gemini, calls) = metered_generation_gemini();
    let spawn = |id: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_kamishibai"))
            .args(["generate", "--wait", id])
            .env("KAMISHIBAI_CACHE", cache.path())
            .env("KAMISHIBAI_GEMINI_URL", gemini.as_str())
            .env("GEMINI_API_KEY", "offline-dummy-key")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the concurrent session must spawn")
    };
    let mut first = spawn("audio-first");
    let mut second = spawn("audio-second");
    let first_ok = finish_success(&mut first, Duration::from_secs(120));
    let second_ok = finish_success(&mut second, Duration::from_secs(120));
    let meta: serde_json::Value = serde_json::from_slice(
        fs::read(cell.join("meta.cost.json"))
            .expect("the meta cost must persist")
            .as_slice(),
    )
    .expect("the meta cost must decode");
    let sound: serde_json::Value = serde_json::from_slice(
        fs::read(cell.join("audio.cost.json"))
            .expect("the sound cost must persist")
            .as_slice(),
    )
    .expect("the sound cost must decode");
    assert_eq!(
        (
            first_ok,
            second_ok,
            calls.meta.load(Ordering::SeqCst),
            calls.sound.load(Ordering::SeqCst),
            meta["requests"].as_u64(),
            meta["cost"]["nanos"].as_u64(),
            sound["requests"].as_u64(),
            sound["cost"]["nanos"].as_u64(),
        ),
        (
            true,
            true,
            1,
            1,
            Some(1),
            Some(52_500),
            Some(1),
            Some(110_000),
        ),
        "two sessions sharing one card duplicated Gemini spend or corrupted its exact costs"
    );
}

#[test]
fn a_detached_generate_reaches_published_while_status_polls_it() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "detached", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "detached"])
        .assert()
        .success();
    let terminal = ["published", "partial", "failed", "interrupted", "cancelled"];
    let phase = poll_quiet_status(
        cache.path(),
        "detached",
        Duration::from_secs(120),
        |phase| terminal.contains(&phase),
    );
    assert_eq!(
        phase, "published",
        "a detached generate must reach published while status polls from outside"
    );
}

#[cfg(unix)]
#[test]
fn status_reports_a_live_worker_while_a_detached_run_is_in_flight() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let gemini = stalled_gemini();
    live_worker_session(cache.path(), out.path(), "live", gemini.as_str());
    let status = poll_full_status(cache.path(), "live", Duration::from_secs(5), |text| {
        text.contains("building ")
    });
    cli(cache.path())
        .args(["cancel", "live"])
        .assert()
        .success();
    assert!(
        status.contains("· generating") && status.contains("building 1 card (pid"),
        "status during a detached run must report the generating phase and a live worker"
    );
}

#[cfg(unix)]
#[test]
fn detached_worker_inherits_the_environment_api_key() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let (gemini, request, release) = observed_stalled_gemini();
    live_worker_session(cache.path(), out.path(), "inherited", gemini.as_str());
    let started = Instant::now();
    let observed = loop {
        let observed = request
            .lock()
            .expect("captured worker request must lock")
            .clone();
        if !observed.is_empty() {
            break observed;
        }
        if started.elapsed() >= Duration::from_secs(10) {
            break String::new();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let cancelled = cli(cache.path())
        .args(["cancel", "inherited"])
        .output()
        .expect("worker cancellation must run")
        .status
        .success();
    release.store(true, Ordering::SeqCst);
    assert_eq!(
        (
            cancelled,
            observed
                .to_ascii_lowercase()
                .contains("x-goog-api-key: offline-dummy-key"),
        ),
        (true, true),
        "the detached worker did not inherit and authenticate with its parent's environment key"
    );
}

#[cfg(unix)]
#[test]
fn cancelling_a_live_worker_marks_the_session_cancelled() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let gemini = stalled_gemini();
    live_worker_session(cache.path(), out.path(), "stop", gemini.as_str());
    cli(cache.path())
        .args(["cancel", "stop"])
        .assert()
        .success();
    let phase = poll_quiet_status(cache.path(), "stop", Duration::from_secs(10), |phase| {
        phase != "generating"
    });
    assert_eq!(
        phase, "cancelled",
        "cancelling a live worker must settle the session cancelled"
    );
}

#[cfg(unix)]
#[test]
fn a_killed_worker_reads_interrupted_not_generating() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let gemini = stalled_gemini();
    live_worker_session(cache.path(), out.path(), "killed", gemini.as_str());
    let pid = worker_pid(cache.path(), "killed");
    std::process::Command::new("kill")
        .args(["-9", pid.to_string().as_str()])
        .status()
        .expect("kill must run");
    let phase = poll_quiet_status(cache.path(), "killed", Duration::from_secs(10), |phase| {
        phase != "generating"
    });
    assert_eq!(
        phase, "interrupted",
        "a SIGKILLed worker must read interrupted once the OS releases its lock"
    );
}

#[cfg(unix)]
#[test]
fn cancelling_an_interrupted_session_settles_it_cancelled() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let gemini = stalled_gemini();
    live_worker_session(cache.path(), out.path(), "settle", gemini.as_str());
    let pid = worker_pid(cache.path(), "settle");
    std::process::Command::new("kill")
        .args(["-9", pid.to_string().as_str()])
        .status()
        .expect("kill must run");
    poll_quiet_status(cache.path(), "settle", Duration::from_secs(10), |phase| {
        phase == "interrupted"
    });
    cli(cache.path())
        .args(["cancel", "settle"])
        .assert()
        .success();
    let phase = poll_quiet_status(cache.path(), "settle", Duration::from_secs(5), |_| true);
    assert_eq!(
        phase, "cancelled",
        "cancelling an interrupted session must settle it cancelled without signalling the stale pid"
    );
}

#[test]
fn a_run_with_one_unbuildable_card_publishes_partial() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "half", TWO_CARDS_JSON);
    seed_artifacts(&cell_of(cache.path(), "canard"));
    fs::remove_file(cell_of(cache.path(), "lanterne").join("meta.json"))
        .expect("the lanterne meta must be dropped");
    let gemini = failing_gemini();
    cli_at(cache.path(), gemini.as_str())
        .args(["generate", "--wait", "half"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let phase = poll_quiet_status(cache.path(), "half", Duration::from_secs(5), |_| true);
    assert_eq!(
        phase, "partial",
        "a run where one card cannot build must still publish the rest as partial"
    );
}

#[test]
fn concurrent_edits_to_different_cards_both_survive() {
    use std::process::{Command as Process, Stdio};
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "race", TWO_CARDS_JSON);
    let binary = env!("CARGO_BIN_EXE_kamishibai");
    let racers: Vec<_> = [
        vec!["exclude", "race", "--card", "canard"],
        vec!["select", "race", "--card", "lanterne", "--sense", "1"],
    ]
    .into_iter()
    .map(|args| {
        Process::new(binary)
            .args(args)
            .env("KAMISHIBAI_CACHE", cache.path())
            .env("GEMINI_API_KEY", "offline-dummy-key")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a concurrent edit must spawn")
    })
    .collect();
    let all_succeeded = racers
        .into_iter()
        .all(|mut racer| racer.wait().expect("a racer must be reapable").success());
    let value = json_stdout(cli(cache.path()).args(["status", "race", "--json"]));
    let items = &value["candidates"]["items"];
    let canard_excluded = items[0]["included"].as_bool() == Some(false);
    let lanterne_selected = items[1]["senses"][0]["selected"].as_bool() == Some(true);
    assert!(
        all_succeeded && canard_excluded && lanterne_selected,
        "concurrent edits to different cards must both succeed and both land in the record"
    );
}

#[test]
fn cancelling_a_published_session_keeps_it_published() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "keep", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "keep"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    cli(cache.path())
        .args(["cancel", "keep"])
        .assert()
        .success();
    let phase = poll_quiet_status(cache.path(), "keep", Duration::from_secs(5), |_| true);
    assert_eq!(
        phase, "published",
        "cancelling an already-published session must leave it published, not cancelled"
    );
}

#[test]
fn result_before_publication_exits_with_the_not_ready_code() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "early", CARDS_JSON);
    cli(cache.path()).args(["result", "early"]).assert().code(4);
}

#[test]
fn result_deck_prints_one_existing_apkg_path() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "deck", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "deck"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let value = json_stdout(cli(cache.path()).args(["result", "deck", "--json"]));
    let path = value["paths"]["deck"]
        .as_str()
        .expect("result --json must carry the deck path")
        .to_string();
    assert!(
        path.ends_with(".apkg") && Path::new(path.as_str()).is_file(),
        "result --json must carry exactly one existing .apkg deck path"
    );
}

#[test]
fn a_corrupt_session_file_exits_with_the_operational_code() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "broken", CARDS_JSON);
    fs::write(
        cache
            .path()
            .join("sessions")
            .join("broken")
            .join("session.json"),
        "not json",
    )
    .expect("the session file must be overwritten");
    cli(cache.path())
        .args(["status", "broken"])
        .assert()
        .code(1);
}

#[test]
fn regenerate_before_generate_is_refused() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "fresh", CARDS_JSON);
    cli(cache.path())
        .args(["regenerate", "fresh", "--failed"])
        .assert()
        .code(2);
    let value = json_stdout(cli(cache.path()).args(["status", "fresh", "--json"]));
    assert_eq!(
        value["phase"].as_str(),
        Some("understood"),
        "regenerate before any generate must be refused, leaving the session understood"
    );
}

#[test]
fn generate_refuses_a_persisted_staged_rewrite_before_worker_or_provider_work() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "staged", CARDS_JSON);
    let session_path = cache.path().join("sessions/staged/session.json");
    let mut session: serde_json::Value = serde_json::from_slice(
        fs::read(&session_path)
            .expect("the staged session must read")
            .as_slice(),
    )
    .expect("the staged session must decode");
    session["drafts"] = serde_json::json!([{
        "term": "canard",
        "understanding": "a duck",
        "rewrite": {
            "previous": null,
            "selection": {
                "values": {
                    "register": "formal",
                    "level": null,
                    "kind": null
                },
                "pinned": ["register"],
                "approx": []
            },
            "note": "make it formal",
            "started": false
        }
    }]);
    let staged = serde_json::to_vec_pretty(&session).expect("the staged session must encode");
    fs::write(&session_path, &staged).expect("the staged session must persist");
    let (gemini, calls) = metered_bulk_correction_gemini();
    let output = cli_at(cache.path(), gemini.as_str())
        .args(["generate", "staged"])
        .output()
        .expect("the staged generate command must exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let preserved = fs::read(&session_path).expect("the refused session must remain readable");
    let record: serde_json::Value =
        serde_json::from_slice(&preserved).expect("the refused session must remain valid");
    assert_eq!(
        (
            output.status.code(),
            stderr.contains("staged card changes waiting for Ctrl+G"),
            stderr.contains("kamishibai open staged"),
            stderr.contains("press Ctrl+G"),
            preserved == staged,
            calls.load(Ordering::SeqCst),
            record["phase"].as_str(),
            record["worker"].is_null(),
            record["drafts"][0]["rewrite"]["started"].as_bool(),
        ),
        (
            Some(2),
            true,
            true,
            true,
            true,
            0,
            Some("understood"),
            true,
            Some(false),
        ),
        "generate crossed the staged-rewrite preflight or mutated its pending session"
    );
}

#[test]
fn excluding_every_card_leaves_nothing_to_generate() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "curate", CARDS_JSON);
    cli(cache.path())
        .args(["exclude", "curate", "--card", "canard"])
        .assert()
        .success();
    cli(cache.path())
        .args(["generate", "--wait", "curate"])
        .assert()
        .code(2);
    let value = json_stdout(cli(cache.path()).args(["status", "curate", "--json"]));
    assert_eq!(
        value["phase"].as_str(),
        Some("understood"),
        "excluding the only card must keep the session understood with nothing to generate"
    );
}

#[test]
fn invoking_new_with_no_words_exits_with_the_usage_code() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path()).args(["new"]).assert().code(2);
}

#[test]
fn selecting_a_sense_out_of_range_exits_with_the_usage_code() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "range", CARDS_JSON);
    cli(cache.path())
        .args(["select", "range", "--card", "canard", "--sense", "2"])
        .assert()
        .code(2);
}

#[test]
fn acting_on_a_missing_session_exits_with_the_not_found_code() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path()).args(["status", "ghost"]).assert().code(3);
}

#[test]
fn mixing_build_with_word_is_refused_at_parse() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path())
        .args(["new", "--build", "cards.json", "--word", "bank"])
        .assert()
        .code(2);
}

#[test]
fn requesting_two_result_selectors_is_refused_at_parse() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path())
        .args(["result", "x", "--deck", "--pdf"])
        .assert()
        .code(2);
}

#[test]
fn a_regenerate_note_without_a_card_is_refused_at_parse() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path())
        .args(["regenerate", "x", "--failed", "--note", "rewrite it"])
        .assert()
        .code(2);
}

#[test]
fn regenerate_json_and_quiet_conflict_is_refused() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path())
        .args(["regenerate", "x", "--failed", "--json", "-q"])
        .assert()
        .code(2);
}

#[test]
fn regenerate_rebuilds_instead_of_resting_at_understood() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "rebuild", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "rebuild"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let gemini = failing_gemini();
    cli_at(cache.path(), &gemini)
        .args(["regenerate", "rebuild", "--card", "canard", "--wait"])
        .timeout(Duration::from_secs(120))
        .output()
        .expect("regenerate must run to completion");
    let phase = poll_quiet_status(cache.path(), "rebuild", Duration::from_secs(5), |p| {
        p != "understood"
    });
    assert_ne!(
        phase, "understood",
        "regenerate must run a worker to rebuild, never drop the session back to understood"
    );
}

/// Run one command expecting `--json` mode, parsing stdout as the single document.
fn json_stdout(command: &mut Command) -> serde_json::Value {
    let output = command.output().expect("the command must run");
    serde_json::from_slice(&output.stdout).expect("stdout must carry one JSON document")
}

/// Run `new --build --json` with a fixed id, returning the creation document.
fn understood_session_json(cache: &Path, out: &Path, id: &str, json: &str) -> serde_json::Value {
    let cards = cache.join("cards.json");
    fs::write(&cards, json).expect("the cards JSON must write");
    json_stdout(cli(cache).args([
        "new",
        "--build",
        cards.to_str().expect("cards path is utf8"),
        "--id",
        id,
        "--out",
        out.to_str().expect("out path is utf8"),
        "--json",
    ]))
}

#[test]
fn a_build_session_returns_its_document_in_json_mode() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let value = understood_session_json(cache.path(), out.path(), "jdoc", CARDS_JSON);
    assert_eq!(
        (
            value["ok"].as_bool(),
            value["session"].as_str(),
            value["phase"].as_str(),
            value["candidates"]["items"][0]["term"].as_str(),
        ),
        (Some(true), Some("jdoc"), Some("understood"), Some("canard")),
        "new --json must print the created session's document instead of the preview"
    );
}

#[test]
fn a_build_session_imports_without_any_api_key() {
    let cache = TempDir::new().expect("cache tempdir");
    let data = TempDir::new().expect("data tempdir");
    let out = TempDir::new().expect("output tempdir");
    let cards = cache.path().join("offline-cards.json");
    fs::write(&cards, CARDS_JSON).expect("cards JSON must write");
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    let output = command
        .env("KAMISHIBAI_CACHE", cache.path())
        .env("KAMISHIBAI_DATA", data.path())
        .env_remove("GEMINI_API_KEY")
        .env_remove("KAMISHIBAI_GEMINI_URL")
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "offline-no-key",
            "--out",
            out.path().to_str().expect("output path is utf8"),
            "--json",
        ])
        .output()
        .expect("offline build import must run");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("build stdout must be JSON");
    assert_eq!(
        (
            output.status.code(),
            value["session"].as_str(),
            value["phase"].as_str(),
        ),
        (Some(0), Some("offline-no-key"), Some("understood")),
        "new --build unexpectedly required a Gemini key"
    );
}

#[test]
fn a_relative_output_is_absolute_in_the_creation_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let work = TempDir::new().expect("work tempdir");
    let cards = work.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("cards JSON must write");
    let value = json_stdout(cli(cache.path()).current_dir(work.path()).args([
        "new",
        "--build",
        cards.to_str().expect("cards path is utf8"),
        "--id",
        "absolute-json-out",
        "--out",
        "exports",
        "--json",
    ]));
    let output = Path::new(
        value["out"]
            .as_str()
            .expect("creation output must be a string"),
    );
    assert!(
        output.is_absolute() && output.ends_with("exports"),
        "new --json returned a non-absolute output path: {}",
        output.display()
    );
}

#[test]
fn a_relative_environment_output_stays_absolute_in_status_and_result_json() {
    let cache = TempDir::new().expect("cache tempdir");
    let work = TempDir::new().expect("work tempdir");
    let cards = work.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("cards JSON must write");
    cli(cache.path())
        .current_dir(work.path())
        .env("KAMISHIBAI_OUTPUT", "exports")
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "absolute-json-paths",
        ])
        .assert()
        .success();
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .current_dir(work.path())
        .args(["generate", "--wait", "absolute-json-paths"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let status = json_stdout(cli(cache.path()).current_dir(work.path()).args([
        "status",
        "absolute-json-paths",
        "--json",
    ]));
    let result = json_stdout(cli(cache.path()).current_dir(work.path()).args([
        "result",
        "absolute-json-paths",
        "--json",
    ]));
    let paths = [
        status["out"].as_str().expect("status out must be a string"),
        result["paths"]["deck"]
            .as_str()
            .expect("result deck must be a string"),
        result["paths"]["pdf"]
            .as_str()
            .expect("result pdf must be a string"),
        result["paths"]["dir"]
            .as_str()
            .expect("result dir must be a string"),
    ];
    assert!(
        paths.iter().all(|path| Path::new(path).is_absolute())
            && Path::new(paths[3]).ends_with("exports")
            && paths[0] == paths[3]
            && Path::new(paths[1]).parent() == Some(Path::new(paths[3]))
            && Path::new(paths[2]).parent() == Some(Path::new(paths[3])),
        "status/result JSON returned a non-absolute output path: {paths:?}"
    );
}

#[test]
fn excluding_a_card_in_json_mode_returns_the_post_mutation_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jmut", CARDS_JSON);
    let value =
        json_stdout(cli(cache.path()).args(["exclude", "jmut", "--card", "canard", "--json"]));
    assert_eq!(
        (
            value["candidates"]["items"][0]["included"].as_bool(),
            value["candidates"]["items"][0]["senses"][0]["selected"].as_bool(),
        ),
        (Some(false), Some(false)),
        "exclude --json must return the document with the card excluded and its sense deselected"
    );
}

#[test]
fn a_detached_generate_in_json_mode_reports_the_generating_phase() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jbg", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    let value = json_stdout(cli(cache.path()).args(["generate", "jbg", "--json"]));
    let terminal = ["published", "partial", "failed", "interrupted", "cancelled"];
    poll_quiet_status(cache.path(), "jbg", Duration::from_secs(120), |phase| {
        terminal.contains(&phase)
    });
    assert_eq!(
        value["phase"].as_str(),
        Some("generating"),
        "a detached generate --json must print the session document already in the generating phase"
    );
}

#[test]
fn a_waited_generate_in_json_mode_prints_one_terminal_document_and_event_lines() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jwait", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    let output = cli(cache.path())
        .args(["generate", "--wait", "jwait", "--json"])
        .timeout(Duration::from_secs(120))
        .output()
        .expect("generate --wait --json must run");
    let stdout = String::from_utf8(output.stdout).expect("stdout must be UTF-8");
    let documents: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line must be JSON"))
        .collect();
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("stderr line is not JSON: {line:?}: {error}"))
        })
        .collect();
    let unnamed_events = events
        .iter()
        .filter(|event| event["event"].as_str().is_none())
        .count();
    assert_eq!(
        (
            documents.len(),
            documents[0]["phase"].as_str(),
            documents[0]["cards"]["items"]
                .as_array()
                .map(|items| items.len()),
            unnamed_events,
        ),
        (1, Some("published"), Some(1), 0),
        "generate --wait --json must print exactly one terminal document on stdout and only tagged NDJSON events on stderr"
    );
}

#[test]
fn result_items_in_json_mode_round_trip_into_a_new_build_session() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jround", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "jround"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let value = json_stdout(cli(cache.path()).args(["result", "jround", "--json"]));
    let reimport = serde_json::json!({ "entries": value["items"] });
    let cards = cache.path().join("reimport.json");
    fs::write(&cards, reimport.to_string()).expect("the reimport JSON must write");
    let reimported = json_stdout(cli(cache.path()).args([
        "new",
        "--build",
        cards.to_str().expect("reimport path is utf8"),
        "--id",
        "jround2",
        "--json",
    ]));
    assert_eq!(
        (
            reimported["ok"].as_bool(),
            reimported["session"].as_str(),
            reimported["candidates"]["items"][0]["term"].as_str(),
        ),
        (Some(true), Some("jround2"), Some("canard")),
        "result items must round-trip back into a fresh build session importing the same card"
    );
}

#[test]
fn ls_in_json_mode_lists_every_session_as_one_item() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jls", CARDS_JSON);
    let value = json_stdout(cli(cache.path()).args(["ls", "--json"]));
    assert_eq!(
        (
            value["sessions"][0]["id"].as_str(),
            value["sessions"][0]["phase"].as_str(),
            value["sessions"][0]["selected"].as_u64(),
        ),
        (Some("jls"), Some("understood"), Some(1)),
        "ls --json must list each session with its live phase and curation count"
    );
}

#[test]
fn cancelling_in_json_mode_returns_the_settled_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jcancel", CARDS_JSON);
    let value = json_stdout(cli(cache.path()).args(["cancel", "jcancel", "--json"]));
    assert_eq!(
        value["phase"].as_str(),
        Some("cancelled"),
        "cancel --json must return the document with the settled cancelled phase"
    );
}

#[test]
fn removing_in_json_mode_acknowledges_the_removed_id() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jrm", CARDS_JSON);
    let value = json_stdout(cli(cache.path()).args(["rm", "jrm", "--json"]));
    assert_eq!(
        (value["ok"].as_bool(), value["removed"].as_str()),
        (Some(true), Some("jrm")),
        "rm --json must acknowledge the removed session id"
    );
}

#[test]
fn removing_with_cache_deletes_every_visual_revision_for_the_card() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jpurge", CARDS_JSON);
    let cell = first_card_dir(cache.path());
    seed_artifacts(cell.as_path());
    let revision = "0000000000000000000000000000000000000000000000000000000000000000";
    let sibling = cell.join(VISUAL_DIRECTORY).join(revision);
    fs::create_dir_all(&sibling).expect("sibling visual directory must be seeded");
    fs::write(sibling.join("scene.json"), b"{}").expect("sibling scene must be seeded");
    fs::copy(fixture_jpeg(), sibling.join("picture.jpg")).expect("sibling picture must be seeded");
    let card = CardCell::new(
        cache.path().to_path_buf(),
        &LanguagePair::new("FR", "EN"),
        "canard",
        "a duck",
    )
    .cache();
    let guard = card
        .visual(revision)
        .expect("sibling revision must be valid")
        .hold_visual(Duration::ZERO)
        .expect("sibling revision must be leased");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_kamishibai"))
        .args(["rm", "jpurge", "--cache", "--json"])
        .env("KAMISHIBAI_CACHE", cache.path())
        .env("GEMINI_API_KEY", "offline-dummy-key")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("rm --cache must spawn");
    let started = Instant::now();
    let blocked = loop {
        if child
            .try_wait()
            .expect("rm --cache must remain observable")
            .is_some()
        {
            break false;
        }
        if started.elapsed() >= Duration::from_millis(100) {
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(guard);
    let output = child
        .wait_with_output()
        .expect("rm --cache must finish after the visual lease is released");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("rm --cache must return JSON");
    assert_eq!(
        (
            output.status.success(),
            blocked,
            value["removed"].as_str(),
            cell.exists(),
            cache.path().join("sessions/jpurge").exists(),
        ),
        (true, true, Some("jpurge"), false, false),
        "rm --cache must wait for sibling visual work before deleting every card revision"
    );
}

#[test]
fn a_missing_session_in_json_mode_prints_the_error_envelope_with_exit_three() {
    let cache = TempDir::new().expect("cache tempdir");
    let output = cli(cache.path())
        .args(["status", "ghost", "--json"])
        .output()
        .expect("status must run");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must carry the error envelope");
    assert_eq!(
        (
            output.status.code(),
            value["ok"].as_bool(),
            value["error"]["code"].as_str(),
        ),
        (Some(3), Some(false), Some("not_found")),
        "a refusal in JSON mode must keep its exit code and put the envelope on stdout"
    );
}

#[test]
fn an_all_failed_waited_run_in_json_mode_prints_the_envelope_not_a_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let gemini = failing_gemini();
    understood_session_json(cache.path(), out.path(), "jfail", CARDS_JSON);
    let output = cli_at(cache.path(), gemini.as_str())
        .args(["generate", "--wait", "jfail", "--json"])
        .timeout(Duration::from_secs(120))
        .output()
        .expect("generate --wait --json must run");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must carry the error envelope");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let warnings = events
        .iter()
        .filter(|event| event["event"].as_str() == Some("warning"))
        .count();
    let unnamed_events = events
        .iter()
        .filter(|event| event["event"].as_str().is_none())
        .count();
    assert_eq!(
        (
            output.status.code(),
            value["ok"].as_bool(),
            value["error"]["exit"].as_u64(),
            warnings > 0,
            unnamed_events,
        ),
        (Some(1), Some(false), Some(1), true, 0),
        "an all-failed waited run in JSON mode must print one error envelope and tagged warning events"
    );
}

#[test]
fn combining_json_with_quiet_is_refused_with_the_usage_code() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path())
        .args(["status", "any", "-q", "--json"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn cache_path_in_json_mode_prints_the_cache_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let value = json_stdout(cli(cache.path()).args(["cache-path", "--json"]));
    assert_eq!(
        value["cache"].as_str(),
        cache.path().to_str(),
        "cache-path --json must carry the cache directory as a document field"
    );
}

#[test]
fn a_lone_session_resolves_when_the_id_is_omitted() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "solo", CARDS_JSON);
    let output = cli(cache.path())
        .args(["status"])
        .output()
        .expect("status must run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert_eq!(
        (
            output.status.code(),
            stdout.contains("your session solo"),
            stdout.contains("understood"),
        ),
        (Some(0), true, true),
        "with one session an omitted id must resolve to it, naming it in the header"
    );
}

#[test]
fn an_unsettled_session_wins_resolution_over_a_published_one() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "older", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "older"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    understood_session(cache.path(), out.path(), "fresh", TWO_CARDS_JSON);
    let phase = cli(cache.path())
        .args(["status"])
        .output()
        .expect("status must run");
    assert_eq!(
        (
            phase.status.code(),
            String::from_utf8(phase.stdout)
                .expect("utf8")
                .contains("your session fresh"),
        ),
        (Some(0), true),
        "the single unfinished session must win resolution over a published one"
    );
}

#[test]
fn two_curatable_sessions_make_an_omitted_id_ambiguous() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "first", CARDS_JSON);
    understood_session(cache.path(), out.path(), "second", TWO_CARDS_JSON);
    let output = cli(cache.path())
        .args(["status"])
        .output()
        .expect("status must run");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert_eq!(
        (
            output.status.code(),
            stderr.contains("first") && stderr.contains("second"),
            stderr.contains("and "),
        ),
        (Some(5), true, false),
        "two unfinished sessions must exit 5 listing both, with no and-N-more line"
    );
}

#[test]
fn seven_sessions_print_the_newest_five_and_a_more_line() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    for index in 1..=7 {
        understood_session(
            cache.path(),
            out.path(),
            format!("s{index}").as_str(),
            CARDS_JSON,
        );
    }
    let output = cli(cache.path())
        .args(["generate"])
        .output()
        .expect("generate must run");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let first_session_line = stderr.lines().nth(1).unwrap_or("");
    assert_eq!(
        (
            output.status.code(),
            stderr.lines().count(),
            first_session_line.starts_with("s7"),
            stderr.contains("and 2 more — kamishibai ls"),
        ),
        (Some(5), 7, true, true),
        "seven ambiguous sessions must list the newest five, newest first, plus an and-2-more line"
    );
}

#[test]
fn an_omitted_id_with_no_sessions_exits_with_the_not_found_code() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path()).args(["status"]).assert().code(3);
}

#[test]
fn an_ambiguous_omitted_id_in_json_mode_carries_the_candidate_sessions() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "left", CARDS_JSON);
    understood_session(cache.path(), out.path(), "right", TWO_CARDS_JSON);
    let output = cli(cache.path())
        .args(["status", "--json"])
        .output()
        .expect("status must run");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must carry the error envelope");
    assert_eq!(
        (
            output.status.code(),
            value["error"]["code"].as_str(),
            value["sessions"][0]["id"].as_str(),
            value["sessions"][1]["id"].as_str(),
        ),
        (Some(5), Some("ambiguous"), Some("right"), Some("left")),
        "an ambiguous omitted id in JSON mode must carry the candidates newest-first in the envelope"
    );
}

#[test]
fn result_without_an_id_on_an_unpublished_session_exits_not_ready() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "pending", CARDS_JSON);
    let output = cli(cache.path())
        .args(["result"])
        .output()
        .expect("result must run");
    assert_eq!(
        (
            output.status.code(),
            String::from_utf8(output.stderr)
                .expect("utf8")
                .contains("kamishibai generate pending"),
        ),
        (Some(4), true),
        "result without an id on an unpublished session must exit 4 naming the resolved session"
    );
}
