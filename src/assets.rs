//! Embedded asset accessors for the Rust rewrite baseline.

const AUDIO_PROMPT: &str = include_str!("../assets/audio_prompt.txt");
const SCENE_PROMPT: &str = include_str!("../assets/scene_prompt.txt");
const MANGA_TEMPLATE: &str = include_str!("../assets/manga_template.json");

/// Return the embedded shared audio prompt template.
pub fn audio_prompt() -> &'static str {
    AUDIO_PROMPT.trim()
}

/// Return the embedded shared scene prompt template.
pub fn scene_prompt() -> &'static str {
    SCENE_PROMPT.trim()
}

/// Return the embedded manga template JSON document.
pub fn manga_template() -> &'static str {
    MANGA_TEMPLATE.trim()
}

/// Render the audio prompt template for one language.
pub fn render_audio_prompt(language: &str) -> String {
    audio_prompt().replace("{language}", language)
}

/// Render the scene prompt template for one language.
pub fn render_scene_prompt(language: &str) -> String {
    scene_prompt().replace("{language}", language)
}
