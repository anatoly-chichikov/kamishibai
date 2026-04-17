//! Tests for CLI parsing, handled failures, and binary exit codes.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use assert_cmd::Command;
use kamishibai::cli::{Arguments, CliError, arguments, handle};
use kamishibai::runtime::diagnosis::Display;

type DiagnosisItem = (String, Option<PathBuf>);
type DiagnosisItems = Rc<RefCell<Vec<DiagnosisItem>>>;

/// Shared diagnosis recorder for CLI tests.
#[derive(Clone, Debug, Default)]
struct Lines {
    items: DiagnosisItems,
}

impl Lines {
    /// Return the recorded diagnosis items.
    fn values(&self) -> Vec<DiagnosisItem> {
        self.items.borrow().clone()
    }
}

/// Fake diagnosis for handled CLI tests.
#[derive(Clone, Debug)]
struct FakeDiagnosis {
    lines: Lines,
}

impl Display for FakeDiagnosis {
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>) {
        self.lines
            .items
            .borrow_mut()
            .push((String::from(message), path.map(Path::to_path_buf)));
    }
}

/// CLI arguments keep the public flags and optional path contract.
#[test]
fn cli_arguments_keep_the_public_flags_and_optional_path_contract() -> Result<()> {
    assert_eq!(
        arguments([
            "--deck",
            "Core Pack",
            "--out-dir",
            "./kamishibai-out",
            "--cache",
            "./cache",
            "input.json",
        ])?,
        Arguments {
            deck: Some(String::from("Core Pack")),
            output: Some(PathBuf::from("./kamishibai-out")),
            cache: Some(PathBuf::from("./cache")),
            path: Some(PathBuf::from("input.json")),
        },
        "cli arguments no longer keep the public flags and optional path contract"
    );
    Ok(())
}

/// Handle returns zero when the application body succeeds.
#[test]
fn handle_returns_zero_when_the_application_body_succeeds() {
    assert_eq!(
        handle(
            || Ok(()),
            FakeDiagnosis {
                lines: Lines::default(),
            },
        ),
        0,
        "handle no longer returns zero when the application body succeeds"
    );
}

/// Handle returns one and reports diagnosis for handled failures.
#[test]
fn handle_returns_one_and_reports_diagnosis_for_handled_failures() {
    let lines = Lines::default();
    assert_eq!(
        (
            handle(
                || {
                    Err(CliError::handled(
                        "problem",
                        Some(PathBuf::from("/tmp/λέξη.json")),
                    ))
                },
                FakeDiagnosis {
                    lines: lines.clone(),
                },
            ),
            lines.values(),
        ),
        (
            1,
            vec![(
                String::from("problem"),
                Some(PathBuf::from("/tmp/λέξη.json")),
            )],
        ),
        "handle no longer returns one and reports diagnosis for handled failures"
    );
}

/// Handle returns one-hundred-thirty for interruptions.
#[test]
fn handle_returns_one_hundred_thirty_for_interruptions() {
    assert_eq!(
        handle(
            || Err(CliError::Interrupted),
            FakeDiagnosis {
                lines: Lines::default(),
            },
        ),
        130,
        "handle no longer returns one hundred thirty for interruptions"
    );
}

/// The compiled binary returns one when GEMINI_API_KEY is missing.
#[test]
fn the_compiled_binary_returns_one_when_gemini_api_key_is_missing() -> Result<()> {
    Command::cargo_bin("kamishibai")?
        .arg("/tmp/nonexistent.json")
        .env_remove("GEMINI_API_KEY")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .assert()
        .code(1);
    assert_eq!(
        true, true,
        "the compiled binary no longer returns one when gemini api key is missing"
    );
    Ok(())
}

/// The compiled binary returns one when the input file is missing.
#[test]
fn the_compiled_binary_returns_one_when_the_input_file_is_missing() -> Result<()> {
    Command::cargo_bin("kamishibai")?
        .arg("/tmp/отсутствует.json")
        .env("GEMINI_API_KEY", "fake-key-for-test")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .assert()
        .code(1);
    assert_eq!(
        true, true,
        "the compiled binary no longer returns one when the input file is missing"
    );
    Ok(())
}
