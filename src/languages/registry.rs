use anyhow::{Result, anyhow};

use super::{DeckNaming, FALLBACK_OCR, LanguageProfile, UiLabels};

/// Registry for supported language profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    /// Return the supported profile for one language code.
    pub fn item(&self, code: &str) -> Result<LanguageProfile> {
        profiles()
            .into_iter()
            .find(|profile| profile.code == code)
            .ok_or_else(|| anyhow!("Unsupported language '{code}'"))
    }

    /// Return the supported language codes in stable order.
    pub fn codes(&self) -> [&'static str; 10] {
        profiles().map(|profile| profile.code)
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

/// Build the single canonical list of supported profiles. The order here is
/// the order surfaced everywhere in the UI (Welcome chips, `Cmd+L` picker,
/// Gemini language list) and is sorted by global learning popularity, not
/// alphabetically.
fn profiles() -> [LanguageProfile; 10] {
    [
        LanguageProfile {
            code: "en",
            prompt: String::from("English"),
            audio_cache: String::from("audio-en"),
            ocr: String::from("eng"),
            image_cache: String::from("manga-en"),
            naming: DeckNaming::new("English Vocabulary", "en", super::DEFAULT_FILE),
            labels: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        },
        LanguageProfile {
            code: "zh",
            prompt: String::from("Mandarin Chinese"),
            audio_cache: String::from("audio-zh"),
            ocr: String::from("eng+chi_sim"),
            image_cache: String::from("manga-zh"),
            naming: DeckNaming::new("Chinese Vocabulary", "zh", super::DEFAULT_FILE),
            labels: UiLabels::new("翻译", "语境", "提示", "重要性"),
        },
        LanguageProfile {
            code: "es",
            prompt: String::from("Spanish"),
            audio_cache: String::from("audio-es"),
            ocr: String::from("eng+spa"),
            image_cache: String::from("manga-es"),
            naming: DeckNaming::new("Spanish Vocabulary", "es", super::DEFAULT_FILE),
            labels: UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
        },
        LanguageProfile {
            code: "ja",
            prompt: String::from("Japanese"),
            audio_cache: String::from("audio-ja"),
            ocr: String::from("eng+jpn"),
            image_cache: String::from("manga-ja"),
            naming: DeckNaming::new("Japanese Vocabulary", "ja", super::DEFAULT_FILE),
            labels: UiLabels::new("翻訳", "文脈", "ヒント", "重要度"),
        },
        LanguageProfile {
            code: "fr",
            prompt: String::from("French"),
            audio_cache: String::from("audio-fr"),
            ocr: String::from("eng+fra"),
            image_cache: String::from("manga-fr"),
            naming: DeckNaming::new("French Vocabulary", "fr", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduction", "Contexte", "Indice", "Importance"),
        },
        LanguageProfile {
            code: "de",
            prompt: String::from("German"),
            audio_cache: String::from("audio-de"),
            ocr: String::from("eng+deu"),
            image_cache: String::from("manga-de"),
            naming: DeckNaming::new("German Vocabulary", "de", super::DEFAULT_FILE),
            labels: UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
        },
        LanguageProfile {
            code: "ru",
            prompt: String::from("Russian"),
            audio_cache: String::from("audio-ru"),
            ocr: String::from("eng+rus"),
            image_cache: String::from("manga-ru"),
            naming: DeckNaming::new("Russian Vocabulary", "ru", super::DEFAULT_FILE),
            labels: UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
        },
        LanguageProfile {
            code: "it",
            prompt: String::from("Italian"),
            audio_cache: String::from("audio-it"),
            ocr: String::from("eng+ita"),
            image_cache: String::from("manga-it"),
            naming: DeckNaming::new("Italian Vocabulary", "it", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduzione", "Contesto", "Suggerimento", "Importanza"),
        },
        LanguageProfile {
            code: "pt",
            prompt: String::from("Portuguese"),
            audio_cache: String::from("audio-pt"),
            ocr: String::from("eng+por"),
            image_cache: String::from("manga-pt"),
            naming: DeckNaming::new("Portuguese Vocabulary", "pt", super::DEFAULT_FILE),
            labels: UiLabels::new("Tradução", "Contexto", "Dica", "Importância"),
        },
        LanguageProfile {
            code: "el",
            prompt: String::from("Greek"),
            audio_cache: String::from("audio-el"),
            ocr: String::from("eng+ell"),
            image_cache: String::from("manga-el"),
            naming: DeckNaming::new("Greek Vocabulary", "el", super::DEFAULT_FILE),
            labels: UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catalog should not declare phantom codes via `codes()` that `item()`
    /// cannot resolve, nor should it stash profiles unreachable from
    /// `codes()`.
    #[test]
    fn codes_and_item_walk_the_same_set() {
        let codes = catalog().codes();
        for code in codes {
            catalog()
                .item(code)
                .expect("code should resolve to a profile");
        }
    }
}
