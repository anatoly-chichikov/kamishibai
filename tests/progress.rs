//! Tests for plain and terminal progress output.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kamishibai::generation::manga::Progress as SceneProgress;
use kamishibai::generation::{BuildProgress, SkippedCard};
use kamishibai::runtime::progress::{
    AlignedStatus, AppProgress, Console, Live, PlainProgress, ProgressSelector, RichProgress,
    SelectedProgress, Spinner, Status,
};
/// Shared line recorder for progress tests.
#[derive(Clone, Debug, Default)]
struct Lines {
    items: Rc<RefCell<Vec<String>>>,
}

impl Lines {
    /// Return the recorded lines.
    fn values(&self) -> Vec<String> {
        self.items.borrow().clone()
    }
}

/// Shared highlight recorder for rich progress tests.
#[derive(Clone, Debug, Default)]
struct Highlights {
    items: Rc<RefCell<Vec<bool>>>,
}

impl Highlights {
    /// Return the recorded highlight flags.
    fn values(&self) -> Vec<bool> {
        self.items.borrow().clone()
    }
}

/// Fake output for plain progress tests.
#[derive(Clone, Debug)]
struct FakeOutput {
    lines: Lines,
}

impl Console for FakeOutput {
    /// Print one output line.
    fn print(&mut self, text: &str, _highlight: bool) {
        self.lines.items.borrow_mut().push(String::from(text));
    }
}

/// Fake console for rich progress tests.
#[derive(Clone, Debug)]
struct FakeConsole {
    highlights: Highlights,
    lines: Lines,
}

impl Console for FakeConsole {
    /// Print one terminal line.
    fn print(&mut self, text: &str, highlight: bool) {
        self.lines.items.borrow_mut().push(String::from(text));
        self.highlights.items.borrow_mut().push(highlight);
    }
}

/// Fake live region for aligned status tests.
#[derive(Clone, Debug)]
struct FakeLive {
    lines: Lines,
}

impl Live for FakeLive {
    /// Start the live region.
    fn start(&mut self) {
        self.lines
            .items
            .borrow_mut()
            .push(String::from("live:start"));
    }

    /// Stop the live region.
    fn stop(&mut self) {
        self.lines
            .items
            .borrow_mut()
            .push(String::from("live:stop"));
    }
}

/// Fake spinner for aligned status tests.
#[derive(Clone, Debug)]
struct FakeSpinner {
    lines: Lines,
}

impl Spinner for FakeSpinner {
    /// Update the spinner text.
    fn update(&mut self, text: &str) {
        self.lines
            .items
            .borrow_mut()
            .push(format!("spinner:{text}"));
    }
}

/// Fake status for rich progress tests.
#[derive(Clone, Debug)]
struct FakeStatus {
    lines: Lines,
}

impl Status for FakeStatus {
    /// Update the visible status text.
    fn update(&mut self, text: &str) {
        self.lines
            .items
            .borrow_mut()
            .push(format!("spinner:{text}"));
    }

    /// Start the status indicator.
    fn start(&mut self) {
        self.lines
            .items
            .borrow_mut()
            .push(String::from("spinner:start"));
    }

    /// Stop the status indicator.
    fn stop(&mut self) {
        self.lines
            .items
            .borrow_mut()
            .push(String::from("spinner:stop"));
    }
}

/// Return one absolute path under the repository root.
fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// Return lines with the repository root replaced by the fixture token.
fn masked(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|item| item.replace(env!("CARGO_MANIFEST_DIR"), "$REPO"))
        .collect()
}

/// Return the frozen rich progress reference manifest.
fn rich() -> Vec<String> {
    serde_json::from_str::<Vec<String>>(
        std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("reference")
                .join("manifests")
                .join("rich-progress.json"),
        )
        .expect("rich progress manifest must exist")
        .as_str(),
    )
    .expect("rich progress manifest must parse")
}

/// Aligned status delegates update, start, and stop to the wrapped objects.
#[test]
fn aligned_status_delegates_update_start_and_stop_to_the_wrapped_objects() {
    let lines = Lines::default();
    let mut status = AlignedStatus::new(
        FakeLive {
            lines: lines.clone(),
        },
        FakeSpinner {
            lines: lines.clone(),
        },
    );
    status.update("tásk...");
    status.start();
    status.stop();
    assert_eq!(
        lines.values(),
        vec![
            String::from("spinner:tásk..."),
            String::from("live:start"),
            String::from("live:stop"),
        ],
        "aligned status no longer delegates update start and stop to the wrapped objects"
    );
}

/// Plain progress keeps the frozen line formatting contract.
#[test]
fn plain_progress_keeps_the_frozen_line_formatting_contract() {
    let lines = Lines::default();
    let mut progress = PlainProgress::new(FakeOutput {
        lines: lines.clone(),
    });
    progress.card(3, 10, "über");
    progress.step("ignored");
    progress.done(
        "Generating audio",
        "generated",
        Some(Path::new("/tmp/demo.wav")),
    );
    progress.retry("Rendering", 2, "Ошибка");
    progress.skip("wörd", "problem");
    progress.result("Anki deck", Path::new("/tmp/output/cards.apkg"));
    progress.finish(9, 10, &[SkippedCard::new("wörd", "problem")]);
    assert_eq!(
        lines.values(),
        vec![
            String::from("Processing card 3/10: über"),
            String::from("  Generating audio: generated (demo.wav)"),
            String::from("  Ошибка (attempt 2), retrying..."),
            String::from("  Skipping wörd - problem"),
            String::from("  Anki deck: cards.apkg (/tmp/output/cards.apkg)"),
            String::from("\nProcessed 9/10 cards"),
            String::from("Skipped 1 card(s):"),
            String::from("  - wörd: problem"),
        ],
        "plain progress no longer keeps the frozen line formatting contract"
    );
}

/// Rich progress keeps the frozen spinner and markup sequence.
#[test]
fn rich_progress_keeps_the_frozen_spinner_and_markup_sequence() {
    let lines = Lines::default();
    let highlights = Highlights::default();
    let mut progress = RichProgress::new(
        FakeConsole {
            highlights: highlights.clone(),
            lines: lines.clone(),
        },
        FakeStatus {
            lines: lines.clone(),
        },
    );
    progress.card(1, 2, "кошка");
    progress.step("Generating audio");
    progress.done(
        "Generating audio",
        "generated",
        Some(repo("output/deck.apkg").as_path()),
    );
    progress.step("Composing scene");
    progress.done(
        "Composing scene",
        "translated",
        Some(repo("output/deck.json").as_path()),
    );
    progress.step("Rendering manga");
    progress.done(
        "Rendering manga",
        "rendered",
        Some(repo("output/deck.jpg").as_path()),
    );
    progress.skip("слово", "Cannot generate audio for empty text");
    progress.result("Anki deck", repo("output/deck.apkg").as_path());
    progress.finish(
        1,
        2,
        &[SkippedCard::new(
            "слово",
            "Cannot generate audio for empty text",
        )],
    );
    assert_eq!(
        masked(lines.values().as_slice()),
        rich(),
        "rich progress no longer keeps the frozen spinner and markup sequence"
    );
}

/// Rich progress disables highlight on path-bearing lines.
#[test]
fn rich_progress_disables_highlight_on_path_bearing_lines() {
    let lines = Lines::default();
    let highlights = Highlights::default();
    let mut progress = RichProgress::new(
        FakeConsole {
            highlights: highlights.clone(),
            lines,
        },
        FakeStatus {
            lines: Lines::default(),
        },
    );
    progress.done(
        "Composing scene",
        "cached",
        Some(Path::new("/tmp/greek_2026-02-10.json")),
    );
    progress.result("Anki deck", Path::new("/tmp/greek_2026-02-10.apkg"));
    assert_eq!(
        highlights.values(),
        vec![false, false],
        "rich progress no longer disables highlight on path bearing lines"
    );
}

/// Progress selector returns the expected mode for terminal capability.
#[test]
fn progress_selector_returns_the_expected_mode_for_terminal_capability() {
    assert_eq!(
        (
            matches!(
                ProgressSelector::new(false).selected(),
                SelectedProgress::Plain(_)
            ),
            matches!(
                ProgressSelector::new(true).selected(),
                SelectedProgress::Rich(_)
            ),
        ),
        (true, true),
        "progress selector no longer returns the expected mode for terminal capability"
    );
}
