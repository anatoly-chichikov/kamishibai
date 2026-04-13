//! Tests for runtime prompt helpers and media wiring.

use std::path::Path;

use anyhow::Result;
use kamishibai::input::NormalizedEntry;
use kamishibai::media::{Media, Profiles, SceneSource};
use kamishibai::profile::{
    AudioProfile, DeckNaming, FontProfile, ImageProfile, LanguageProfile, UiLabels,
};
use kamishibai::scene::ImageSource;
use serde_json::Value;

/// Fake runtime client for media wiring tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FakeClient;

impl kamishibai::audio::Speaker for FakeClient {
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, _prompt: &str, _text: &str) -> Result<Vec<u8>> {
        Ok(vec![0, 0, 1, 0])
    }
}

impl SceneSource for FakeClient {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value> {
        Ok(serde_json::json!({
            "manga_panel": {
                "meta": {
                    "description": language,
                    "target_lang": target,
                    "title": sentence,
                },
                "panels": [{"scene": {"description": sentence}}],
            },
        }))
    }
}

impl ImageSource for FakeClient {
    /// Return one encoded image payload for the scene and word.
    fn image(&self, _scene: &Value, _word: &str) -> Result<Vec<u8>> {
        Ok(include_bytes!("../assets/manga_template.json").to_vec())
    }
}

/// Fake profile registry for runtime tests.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FakeProfiles {
    fallback: String,
    item: LanguageProfile,
}

impl FakeProfiles {
    /// Create one fake profile registry.
    fn new() -> Self {
        Self {
            fallback: String::from("osd"),
            item: LanguageProfile::new(
                "ga",
                AudioProfile::new("Irish", "audio-ga"),
                ImageProfile::new("eng+gle", "manga-ga"),
                DeckNaming::new("Irish Vocabulary", "ga", "kamishibai.json"),
                FontProfile::new("DejaVu Sans"),
                UiLabels::new("Translation", "Context", "Hint", "Importance"),
            ),
        }
    }
}

impl Profiles for FakeProfiles {
    /// Return one language profile for the target code.
    fn item(&self, code: &str) -> Result<LanguageProfile> {
        if code == self.item.code() {
            return Ok(self.item.clone());
        }
        Err(anyhow::anyhow!("Unsupported target language '{code}'"))
    }

    /// Return the fallback OCR selection.
    fn fallback(&self) -> &str {
        self.fallback.as_str()
    }
}

/// Return one normalized entry for runtime tests.
fn entry(target: &str) -> NormalizedEntry {
    NormalizedEntry {
        word: String::from("focal"),
        pronunciation: String::new(),
        translation: String::from("значение"),
        example: String::from("frása"),
        source_lang: String::from("ru"),
        target_lang: String::from(target),
        sentence: String::from("пример"),
        highlight: String::new(),
        hint: String::new(),
        context: String::new(),
        importance: String::new(),
        transcription: String::new(),
    }
}

/// Audio prompt rendering keeps the shared template semantics.
#[test]
fn audio_prompt_rendering_keeps_the_shared_template_semantics() {
    assert_eq!(
        kamishibai::assets::render_audio_prompt("Language_x1"),
        "Say in natural Language_x1: {text}",
        "audio prompt rendering no longer keeps the shared template semantics"
    );
}

/// Scene prompt rendering keeps the target language and JSON braces.
#[test]
fn scene_prompt_rendering_keeps_the_target_language_and_json_braces() {
    let prompt = kamishibai::assets::render_scene_prompt("Language_x1");
    assert_eq!(
        (
            prompt.contains("educational Language_x1 flashcards"),
            prompt.contains("\"x\": int"),
        ),
        (true, true),
        "scene prompt rendering no longer keeps the target language and json braces"
    );
}

/// Media supports injected profiles for both cache roots.
#[test]
fn media_supports_injected_profiles_for_both_cache_roots() -> Result<()> {
    let directory = tempfile::TempDir::new()?;
    let media = Media::configured(FakeClient, directory.path(), FakeProfiles::new());
    let audio = media.audio(&entry("ga"))?;
    let illustration = media.illustration(&entry("ga"))?;
    assert_eq!(
        (
            audio
                .filepath("tone.wav")?
                .ends_with(Path::new("audio-ga").join("tone.wav")),
            illustration
                .filepath("panel.jpg")?
                .ends_with(Path::new("manga-ga").join("panel.jpg")),
        ),
        (true, true),
        "media no longer supports injected profiles for both cache roots"
    );
    Ok(())
}

/// Media keeps the injected fallback OCR policy in illustration wiring.
#[test]
fn media_keeps_the_injected_fallback_ocr_policy_in_illustration_wiring() -> Result<()> {
    let directory = tempfile::TempDir::new()?;
    let media = Media::configured(FakeClient, directory.path(), FakeProfiles::new());
    assert!(
        format!("{:?}", media.illustration(&entry("ga"))?).contains("\"osd\""),
        "media no longer keeps the injected fallback ocr policy in illustration wiring"
    );
    Ok(())
}
