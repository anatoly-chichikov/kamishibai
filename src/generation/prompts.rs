//! Embedded asset accessors for the Rust rewrite baseline.

use std::{fmt::Write as _, sync::OnceLock};

use sha2::{Digest, Sha256};

const AUDIO_PROMPT: &str = include_str!("../../assets/audio_prompt.txt");
const MANGA_TEMPLATE: &str = include_str!("../../assets/manga_template.json");
const LAYOUT_FEATURES_PROMPT: &str = include_str!("../../assets/layout_features_prompt.txt");
const LAYOUT_SCENE_PROMPT: &str = include_str!("../../assets/layout_scene_prompt.txt");
const LAYOUT_SCENE_SCHEMA: &str = include_str!("../../assets/layout_scene_schema.json");
const LAYOUT_REGISTRY: &str = include_str!("../../assets/layout_registry_v2.json");
const DEVICE_REGISTRY: &str = include_str!("../../assets/device_registry_v3.json");
const LAYOUT_POLICY_VERSION: &str =
    "kamishibai-layout-registry-production-v36-safe-structural-recovery";
static VISUAL_REVISION: OnceLock<String> = OnceLock::new();

/// Return the embedded shared audio prompt template.
pub fn audio_prompt() -> &'static str {
    AUDIO_PROMPT.trim()
}

/// Return the embedded registry feature-extraction prompt.
pub(crate) fn layout_features_prompt() -> &'static str {
    LAYOUT_FEATURES_PROMPT.trim()
}

/// Return the embedded registry-bound scene-composition prompt.
pub(crate) fn layout_scene_prompt() -> &'static str {
    LAYOUT_SCENE_PROMPT.trim()
}

/// Return the embedded registry scene-composer response schema.
pub(crate) fn layout_scene_schema() -> &'static str {
    LAYOUT_SCENE_SCHEMA.trim()
}

/// Return the embedded layout registry JSON document.
pub(crate) fn layout_registry() -> &'static str {
    LAYOUT_REGISTRY.trim()
}

/// Return the embedded operational device registry JSON document.
pub(crate) fn device_registry() -> &'static str {
    DEVICE_REGISTRY.trim()
}

/// Return the embedded manga template JSON document.
pub fn manga_template() -> &'static str {
    MANGA_TEMPLATE.trim()
}

/// Render the audio prompt template for one language.
pub fn render_audio_prompt(language: &str) -> String {
    audio_prompt().replace("{language}", language)
}

/// Return the stable SHA-256 revision of the versioned production visual policy.
pub fn visual_revision() -> &'static str {
    VISUAL_REVISION
        .get_or_init(|| {
            let mut digest = Sha256::new();
            digest.update(LAYOUT_POLICY_VERSION.as_bytes());
            digest.update([0]);
            digest.update(layout_features_prompt().as_bytes());
            digest.update([0]);
            digest.update(layout_scene_prompt().as_bytes());
            digest.update([0]);
            digest.update(layout_scene_schema().as_bytes());
            digest.update([0]);
            digest.update(layout_registry().as_bytes());
            digest.update([0]);
            digest.update(device_registry().as_bytes());
            digest.update([0]);
            digest.update(manga_template().as_bytes());
            digest
                .finalize()
                .iter()
                .fold(String::with_capacity(64), |mut revision, byte| {
                    write!(&mut revision, "{byte:02x}")
                        .expect("invariant: writing hexadecimal bytes to a string cannot fail");
                    revision
                })
        })
        .as_str()
}
