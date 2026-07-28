use std::fmt::{Display, Formatter, Result as FmtResult};

use anyhow::{Result, anyhow};

use super::prompt_examples::{LanguagePromptExamples, examples};
use super::{DeckNaming, FALLBACK_OCR, LanguageProfile, UiLabels};

/// One supported language code in the canonical uppercase form.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LanguageCode(String);

impl AsRef<str> for LanguageCode {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for LanguageCode {
    /// Write the canonical language code.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.0.as_str())
    }
}

/// Registry for supported language profiles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    /// Return the supported profile for one language code.
    ///
    /// The lookup is case-insensitive: codes are canonically UPPERCASE across the
    /// app (config, session ids, cache layout, deck names, plain and JSON output),
    /// while the catalog stores the lowercase ISO code, so `item("FR")` and
    /// `item("fr")` both resolve the French profile.
    pub fn item(&self, code: &str) -> Result<LanguageProfile> {
        profiles()
            .into_iter()
            .find(|profile| profile.code.eq_ignore_ascii_case(code))
            .ok_or_else(|| anyhow!("Unsupported language '{code}'"))
    }

    /// Resolve one accepted spelling into a supported canonical code.
    pub fn resolve(&self, code: &str) -> Result<LanguageCode> {
        Ok(LanguageCode(self.item(code)?.code.to_uppercase()))
    }

    /// Return the supported language codes in stable order.
    pub fn codes(&self) -> [&'static str; 11] {
        profile_codes()
    }

    /// Return the fallback OCR language string.
    pub fn fallback_ocr(&self) -> &'static str {
        FALLBACK_OCR
    }

    /// Return the typed prompt examples for one supported language code.
    pub(crate) fn prompts(&self, code: &str) -> Result<LanguagePromptExamples> {
        let profile = self.item(code)?;
        Ok(examples(profile.code))
    }

    /// Identify a profile from either its supported code or prompt display name.
    pub(crate) fn identify(&self, value: &str) -> Result<LanguageProfile> {
        self.item(value).or_else(|_| {
            profiles()
                .into_iter()
                .find(|profile| profile.prompt.eq_ignore_ascii_case(value))
                .ok_or_else(|| anyhow!("Unsupported language '{value}'"))
        })
    }
}

/// Return codes from the canonical profile declarations.
pub(super) fn profile_codes() -> [&'static str; 11] {
    profiles().map(|profile| profile.code)
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
fn profiles() -> [LanguageProfile; 11] {
    [
        LanguageProfile {
            code: "en",
            prompt: String::from("English"),
            ocr: String::from("eng"),
            naming: DeckNaming::new("English Vocabulary", "en", super::DEFAULT_FILE),
            labels: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        },
        LanguageProfile {
            code: "zh",
            prompt: String::from("Mandarin Chinese"),
            ocr: String::from("eng+chi_sim"),
            naming: DeckNaming::new("Chinese Vocabulary", "zh", super::DEFAULT_FILE),
            labels: UiLabels::new("翻译", "语境", "提示", "重要性"),
        },
        LanguageProfile {
            code: "es",
            prompt: String::from("Spanish"),
            ocr: String::from("eng+spa"),
            naming: DeckNaming::new("Spanish Vocabulary", "es", super::DEFAULT_FILE),
            labels: UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
        },
        LanguageProfile {
            code: "ja",
            prompt: String::from("Japanese"),
            ocr: String::from("eng+jpn"),
            naming: DeckNaming::new("Japanese Vocabulary", "ja", super::DEFAULT_FILE),
            labels: UiLabels::new("翻訳", "文脈", "ヒント", "重要度"),
        },
        LanguageProfile {
            code: "fr",
            prompt: String::from("French"),
            ocr: String::from("eng+fra"),
            naming: DeckNaming::new("French Vocabulary", "fr", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduction", "Contexte", "Indice", "Importance"),
        },
        LanguageProfile {
            code: "de",
            prompt: String::from("German"),
            ocr: String::from("eng+deu"),
            naming: DeckNaming::new("German Vocabulary", "de", super::DEFAULT_FILE),
            labels: UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
        },
        LanguageProfile {
            code: "ru",
            prompt: String::from("Russian"),
            ocr: String::from("eng+rus"),
            naming: DeckNaming::new("Russian Vocabulary", "ru", super::DEFAULT_FILE),
            labels: UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
        },
        LanguageProfile {
            code: "it",
            prompt: String::from("Italian"),
            ocr: String::from("eng+ita"),
            naming: DeckNaming::new("Italian Vocabulary", "it", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduzione", "Contesto", "Suggerimento", "Importanza"),
        },
        LanguageProfile {
            code: "pt",
            prompt: String::from("Portuguese"),
            ocr: String::from("eng+por"),
            naming: DeckNaming::new("Portuguese Vocabulary", "pt", super::DEFAULT_FILE),
            labels: UiLabels::new("Tradução", "Contexto", "Dica", "Importância"),
        },
        LanguageProfile {
            code: "el",
            prompt: String::from("Greek"),
            ocr: String::from("eng+ell"),
            naming: DeckNaming::new("Greek Vocabulary", "el", super::DEFAULT_FILE),
            labels: UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
        },
        LanguageProfile {
            code: "nl",
            prompt: String::from("Dutch"),
            ocr: String::from("eng+nld"),
            naming: DeckNaming::new("Dutch Vocabulary", "nl", super::DEFAULT_FILE),
            labels: UiLabels::new("Vertaling", "Context", "Hint", "Belang"),
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

    #[test]
    fn resolve_normalises_a_supported_code_to_uppercase() {
        assert_eq!(
            catalog()
                .resolve("fR")
                .expect("French must resolve")
                .as_ref(),
            "FR",
            "the language catalog returned a non-canonical supported code"
        );
    }
}
