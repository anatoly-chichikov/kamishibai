//! Layering enforcement: the console/API layer must never link the TUI side.
//!
//! The console closure is `error.rs`, `console.rs`, the workflow ports
//! (`card_workflow.rs`), the live generator, and everything under
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
        "src/cli/live_generator.rs",
    ]
    .iter()
    .map(|file| root.join(file))
    .collect();
    let session = root.join("src/cli/session");
    for entry in fs::read_dir(&session).expect("session dir must be readable") {
        let path = entry.expect("session entry must be readable").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
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
