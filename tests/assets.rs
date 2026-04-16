//! Tests for embedded Rust assets.

use kamishibai::infrastructure::assets;

/// Embedded audio prompt stays aligned with the Python baseline asset.
#[test]
fn embedded_audio_prompt_matches_the_baseline_asset() {
    assert_eq!(
        assets::audio_prompt(),
        "Say in natural {language}: {text}",
        "embedded audio prompt drifted away from the baseline asset"
    );
}

/// Rendered audio prompt interpolates the target language.
#[test]
fn rendered_audio_prompt_inserts_the_target_language() {
    assert_eq!(
        assets::render_audio_prompt("Greek"),
        "Say in natural Greek: {text}",
        "rendered audio prompt did not interpolate the target language"
    );
}

/// Rendered scene prompt keeps the target language in the instruction header.
#[test]
fn rendered_scene_prompt_mentions_the_target_language() {
    assert!(
        assets::render_scene_prompt("Spanish").contains("educational Spanish flashcards"),
        "rendered scene prompt did not include the target language"
    );
}

/// Rendered scene prompt keeps the JSON schema braces intact.
#[test]
fn rendered_scene_prompt_keeps_the_json_schema_braces() {
    assert!(
        assets::render_scene_prompt("English").contains("\"x\": int"),
        "rendered scene prompt lost the embedded JSON schema braces"
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
