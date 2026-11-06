//! Structural, copy and provenance checks for the authored TUI intent inventory.

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::example::Example;
use catalog::Catalog;

#[path = "common/tui_catalog.rs"]
mod catalog;

fn gallery() -> Example {
    Example::built("tui_states")
}

fn fixture_names(gallery: &Example) -> Vec<String> {
    gallery
        .printed(&["--list-states"], Duration::from_secs(30))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn document() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/tui-states/catalog.ron"))
        .expect("invariant: TUI catalog must be readable")
}

fn catalog() -> Catalog {
    Catalog::try_from_document(&document())
        .expect("invariant: TUI catalog must match its typed schema")
}

#[test]
fn every_catalog_head_stays_on_one_grep_line() {
    let faults = catalog().inspect_layout(&document());
    assert!(
        faults.is_empty(),
        "catalog heads left the single line the grep recipes read: {faults:#?}"
    );
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
    let gallery = gallery();
    let renders = fixture_names(&gallery)
        .into_iter()
        .map(|name| {
            let screen = gallery.printed(
                &["--state", &name, "--render", "160x48"],
                Duration::from_secs(30),
            );
            (name, screen)
        })
        .collect::<Vec<_>>();
    let faults = catalog().inspect_renders(&renders);
    assert!(
        !renders.is_empty() && faults.is_empty(),
        "catalog headers or footers disagree with the real renderer: {faults:#?}"
    );
}
