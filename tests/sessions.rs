//! Offline end-to-end test of the session flow — no Gemini calls.
//!
//! Drives `new --build` (which writes each card's meta to the cache), seeds the
//! remaining artifacts into the card folder so every step is a cache hit, then
//! runs `generate --wait` and checks the session reaches `published`. Uses
//! `--wait` (deterministic, foreground) rather than the background worker so the
//! test never races a detached process.

use std::fs;
use std::path::{Path, PathBuf};

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

fn cli(cache: &Path) -> Command {
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    command
        .env("KAMISHIBAI_CACHE", cache)
        .env("GEMINI_API_KEY", "offline-dummy-key");
    command
}

fn first_card_dir(cache: &Path) -> PathBuf {
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
                return cell.path();
            }
        }
    }
    panic!("`new --build` created no card folder under the cache");
}

fn fixture_jpeg() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/hero/hero.jpg");
    assert!(path.is_file(), "missing JPEG fixture for the offline test");
    path
}

fn read_rev(cache: &Path, id: &str) -> u64 {
    let path = cache.join("sessions").join(id).join("session.json");
    let text = fs::read_to_string(&path).expect("the session file must exist");
    let record: serde_json::Value =
        serde_json::from_str(text.as_str()).expect("the session file must be valid JSON");
    record
        .get("rev")
        .and_then(serde_json::Value::as_u64)
        .expect("the session record must carry a rev")
}

// Many separate processes hammer the same session's save path at once. The save
// is a compare-and-swap (a rev counter) made atomic across processes by a
// per-session write lock, so each winning save must bump the rev exactly once
// and the losers must be refused. The invariant `final rev == initial + winners`
// holds only if no two writers ever shared a base and both committed — i.e. only
// if the write lock really serialized the read-modify-write. Without it, two
// racers would both pass the check and the last rename would win, leaving the
// final rev lower than the number of reported successes (a lost update).
#[test]
fn concurrent_saves_never_lose_an_update_under_the_write_lock() {
    use std::process::{Command as Process, Stdio};
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let cards = cache.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("the cards JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "race",
            "--out",
            out.path().to_str().expect("out path is utf8"),
            "--quiet",
        ])
        .assert()
        .success();
    let before = read_rev(cache.path(), "race");
    let binary = env!("CARGO_BIN_EXE_kamishibai");
    let racers: Vec<_> = (0..16)
        .map(|_| {
            Process::new(binary)
                .args(["exclude", "race", "--card", "canard"])
                .env("KAMISHIBAI_CACHE", cache.path())
                .env("GEMINI_API_KEY", "offline-dummy-key")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("a concurrent exclude must spawn")
        })
        .collect();
    let winners = racers
        .into_iter()
        .map(|mut racer| racer.wait().expect("a racer must be reapable").success())
        .filter(|succeeded| *succeeded)
        .count();
    let after = read_rev(cache.path(), "race");
    assert_eq!(
        after,
        before + u64::try_from(winners).expect("the winner count fits in u64"),
        "every winning concurrent save must bump the rev exactly once; a lower final rev means the write lock let two writers clobber the same base"
    );
}

#[test]
fn a_fully_cached_build_session_runs_to_published_offline() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let cards = cache.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("the cards JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "offline",
            "--out",
            out.path().to_str().expect("out path is utf8"),
            "--quiet",
        ])
        .assert()
        .success();
    let cell = first_card_dir(cache.path());
    fs::write(cell.join("voice.wav"), b"RIFFxxxxWAVE").expect("seed voice");
    fs::write(cell.join("scene.json"), b"{}").expect("seed scene");
    fs::copy(fixture_jpeg(), cell.join("illustration.jpg")).expect("seed illustration");
    cli(cache.path())
        .args(["generate", "--wait", "offline", "--quiet"])
        .assert()
        .success();
    let status = cli(cache.path())
        .args(["status", "offline", "--quiet"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let phase = String::from_utf8(status).expect("status output must be UTF-8");
    assert_eq!(
        phase.trim(),
        "published",
        "a fully-cached build session must run to published with no Gemini calls"
    );
}

#[test]
fn cancelling_a_published_session_keeps_it_published() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let cards = cache.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("the cards JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "keep",
            "--out",
            out.path().to_str().expect("out path is utf8"),
            "--quiet",
        ])
        .assert()
        .success();
    let cell = first_card_dir(cache.path());
    fs::write(cell.join("voice.wav"), b"RIFFxxxxWAVE").expect("seed voice");
    fs::write(cell.join("scene.json"), b"{}").expect("seed scene");
    fs::copy(fixture_jpeg(), cell.join("illustration.jpg")).expect("seed illustration");
    cli(cache.path())
        .args(["generate", "--wait", "keep", "--quiet"])
        .assert()
        .success();
    cli(cache.path())
        .args(["cancel", "keep"])
        .assert()
        .success();
    let status = cli(cache.path())
        .args(["status", "keep", "--quiet"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(status)
            .expect("status output must be UTF-8")
            .trim(),
        "published",
        "cancelling an already-published session must leave it published, not cancelled"
    );
}

#[test]
fn regenerate_before_generate_is_refused() {
    let cache = TempDir::new().expect("cache tempdir");
    let out = TempDir::new().expect("output tempdir");
    let cards = cache.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("the cards JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "fresh",
            "--out",
            out.path().to_str().expect("out path is utf8"),
            "--quiet",
        ])
        .assert()
        .success();
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
        .failure();
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
    let cards = cache.path().join("cards.json");
    fs::write(&cards, CARDS_JSON).expect("the cards JSON must write");
    cli(cache.path())
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path is utf8"),
            "--id",
            "curate",
            "--out",
            out.path().to_str().expect("out path is utf8"),
            "--quiet",
        ])
        .assert()
        .success();
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
        .failure();
    assert_eq!(
        String::from_utf8(understood)
            .expect("status output must be UTF-8")
            .trim(),
        "understood",
        "excluding the only card must keep the session understood with nothing to generate"
    );
}
