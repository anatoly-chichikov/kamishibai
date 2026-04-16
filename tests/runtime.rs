//! Tests for runtime prompt helpers and media wiring.

use std::path::Path;

use anyhow::Result;
use kamishibai::generation::manga::ImageSource;
use kamishibai::generation::prompts as assets;
use kamishibai::generation::{GeneratorCatalog, SceneSource, Speaker};
use kamishibai::vocabulary::VocabularyEntry;
use serde_json::Value;

/// Fake runtime client for media wiring tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FakeClient;

impl Speaker for FakeClient {
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

/// Return one normalized entry for runtime tests.
fn entry(target: &str) -> VocabularyEntry {
    VocabularyEntry {
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
        assets::render_audio_prompt("Language_x1"),
        "Say in natural Language_x1: {text}",
        "audio prompt rendering no longer keeps the shared template semantics"
    );
}

/// Scene prompt rendering keeps the target language and JSON braces.
#[test]
fn scene_prompt_rendering_keeps_the_target_language_and_json_braces() {
    let prompt = assets::render_scene_prompt("Language_x1");
    assert_eq!(
        (
            prompt.contains("educational Language_x1 flashcards"),
            prompt.contains("\"x\": int"),
        ),
        (true, true),
        "scene prompt rendering no longer keeps the target language and json braces"
    );
}

/// GeneratorCatalog resolves the supported profile cache roots for both service types.
#[test]
fn media_resolves_the_supported_profile_cache_roots_for_both_service_types() -> Result<()> {
    let directory = tempfile::TempDir::new()?;
    let mut media = GeneratorCatalog::new(FakeClient, directory.path());
    let audio = media.audio(&entry("en"))?;
    let illustration = media.illustration(&entry("en"))?;
    assert_eq!(
        (
            audio
                .filepath("tone.wav")?
                .ends_with(Path::new("audio-en").join("tone.wav")),
            illustration
                .filepath("panel.jpg")?
                .ends_with(Path::new("manga-en").join("panel.jpg")),
        ),
        (true, true),
        "media no longer resolves the supported profile cache roots for both service types"
    );
    Ok(())
}

/// GeneratorCatalog keeps the registry fallback OCR policy in illustration wiring.
#[test]
fn media_keeps_the_registry_fallback_ocr_policy_in_illustration_wiring() -> Result<()> {
    let directory = tempfile::TempDir::new()?;
    let mut media = GeneratorCatalog::new(FakeClient, directory.path());
    assert!(
        format!("{:?}", media.illustration(&entry("en"))?).contains("\"eng\""),
        "media no longer keeps the registry fallback ocr policy in illustration wiring"
    );
    Ok(())
}
