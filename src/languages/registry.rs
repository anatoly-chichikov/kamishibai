use std::fmt::{Display, Formatter, Result as FmtResult};

use anyhow::{Result, anyhow};

use super::prompt_examples::{LanguagePromptExamples, examples};
use super::{
    DeckNaming, FALLBACK_OCR, LanguageProfile, OcrModel, TextDirection, TextGate, UiLabels,
};

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
    pub fn codes(&self) -> [&'static str; 21] {
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
pub(super) fn profile_codes() -> [&'static str; 21] {
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
fn profiles() -> [LanguageProfile; 21] {
    [
        LanguageProfile {
            code: "en",
            prompt: String::from("English"),
            text_gate: TextGate::Ocr(OcrModel::En),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("English Vocabulary", "en", super::DEFAULT_FILE),
            labels: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        },
        LanguageProfile {
            code: "zh",
            prompt: String::from("Mandarin Chinese"),
            text_gate: TextGate::Ocr(OcrModel::Default),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Chinese Vocabulary", "zh", super::DEFAULT_FILE),
            labels: UiLabels::new("翻译", "语境", "提示", "重要性"),
        },
        LanguageProfile {
            code: "es",
            prompt: String::from("Spanish"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Spanish Vocabulary", "es", super::DEFAULT_FILE),
            labels: UiLabels::new("Traducción", "Contexto", "Pista", "Importancia"),
        },
        LanguageProfile {
            code: "ja",
            prompt: String::from("Japanese"),
            text_gate: TextGate::Ocr(OcrModel::Default),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Japanese Vocabulary", "ja", super::DEFAULT_FILE),
            labels: UiLabels::new("翻訳", "文脈", "ヒント", "重要度"),
        },
        LanguageProfile {
            code: "fr",
            prompt: String::from("French"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("French Vocabulary", "fr", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduction", "Contexte", "Indice", "Importance"),
        },
        LanguageProfile {
            code: "de",
            prompt: String::from("German"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("German Vocabulary", "de", super::DEFAULT_FILE),
            labels: UiLabels::new("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
        },
        LanguageProfile {
            code: "ko",
            prompt: String::from("Korean"),
            text_gate: TextGate::Ocr(OcrModel::Korean),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Korean Vocabulary", "ko", super::DEFAULT_FILE),
            labels: UiLabels::new("번역", "문맥", "힌트", "중요도"),
        },
        LanguageProfile {
            code: "ru",
            prompt: String::from("Russian"),
            text_gate: TextGate::Ocr(OcrModel::Cyrillic),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Russian Vocabulary", "ru", super::DEFAULT_FILE),
            labels: UiLabels::new("Перевод", "Контекст", "Подсказка", "Важность"),
        },
        LanguageProfile {
            code: "it",
            prompt: String::from("Italian"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Italian Vocabulary", "it", super::DEFAULT_FILE),
            labels: UiLabels::new("Traduzione", "Contesto", "Suggerimento", "Importanza"),
        },
        LanguageProfile {
            code: "pt",
            prompt: String::from("Portuguese"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Portuguese Vocabulary", "pt", super::DEFAULT_FILE),
            labels: UiLabels::new("Tradução", "Contexto", "Dica", "Importância"),
        },
        LanguageProfile {
            code: "hi",
            prompt: String::from("Hindi"),
            text_gate: TextGate::Ocr(OcrModel::Devanagari),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Hindi Vocabulary", "hi", super::DEFAULT_FILE),
            labels: UiLabels::new("अनुवाद", "संदर्भ", "संकेत", "महत्त्व"),
        },
        LanguageProfile {
            code: "ar",
            prompt: String::from("Arabic"),
            text_gate: TextGate::Ocr(OcrModel::Arabic),
            direction: TextDirection::Rtl,
            naming: DeckNaming::new("Arabic Vocabulary", "ar", super::DEFAULT_FILE),
            labels: UiLabels::new("الترجمة", "السياق", "تلميح", "الأهمية"),
        },
        LanguageProfile {
            code: "tr",
            prompt: String::from("Turkish"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Turkish Vocabulary", "tr", super::DEFAULT_FILE),
            labels: UiLabels::new("Çeviri", "Bağlam", "İpucu", "Önem"),
        },
        LanguageProfile {
            code: "pl",
            prompt: String::from("Polish"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Polish Vocabulary", "pl", super::DEFAULT_FILE),
            labels: UiLabels::new("Tłumaczenie", "Kontekst", "Wskazówka", "Ważność"),
        },
        LanguageProfile {
            code: "uk",
            prompt: String::from("Ukrainian"),
            text_gate: TextGate::Ocr(OcrModel::Cyrillic),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Ukrainian Vocabulary", "uk", super::DEFAULT_FILE),
            labels: UiLabels::new("Переклад", "Контекст", "Підказка", "Важливість"),
        },
        LanguageProfile {
            code: "id",
            prompt: String::from("Indonesian"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Indonesian Vocabulary", "id", super::DEFAULT_FILE),
            labels: UiLabels::new("Terjemahan", "Konteks", "Petunjuk", "Tingkat Kepentingan"),
        },
        LanguageProfile {
            code: "vi",
            prompt: String::from("Vietnamese"),
            text_gate: TextGate::LlmJudge,
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Vietnamese Vocabulary", "vi", super::DEFAULT_FILE),
            labels: UiLabels::new("Bản dịch", "Ngữ cảnh", "Gợi ý", "Mức độ quan trọng"),
        },
        LanguageProfile {
            code: "th",
            prompt: String::from("Thai"),
            text_gate: TextGate::Ocr(OcrModel::Th),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Thai Vocabulary", "th", super::DEFAULT_FILE),
            labels: UiLabels::new("คำแปล", "บริบท", "คำใบ้", "ความสำคัญ"),
        },
        LanguageProfile {
            code: "el",
            prompt: String::from("Greek"),
            text_gate: TextGate::Ocr(OcrModel::El),
            direction: TextDirection::Ltr,
            naming: DeckNaming::new("Greek Vocabulary", "el", super::DEFAULT_FILE),
            labels: UiLabels::new("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
        },
        LanguageProfile {
            code: "he",
            prompt: String::from("Hebrew"),
            text_gate: TextGate::LlmJudge,
            direction: TextDirection::Rtl,
            naming: DeckNaming::new("Hebrew Vocabulary", "he", super::DEFAULT_FILE),
            labels: UiLabels::new("תרגום", "הקשר", "רמז", "חשיבות"),
        },
        LanguageProfile {
            code: "nl",
            prompt: String::from("Dutch"),
            text_gate: TextGate::Ocr(OcrModel::Latin),
            direction: TextDirection::Ltr,
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

    #[test]
    fn profiles_keep_the_requested_global_popularity_order() {
        assert_eq!(
            catalog().codes(),
            [
                "en", "zh", "es", "ja", "fr", "de", "ko", "ru", "it", "pt", "hi", "ar", "tr", "pl",
                "uk", "id", "vi", "th", "el", "he", "nl",
            ],
            "language profiles no longer follow the requested popularity order"
        );
    }

    #[test]
    fn profiles_declare_every_text_gate_and_direction() {
        let values = profiles().map(|profile| (profile.code, profile.text_gate, profile.direction));
        assert_eq!(
            values,
            [
                ("en", TextGate::Ocr(OcrModel::En), TextDirection::Ltr),
                ("zh", TextGate::Ocr(OcrModel::Default), TextDirection::Ltr),
                ("es", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("ja", TextGate::Ocr(OcrModel::Default), TextDirection::Ltr),
                ("fr", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("de", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("ko", TextGate::Ocr(OcrModel::Korean), TextDirection::Ltr),
                ("ru", TextGate::Ocr(OcrModel::Cyrillic), TextDirection::Ltr),
                ("it", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("pt", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                (
                    "hi",
                    TextGate::Ocr(OcrModel::Devanagari),
                    TextDirection::Ltr
                ),
                ("ar", TextGate::Ocr(OcrModel::Arabic), TextDirection::Rtl),
                ("tr", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("pl", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("uk", TextGate::Ocr(OcrModel::Cyrillic), TextDirection::Ltr),
                ("id", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
                ("vi", TextGate::LlmJudge, TextDirection::Ltr),
                ("th", TextGate::Ocr(OcrModel::Th), TextDirection::Ltr),
                ("el", TextGate::Ocr(OcrModel::El), TextDirection::Ltr),
                ("he", TextGate::LlmJudge, TextDirection::Rtl),
                ("nl", TextGate::Ocr(OcrModel::Latin), TextDirection::Ltr),
            ],
            "a language profile lost its authoritative gate or direction declaration"
        );
    }

    #[test]
    fn new_profiles_expose_complete_native_catalog_values() {
        let values = ["ko", "tr", "pl", "uk", "id", "hi", "ar", "th", "he", "vi"].map(|code| {
            let profile = catalog()
                .item(code)
                .expect("new language profile must resolve");
            (
                profile.code,
                profile.prompt,
                profile.naming.name,
                profile.naming.prefix,
                profile.labels.sentence,
                profile.labels.context,
                profile.labels.hint,
                profile.labels.importance,
            )
        });
        assert_eq!(
            values,
            [
                (
                    "ko",
                    "Korean".into(),
                    "Korean Vocabulary".into(),
                    "ko".into(),
                    "번역".into(),
                    "문맥".into(),
                    "힌트".into(),
                    "중요도".into()
                ),
                (
                    "tr",
                    "Turkish".into(),
                    "Turkish Vocabulary".into(),
                    "tr".into(),
                    "Çeviri".into(),
                    "Bağlam".into(),
                    "İpucu".into(),
                    "Önem".into()
                ),
                (
                    "pl",
                    "Polish".into(),
                    "Polish Vocabulary".into(),
                    "pl".into(),
                    "Tłumaczenie".into(),
                    "Kontekst".into(),
                    "Wskazówka".into(),
                    "Ważność".into()
                ),
                (
                    "uk",
                    "Ukrainian".into(),
                    "Ukrainian Vocabulary".into(),
                    "uk".into(),
                    "Переклад".into(),
                    "Контекст".into(),
                    "Підказка".into(),
                    "Важливість".into()
                ),
                (
                    "id",
                    "Indonesian".into(),
                    "Indonesian Vocabulary".into(),
                    "id".into(),
                    "Terjemahan".into(),
                    "Konteks".into(),
                    "Petunjuk".into(),
                    "Tingkat Kepentingan".into()
                ),
                (
                    "hi",
                    "Hindi".into(),
                    "Hindi Vocabulary".into(),
                    "hi".into(),
                    "अनुवाद".into(),
                    "संदर्भ".into(),
                    "संकेत".into(),
                    "महत्त्व".into()
                ),
                (
                    "ar",
                    "Arabic".into(),
                    "Arabic Vocabulary".into(),
                    "ar".into(),
                    "الترجمة".into(),
                    "السياق".into(),
                    "تلميح".into(),
                    "الأهمية".into()
                ),
                (
                    "th",
                    "Thai".into(),
                    "Thai Vocabulary".into(),
                    "th".into(),
                    "คำแปล".into(),
                    "บริบท".into(),
                    "คำใบ้".into(),
                    "ความสำคัญ".into()
                ),
                (
                    "he",
                    "Hebrew".into(),
                    "Hebrew Vocabulary".into(),
                    "he".into(),
                    "תרגום".into(),
                    "הקשר".into(),
                    "רמז".into(),
                    "חשיבות".into()
                ),
                (
                    "vi",
                    "Vietnamese".into(),
                    "Vietnamese Vocabulary".into(),
                    "vi".into(),
                    "Bản dịch".into(),
                    "Ngữ cảnh".into(),
                    "Gợi ý".into(),
                    "Mức độ quan trọng".into()
                ),
            ],
            "a new language profile lost its complete native catalog contract"
        );
    }
}
