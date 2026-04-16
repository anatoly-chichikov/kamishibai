use anyhow::{Result, bail};

use super::{
    AudioProfile, DEFAULT_FONT, DeckNaming, FALLBACK_OCR, FontProfile, ImageProfile,
    LanguageProfile, UiLabels,
};

/// Registry for supported language profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    /// Return the supported profile for one language code.
    pub fn item(&self, code: &str) -> Result<LanguageProfile> {
        match code {
            "de" => Ok(LanguageProfile::new(
                "de",
                AudioProfile::new("German", "audio-de"),
                ImageProfile::new("eng+deu", "manga-de"),
                DeckNaming::new("German Vocabulary", "de", super::DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
            )),
            "el" => Ok(LanguageProfile::new(
                "el",
                AudioProfile::new("Greek", "audio-el"),
                ImageProfile::new("eng+ell", "manga-el"),
                DeckNaming::new("Greek Vocabulary", "el", super::DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
            )),
            "en" => Ok(LanguageProfile::new(
                "en",
                AudioProfile::new("English", "audio-en"),
                ImageProfile::new("eng", "manga-en"),
                DeckNaming::new("English Vocabulary", "en", super::DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Translation", "Context", "Hint", "Importance"),
            )),
            "es" => Ok(LanguageProfile::new(
                "es",
                AudioProfile::new("Spanish", "audio-es"),
                ImageProfile::new("eng+spa", "manga-es"),
                DeckNaming::new("Spanish Vocabulary", "es", super::DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
            )),
            "ru" => Ok(LanguageProfile::new(
                "ru",
                AudioProfile::new("Russian", "audio-ru"),
                ImageProfile::new("eng+rus", "manga-ru"),
                DeckNaming::new("Russian Vocabulary", "ru", super::DEFAULT_FILE),
                FontProfile::new(DEFAULT_FONT),
                UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
            )),
            "zh" => Ok(LanguageProfile::new(
                "zh",
                AudioProfile::new("Mandarin Chinese", "audio-zh"),
                ImageProfile::new("eng+chi_sim", "manga-zh"),
                DeckNaming::new("Chinese Vocabulary", "zh", super::DEFAULT_FILE),
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

/// Return the supported language catalog.
pub fn catalog() -> LanguageCatalog {
    LanguageCatalog
}

/// Return one supported language profile.
pub fn language(code: &str) -> Result<LanguageProfile> {
    catalog().item(code)
}
