//! Tests for plain and terminal diagnosis output.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kamishibai::presentation::diagnosis::{
    DiagnosisSelector, Display, PlainDiagnosis, RichDiagnosis, SelectedDiagnosis,
};
use kamishibai::presentation::progress::Console;
use serde_json::Value;

/// Shared line recorder for diagnosis tests.
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

/// Fake output for plain diagnosis tests.
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

/// Fake console for rich diagnosis tests.
#[derive(Clone, Debug)]
struct FakeConsole {
    lines: Lines,
}

impl Console for FakeConsole {
    /// Print one terminal renderable.
    fn print(&mut self, text: &str, _highlight: bool) {
        self.lines.items.borrow_mut().push(String::from(text));
    }
}

/// Return lines with the repository root replaced by the fixture token.
fn masked(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|item| item.replace(env!("CARGO_MANIFEST_DIR"), "$REPO"))
        .collect()
}

/// Return the frozen diagnosis manifest.
fn manifest() -> Value {
    serde_json::from_str(
        std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("reference")
                .join("manifests")
                .join("diagnosis.json"),
        )
        .expect("diagnosis manifest must exist")
        .as_str(),
    )
    .expect("diagnosis manifest must parse")
}

/// Plain diagnosis keeps the frozen prefix and file output.
#[test]
fn plain_diagnosis_keeps_the_frozen_prefix_and_file_output() {
    let lines = Lines::default();
    let mut display = PlainDiagnosis::new(FakeOutput {
        lines: lines.clone(),
    });
    display.show(
        "problem",
        Some(Path::new(&format!(
            "{}/tmp/broken.json",
            env!("CARGO_MANIFEST_DIR")
        ))),
    );
    assert_eq!(
        masked(lines.values().as_slice()),
        manifest()["plain"]
            .as_array()
            .expect("plain diagnosis manifest must be an array")
            .iter()
            .map(|item| String::from(
                item.as_str()
                    .expect("plain diagnosis lines must be strings")
            ))
            .collect::<Vec<_>>(),
        "plain diagnosis no longer keeps the frozen prefix and file output"
    );
}

/// Rich diagnosis emits exactly one terminal renderable.
#[test]
fn rich_diagnosis_emits_exactly_one_terminal_renderable() {
    let lines = Lines::default();
    let mut display = RichDiagnosis::new(FakeConsole {
        lines: lines.clone(),
    });
    display.show("Ключ не задан", Some(Path::new("/tmp/вокабуляр.json")));
    assert_eq!(
        lines.values().len(),
        manifest()["rich_count"]
            .as_u64()
            .expect("rich diagnosis manifest must contain a count") as usize,
        "rich diagnosis no longer emits exactly one terminal renderable"
    );
}

/// Diagnosis selector returns the expected mode for terminal capability.
#[test]
fn diagnosis_selector_returns_the_expected_mode_for_terminal_capability() {
    assert_eq!(
        (
            matches!(
                DiagnosisSelector::new(false).selected(),
                SelectedDiagnosis::Plain(_)
            ),
            matches!(
                DiagnosisSelector::new(true).selected(),
                SelectedDiagnosis::Rich(_)
            ),
        ),
        (true, true),
        "diagnosis selector no longer returns the expected mode for terminal capability"
    );
}
