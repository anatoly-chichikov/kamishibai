//! Structural, copy and provenance checks for the authored TUI intent inventory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use catalog::Catalog;

#[path = "common/tui_catalog.rs"]
mod catalog;

fn gallery() -> PathBuf {
    let build = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "build",
            "--example",
            "tui_states",
            "--offline",
            "--locked",
            "--quiet",
        ])
        .output()
        .expect("invariant: capture example build must be launchable");
    if !build.status.success() {
        panic!(
            "capture example build failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );
    }
    std::env::current_exe()
        .expect("invariant: running test executable must have a path")
        .parent()
        .and_then(Path::parent)
        .expect("invariant: test executable must live in the profile deps directory")
        .join("examples")
        .join(format!("tui_states{}", std::env::consts::EXE_SUFFIX))
}

fn printed(binary: &Path, arguments: &[&str]) -> String {
    let output = Command::new(binary)
        .args(arguments)
        .output()
        .expect("invariant: capture gallery must be launchable");
    if !output.status.success() {
        panic!(
            "capture gallery {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).expect("invariant: capture gallery prints UTF-8")
}

fn fixture_names(binary: &Path) -> Vec<String> {
    printed(binary, &["--list-states"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn catalog() -> Catalog {
    let document = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/tui-states/catalog.ron"),
    )
    .expect("invariant: TUI catalog must be readable");
    Catalog::try_from_document(&document)
        .expect("invariant: TUI catalog must match its typed schema")
}

#[test]
fn every_real_capture_fixture_requires_an_intent_state() {
    let names = fixture_names(&gallery());
    let faults = catalog().inspect_scenes(&names);
    assert!(
        !names.is_empty() && faults.is_empty(),
        "capture fixtures bypassed the inventory: {faults:#?}"
    );
}

#[test]
fn tui_catalog_cannot_publish_broken_states_routes_or_evidence() {
    let faults = catalog().inspect(Path::new(env!("CARGO_MANIFEST_DIR")));
    assert!(
        faults.is_empty(),
        "TUI inventory has broken references: {faults:#?}"
    );
}

#[test]
fn catalog_copy_cannot_drift_from_the_rendered_scenes() {
    let binary = gallery();
    let renders = fixture_names(&binary)
        .into_iter()
        .map(|name| {
            let screen = printed(&binary, &["--state", &name, "--render", "160x48"]);
            (name, screen)
        })
        .collect::<Vec<_>>();
    let faults = catalog().inspect_renders(&renders);
    assert!(
        !renders.is_empty() && faults.is_empty(),
        "catalog headers or footers disagree with the real renderer: {faults:#?}"
    );
}
