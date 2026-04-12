//! Language profiles, naming, labels, and font selection.

use std::collections::BTreeSet;

use anyhow::{Result, bail};

use crate::input::NormalizedEntry;

const DEFAULT_FONT: &str = "DejaVu Sans";
const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

/// Expose language codes from normalized entries.
pub trait LanguageEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str>;
    /// Return the optional target language code.
    fn target(&self) -> Option<&str>;
}

impl LanguageEntry for NormalizedEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str> {
        Some(self.source_lang.as_str())
    }

    /// Return the optional target language code.
    fn target(&self) -> Option<&str> {
        Some(self.target_lang.as_str())
    }
}

/// Audio generation settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioProfile {
    language: String,
    cache: String,
}

impl AudioProfile {
    /// Create one audio profile.
    pub fn new(language: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            cache: cache.into(),
        }
    }

    /// Return the prompt language.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Return the audio cache directory name.
    pub fn cache(&self) -> &str {
        &self.cache
    }
}

/// Image generation settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProfile {
    ocr: String,
    cache: String,
}

impl ImageProfile {
    /// Create one image profile.
    pub fn new(ocr: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            ocr: ocr.into(),
            cache: cache.into(),
        }
    }

    /// Return the OCR language string.
    pub fn ocr(&self) -> &str {
        &self.ocr
    }

    /// Return the illustration cache directory name.
    pub fn cache(&self) -> &str {
        &self.cache
    }
}

/// Deck naming settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckNaming {
    name: String,
    prefix: String,
    default: String,
}

impl DeckNaming {
    /// Create one deck naming profile.
    pub fn new(
        name: impl Into<String>,
        prefix: impl Into<String>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prefix: prefix.into(),
            default: default.into(),
        }
    }

    /// Return the deck name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the filesystem-safe deck prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Return the default input filename.
    pub fn default(&self) -> &str {
        &self.default
    }
}

/// Report font settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontProfile {
    report: String,
}

impl FontProfile {
    /// Create one font profile.
    pub fn new(report: impl Into<String>) -> Self {
        Self {
            report: report.into(),
        }
    }

    /// Return the report font family name.
    pub fn report(&self) -> &str {
        &self.report
    }
}

/// User-facing labels for reports and related UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiLabels {
    sentence: String,
    context: String,
    hint: String,
    importance: String,
}

impl UiLabels {
    /// Create one label set.
    pub fn new(
        sentence: impl Into<String>,
        context: impl Into<String>,
        hint: impl Into<String>,
        importance: impl Into<String>,
    ) -> Self {
        Self {
            sentence: sentence.into(),
            context: context.into(),
            hint: hint.into(),
            importance: importance.into(),
        }
    }

    /// Return the sentence label.
    pub fn sentence(&self) -> &str {
        &self.sentence
    }

    /// Return the context label.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Return the hint label.
    pub fn hint(&self) -> &str {
        &self.hint
    }

    /// Return the importance label.
    pub fn importance(&self) -> &str {
        &self.importance
    }
}

/// One language profile composed from runtime and UI settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageProfile {
    code: String,
    audio: AudioProfile,
    imagery: ImageProfile,
    naming: DeckNaming,
    font: FontProfile,
    labels: UiLabels,
}

impl LanguageProfile {
    /// Create one language profile.
    pub fn new(
        code: impl Into<String>,
        audio: AudioProfile,
        imagery: ImageProfile,
        naming: DeckNaming,
        font: FontProfile,
        labels: UiLabels,
    ) -> Self {
        Self {
            code: code.into(),
            audio,
            imagery,
            naming,
            font,
            labels,
        }
    }

    /// Return the language code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Return the audio profile.
    pub fn audio(&self) -> &AudioProfile {
        &self.audio
    }

    /// Return the image profile.
    pub fn imagery(&self) -> &ImageProfile {
        &self.imagery
    }

    /// Return the naming profile.
    pub fn naming(&self) -> &DeckNaming {
        &self.naming
    }

    /// Return the font profile.
    pub fn font(&self) -> &FontProfile {
        &self.font
    }

    /// Return the UI labels.
    pub fn labels(&self) -> &UiLabels {
        &self.labels
    }
}

/// Registry for supported language profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProfileRegistry;

impl ProfileRegistry {
    /// Return the supported profile for one language code.
    pub fn item(&self, code: &str) -> Result<LanguageProfile> {
        match code {
            "de" => Ok(LanguageProfile::new(
                "de",
                AudioProfile::new("German", "audio-de"),
                ImageProfile::new("eng+deu", "manga-de"),
                DeckNaming::new("German Vocabulary", "de", DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
            )),
            "el" => Ok(LanguageProfile::new(
                "el",
                AudioProfile::new("Greek", "audio-el"),
                ImageProfile::new("eng+ell", "manga-el"),
                DeckNaming::new("Greek Vocabulary", "el", DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
            )),
            "en" => Ok(LanguageProfile::new(
                "en",
                AudioProfile::new("English", "audio-en"),
                ImageProfile::new("eng", "manga-en"),
                DeckNaming::new("English Vocabulary", "en", DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Translation", "Context", "Hint", "Importance"),
            )),
            "es" => Ok(LanguageProfile::new(
                "es",
                AudioProfile::new("Spanish", "audio-es"),
                ImageProfile::new("eng+spa", "manga-es"),
                DeckNaming::new("Spanish Vocabulary", "es", DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
            )),
            "ru" => Ok(LanguageProfile::new(
                "ru",
                AudioProfile::new("Russian", "audio-ru"),
                ImageProfile::new("eng+rus", "manga-ru"),
                DeckNaming::new("Russian Vocabulary", "ru", DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
            )),
            "zh" => Ok(LanguageProfile::new(
                "zh",
                AudioProfile::new("Mandarin Chinese", "audio-zh"),
                ImageProfile::new("eng+chi_sim", "manga-zh"),
                DeckNaming::new("Chinese Vocabulary", "zh", DEFAULT_FILE),
                FontProfile::new("Hiragino Sans GB"),
                UiLabels::new("翻译", "语境", "提示", "重要性"),
            )),
            _ => bail!("Unsupported language '{code}'"),
        }
    }

    /// Return the supported language codes in stable order.
    pub fn codes(&self) -> [&'static str; 6] {
        ["de", "el", "en", "es", "ru", "zh"]
    }

    /// Return the fallback OCR language string.
    pub fn fallback_ocr(&self) -> &'static str {
        FALLBACK_OCR
    }
}

/// Return the supported profile registry.
pub fn profiles() -> ProfileRegistry {
    ProfileRegistry
}

/// Return one supported language profile.
pub fn profile(code: &str) -> Result<LanguageProfile> {
    profiles().item(code)
}

/// Return a filesystem-safe deck prefix.
pub fn prefix(name: &str) -> String {
    let mut value = String::new();
    for item in name.chars() {
        if item.is_ascii_alphanumeric() {
            value.push(item.to_ascii_lowercase());
        } else if !value.ends_with('-') && !value.is_empty() {
            value.push('-');
        }
    }
    let trimmed = value.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return String::from("deck");
    }
    trimmed
}

/// Return the effective deck naming after applying CLI overrides.
pub fn naming(custom: Option<&str>, entries: &[NormalizedEntry]) -> DeckNaming {
    if let Some(item) = custom {
        return DeckNaming::new(item, prefix(item), DEFAULT_FILE);
    }
    let codes = entries
        .iter()
        .map(|entry| entry.target_lang.clone())
        .collect::<BTreeSet<_>>();
    if codes.len() == 1 {
        return profile(
            codes
                .iter()
                .next()
                .expect("single target set must contain one code"),
        )
        .expect("supported target language must resolve")
        .naming()
        .clone();
    }
    DeckNaming::new(DEFAULT_DECK, DEFAULT_PREFIX, DEFAULT_FILE)
}

/// Font family name selected for one report entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFamily {
    name: String,
}

impl FontFamily {
    /// Create one font family handle.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Return the font family name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Select report fonts from the language profiles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fonts {
    default: String,
}

impl Default for Fonts {
    /// Return the default font selector.
    fn default() -> Self {
        Self {
            default: String::from(DEFAULT_FONT),
        }
    }
}

impl Fonts {
    /// Return the selected font family for one entry.
    pub fn selected<T>(&self, entry: &T) -> FontFamily
    where
        T: LanguageEntry,
    {
        let names = [entry.source(), entry.target()]
            .into_iter()
            .flatten()
            .filter_map(|code| {
                profile(code)
                    .ok()
                    .map(|item| String::from(item.font().report()))
            })
            .collect::<Vec<_>>();
        if let Some(item) = names
            .iter()
            .find(|name| name.as_str() != self.default.as_str())
        {
            return FontFamily::new(item.clone());
        }
        if let Some(item) = names.first() {
            return FontFamily::new(item.clone());
        }
        FontFamily::new(self.default.clone())
    }
}

/// Select user-facing labels from the source language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Labels {
    default: UiLabels,
}

impl Default for Labels {
    /// Return the default label selector.
    fn default() -> Self {
        Self {
            default: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        }
    }
}

impl Labels {
    /// Return the selected labels for one entry.
    pub fn selected<T>(&self, entry: &T) -> UiLabels
    where
        T: LanguageEntry,
    {
        let Some(code) = entry.source() else {
            return self.default.clone();
        };
        match profile(code) {
            Ok(item) => item.labels().clone(),
            Err(_) => self.default.clone(),
        }
    }
}
