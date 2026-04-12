//! Smoke tests for the Rust workspace bootstrap.

use kamishibai::assets;

/// Embedded audio prompt stays aligned with the Python baseline asset.
#[test]
fn embedded_audio_prompt_matches_the_baseline_asset() {
    assert_eq!(
        assets::audio_prompt(),
        "Say in natural {language}: {text}",
        "embedded audio prompt drifted away from the baseline asset"
    );
}

/// Embedded scene prompt stays non-empty and available to the crate.
#[test]
fn embedded_scene_prompt_mentions_manga_panels() {
    assert!(
        assets::scene_prompt().contains("JSON array of manga panels"),
        "embedded scene prompt was not loaded into the Rust crate"
    );
}

/// Embedded manga template keeps the expected root object name.
#[test]
fn embedded_manga_template_keeps_the_panel_root() {
    assert!(
        assets::manga_template().contains("\"manga_panel\""),
        "embedded manga template lost the manga_panel root"
    );
}
