//! Checks the typed TUI recording procedure against the tapes and helper files on disk.

use std::fs;
use std::path::Path;

use recording::Recording;

#[path = "common/tui_recording.rs"]
mod recording;

#[test]
fn recording_procedure_cannot_drift_from_its_tapes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let document = fs::read_to_string(root.join("docs/tui-states/recording.ron"))
        .expect("invariant: TUI recording procedure must be readable");
    let faults = Recording::try_from_document(&document)
        .expect("invariant: TUI recording procedure must match its typed schema")
        .inspect(root);
    assert!(
        faults.is_empty(),
        "the recording procedure disagrees with the tapes on disk: {faults:#?}"
    );
}
