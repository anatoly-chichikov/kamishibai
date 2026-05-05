//! Tests for language profiles, naming, labels, and fonts.

use anyhow::Result;
use kamishibai::languages::{LanguageEntry, ReportLabels, catalog, language};
use kamishibai::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyEntry, VocabularySource, VocabularyTarget,
};

/// Build one strict entry for profile selection tests.
fn entry(source: &str, target: &str) -> VocabularyEntry {
    VocabularyEntry {
        term: text("word"),
        meaning: text("значение"),
        pronunciation: text("wɜːd"),
        transcription: text("word"),
        importance: score(5),
        source: VocabularySource {
            sentence: text("пример"),
            lang: code(source),
            highlight: text("пример"),
            hint: text("подсказка"),
            context: text("контекст"),
        },
        target: VocabularyTarget {
            sentence: text("example"),
            lang: code(target),
        },
    }
}

/// Return one validated text fixture.
fn text(value: &str) -> NonEmptyText {
    NonEmptyText::new(value).expect("test text must be valid")
}

/// Return one validated language fixture.
fn code(value: &str) -> LanguageCode {
    LanguageCode::new(value).expect("test language must be valid")
}

/// Return one validated importance fixture.
fn score(value: u8) -> Importance {
    Importance::new(value).expect("test importance must be valid")
}

/// Supported profiles keep the frozen runtime values.
#[test]
fn english_profile_keeps_the_frozen_runtime_values() -> Result<()> {
    let item = language("en")?;
    assert_eq!(
        (item.prompt, item.ocr, item.naming.name),
        (
            String::from("English"),
            String::from("eng"),
            String::from("English Vocabulary"),
        ),
        "english profile drifted away from the frozen runtime values"
    );
    Ok(())
}

/// Unknown profiles fail with the frozen error wording.
#[test]
fn unknown_profiles_raise_the_frozen_error_message() {
    assert_eq!(
        language("xx").unwrap_err().to_string(),
        "Unsupported language 'xx'",
        "unknown profiles did not raise the frozen validation message"
    );
}

/// The registry keeps the supported codes in stable order.
#[test]
fn registry_keeps_the_supported_codes_in_stable_order() {
    assert_eq!(
        catalog().codes(),
        ["de", "el", "en", "es", "ru", "zh"],
        "profile registry codes no longer match the frozen order"
    );
}

/// The registry keeps the fallback OCR token.
#[test]
fn registry_keeps_the_fallback_ocr_token() {
    assert_eq!(
        catalog().fallback_ocr(),
        "eng",
        "profile registry fallback OCR token drifted away from the frozen value"
    );
}

/// Custom deck overrides replace the name and derive a prefix.
#[test]
fn custom_deck_overrides_replace_the_name_and_derive_the_prefix() {
    let item = kamishibai::languages::naming(Some("Core Pack"), &[entry("ru", "en")]);
    assert_eq!(
        (item.name, item.prefix, item.default),
        (
            String::from("Core Pack"),
            String::from("core-pack"),
            String::from("kamishibai.json"),
        ),
        "custom deck overrides no longer derive the frozen naming tuple"
    );
}

/// Mixed targets keep the generic deck fallback.
#[test]
fn mixed_targets_keep_the_generic_deck_fallback() {
    let item = kamishibai::languages::naming(None, &[entry("ru", "el"), entry("ru", "zh")]);
    assert_eq!(
        item.name, "Kamishibai Deck",
        "mixed targets no longer fall back to the generic deck name"
    );
}

/// Label selection only depends on the source language.
#[test]
fn label_selection_only_depends_on_the_source_language() {
    assert_eq!(
        ReportLabels::default()
            .selected(&entry("ru", "zh"))
            .sentence,
        "Перевод",
        "label selection no longer depends only on the source language"
    );
}

/// Missing source languages keep the default labels.
#[test]
fn missing_source_languages_keep_the_default_labels() {
    struct Empty;
    impl LanguageEntry for Empty {
        fn source(&self) -> Option<&str> {
            None
        }
        fn target(&self) -> Option<&str> {
            None
        }
    }
    assert_eq!(
        ReportLabels::default().selected(&Empty).sentence,
        "Translation",
        "missing source languages no longer keep the default labels"
    );
}
