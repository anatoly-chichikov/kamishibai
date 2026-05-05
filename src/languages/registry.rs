use anyhow::{Result, bail};

use super::{DeckNaming, FALLBACK_OCR, LanguageProfile, UiLabels};

/// Registry for supported language profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    /// Return the supported profile for one language code.
    pub fn item(&self, code: &str) -> Result<LanguageProfile> {
        match code {
            "de" => Ok(LanguageProfile {
                code: String::from("de"),
                prompt: String::from("German"),
                audio_cache: String::from("audio-de"),
                ocr: String::from("eng+deu"),
                image_cache: String::from("manga-de"),
                naming: DeckNaming::new("German Vocabulary", "de", super::DEFAULT_FILE),
                labels: UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
            }),
            "el" => Ok(LanguageProfile {
                code: String::from("el"),
                prompt: String::from("Greek"),
                audio_cache: String::from("audio-el"),
                ocr: String::from("eng+ell"),
                image_cache: String::from("manga-el"),
                naming: DeckNaming::new("Greek Vocabulary", "el", super::DEFAULT_FILE),
                labels: UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
            }),
            "en" => Ok(LanguageProfile {
                code: String::from("en"),
                prompt: String::from("English"),
                audio_cache: String::from("audio-en"),
                ocr: String::from("eng"),
                image_cache: String::from("manga-en"),
                naming: DeckNaming::new("English Vocabulary", "en", super::DEFAULT_FILE),
                labels: UiLabels::new("Translation", "Context", "Hint", "Importance"),
            }),
            "es" => Ok(LanguageProfile {
                code: String::from("es"),
                prompt: String::from("Spanish"),
                audio_cache: String::from("audio-es"),
                ocr: String::from("eng+spa"),
                image_cache: String::from("manga-es"),
                naming: DeckNaming::new("Spanish Vocabulary", "es", super::DEFAULT_FILE),
                labels: UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
            }),
            "ru" => Ok(LanguageProfile {
                code: String::from("ru"),
                prompt: String::from("Russian"),
                audio_cache: String::from("audio-ru"),
                ocr: String::from("eng+rus"),
                image_cache: String::from("manga-ru"),
                naming: DeckNaming::new("Russian Vocabulary", "ru", super::DEFAULT_FILE),
                labels: UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
            }),
            "zh" => Ok(LanguageProfile {
                code: String::from("zh"),
                prompt: String::from("Mandarin Chinese"),
                audio_cache: String::from("audio-zh"),
                ocr: String::from("eng+chi_sim"),
                image_cache: String::from("manga-zh"),
                naming: DeckNaming::new("Chinese Vocabulary", "zh", super::DEFAULT_FILE),
                labels: UiLabels::new("翻译", "语境", "提示", "重要性"),
            }),
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
