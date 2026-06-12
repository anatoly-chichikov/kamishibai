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
use std::time::{Duration, Instant};

use assert_cmd::Command;
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
            "--quiet",
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
    fs::write(cell.join("voice.wav"), b"RIFFxxxxWAVE").expect("seed voice");
    fs::write(cell.join("scene.json"), b"{}").expect("seed scene");
    fs::copy(fixture_jpeg(), cell.join("illustration.jpg")).expect("seed illustration");
}

/// Poll `status -q` until the phase satisfies the predicate, panicking with the
/// last seen phase when the deadline expires (a harness failure, not a verdict).
fn poll_quiet_status(
    cache: &Path,
    id: &str,
    deadline: Duration,
    until: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    loop {
        let output = cli(cache)
            .args(["status", id, "--quiet"])
            .output()
            .expect("status must run");
        let phase = String::from_utf8(output.stdout)
            .expect("status output must be UTF-8")
            .trim()
            .to_string();
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

/// Read the recorded worker pid out of one session's file.
#[cfg(unix)]
fn worker_pid(cache: &Path, id: &str) -> i64 {
    let path = cache.join("sessions").join(id).join("session.json");
    let text = fs::read_to_string(&path).expect("the session file must exist");
    let record: serde_json::Value =
        serde_json::from_str(text.as_str()).expect("the session file must be valid JSON");
    record["worker"]["pid"]
        .as_i64()
        .expect("the session must record a worker pid")
}

/// Seed a session whose detached worker provably stays alive: every artifact
/// except the voice is cached, so the worker blocks on one TTS request against
/// the stalled listener. Returns once `status` reports the worker alive.
#[cfg(unix)]
fn live_worker_session(cache: &Path, out: &Path, id: &str, gemini: &str) {
    understood_session(cache, out, id, CARDS_JSON);
    let cell = first_card_dir(cache);
    fs::write(cell.join("scene.json"), b"{}").expect("seed scene");
    fs::copy(fixture_jpeg(), cell.join("illustration.jpg")).expect("seed illustration");
    cli_at(cache, gemini)
        .args(["generate", id, "--quiet"])
        .assert()
        .success();
    poll_full_status(cache, id, Duration::from_secs(30), |text| {
        text.contains(" alive")
    });
}

#[test]
fn a_fully_cached_build_session_runs_to_published_offline() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "offline", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "--wait", "offline", "--quiet"])
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
fn a_detached_generate_reaches_published_while_status_polls_it() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "detached", CARDS_JSON);
    seed_artifacts(&first_card_dir(cache.path()));
    cli(cache.path())
        .args(["generate", "detached", "--quiet"])
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
        text.contains(" alive")
    });
    cli(cache.path())
        .args(["cancel", "live"])
        .assert()
        .success();
    assert!(
        status.contains("phase    generating") && status.contains(" alive"),
        "status during a detached run must report the generating phase and a live worker"
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
        .args(["generate", "--wait", "half", "--quiet"])
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
    let status = poll_full_status(cache.path(), "race", Duration::from_secs(5), |_| true);
    assert!(
        all_succeeded
            && status.contains("word  canard   skip")
            && status.contains("word  lanterne   card"),
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
        .args(["generate", "--wait", "keep", "--quiet"])
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
        .args(["generate", "--wait", "deck", "--quiet"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let stdout = cli(cache.path())
        .args(["result", "deck", "--deck"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let path = String::from_utf8(stdout)
        .expect("result output must be UTF-8")
        .trim()
        .to_string();
    assert!(
        path.ends_with(".apkg") && Path::new(path.as_str()).is_file(),
        "result --deck must print exactly one existing .apkg path"
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
        .args(["status", "broken", "--quiet"])
        .assert()
        .code(1);
}

#[test]
fn regenerate_before_generate_is_refused() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session(cache.path(), out.path(), "fresh", CARDS_JSON);
    let understood = cli(cache.path())
        .args(["status", "fresh", "--quiet"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    cli(cache.path())
        .args(["regenerate", "fresh", "--failed"])
        .assert()
        .code(2);
    assert_eq!(
        String::from_utf8(understood)
            .expect("status output must be UTF-8")
            .trim(),
        "understood",
        "regenerate before any generate must be refused, leaving the session understood"
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
    let understood = cli(cache.path())
        .args(["status", "curate", "--quiet"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    cli(cache.path())
        .args(["generate", "--wait", "curate", "--quiet"])
        .assert()
        .code(2);
    assert_eq!(
        String::from_utf8(understood)
            .expect("status output must be UTF-8")
            .trim(),
        "understood",
        "excluding the only card must keep the session understood with nothing to generate"
    );
}

#[test]
fn invoking_new_with_no_words_exits_with_the_usage_code() {
    let cache = TempDir::new().expect("cache tempdir");
    cli(cache.path()).args(["new", "--quiet"]).assert().code(2);
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
    cli(cache.path())
        .args(["status", "ghost", "--quiet"])
        .assert()
        .code(3);
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
fn excluding_a_card_in_json_mode_returns_the_post_mutation_document() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    understood_session_json(cache.path(), out.path(), "jmut", CARDS_JSON);
    let value =
        json_stdout(cli(cache.path()).args(["exclude", "jmut", "--card", "canard", "--json"]));
    assert_eq!(
        (
            value["candidates"]["items"][0]["included"].as_bool(),
            value["candidates"]["selected"].as_u64(),
        ),
        (Some(false), Some(0)),
        "exclude --json must return the document as the mutation left it, with no follow-up status needed"
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
    let unnamed_events = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["event"].as_str().is_none())
        .count();
    assert_eq!(
        (
            documents.len(),
            documents[0]["phase"].as_str(),
            documents[0]["cards"]["ready"].as_u64(),
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
        .args(["generate", "--wait", "jround", "--quiet"])
        .timeout(Duration::from_secs(120))
        .assert()
        .success();
    let value = json_stdout(cli(cache.path()).args(["result", "jround", "--json"]));
    let reimport = serde_json::json!({ "entries": value["items"] });
    let cards = cache.path().join("reimport.json");
    fs::write(&cards, reimport.to_string()).expect("the reimport JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("reimport path is utf8"),
            "--id",
            "jround2",
            "--quiet",
        ])
        .assert()
        .success()
        .stdout("jround2\n");
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
    assert_eq!(
        (
            output.status.code(),
            value["ok"].as_bool(),
            value["error"]["exit"].as_u64()
        ),
        (Some(1), Some(false), Some(1)),
        "an all-failed waited run in JSON mode must print the error envelope, never a success document"
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
