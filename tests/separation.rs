//! Layering enforcement: the console/API layer must never link the TUI side.
//!
//! The console closure is `error.rs`, `console.rs`, the workflow ports
//! (`card_workflow.rs`), the Gemini workflow, and everything under
//! `cli/session/`. `cli.rs` itself stays out of the closure: it is the
//! composition root that legitimately wires both sides together through the
//! `SessionOpener` port. The TUI side (`shell`, `terminal`, `bridge`, `batch`,
//! `src/tui`) depends on the console one-way; any TUI-side reference showing
//! up in the console closure is a layering regression.

use std::fs;
use std::path::{Path, PathBuf};

fn console_sources() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<PathBuf> = [
        "src/cli/error.rs",
        "src/cli/console.rs",
        "src/cli/card_workflow.rs",
    ]
    .iter()
    .map(|file| root.join(file))
    .collect();
    sources.extend(rust_sources(root.join("src/cli/gemini_workflow").as_path()));
    sources.extend(rust_sources(root.join("src/cli/session").as_path()));
    sources
}

fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory).expect("source directory must be readable") {
        let path = entry.expect("source entry must be readable").path();
        if path.is_dir() {
            sources.extend(rust_sources(path.as_path()));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources
}

/// Detect any reference to the TUI side, including ones hidden inside grouped
/// imports (`use crate::{…, tui::App}`), by normalizing braces and spaces away.
fn links_tui(path: &Path) -> bool {
    let text = fs::read_to_string(path)
        .expect("console source must be readable")
        .replace(['{', ' '], "");
    [
        "tui::",
        "super::bridge",
        "super::terminal",
        "super::shell",
        "super::batch",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

#[test]
fn the_console_layer_never_imports_the_tui() {
    let offenders: Vec<String> = console_sources()
        .iter()
        .filter(|path| links_tui(path))
        .map(|path| path.display().to_string())
        .collect();
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "the console layer must not import the TUI"
    );
}

#[test]
fn the_legacy_live_generator_module_cannot_return() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !root.join("src/cli/live_generator.rs").exists(),
        "the implementation lost its Gemini workflow vocabulary"
    );
}
