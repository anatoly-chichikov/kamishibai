//! Typed language-local examples injected into Gemini prompt templates.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::registry::profile_codes;

const DOCUMENT: &str = include_str!("../../assets/prompt_examples.json");
static EXAMPLES: OnceLock<BTreeMap<String, LanguagePromptExamples>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
/// One language's support-writing and hidden-target prompt examples.
pub(crate) struct LanguagePromptExamples {
    spacing: TextSpacing,
    understanding: UnderstandingExamples,
    card: CardExamples,
    recall: RecallExamples,
}

impl LanguagePromptExamples {
    /// Render localized pairs demonstrating natural learner-facing wording.
    pub(crate) fn writing(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.card.writing)?)
    }

    /// Render localized sense markers and forbidden wording as typed JSON.
    pub(crate) fn sense_conventions(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&SenseConventions {
            markers: &self.understanding.markers,
            forbidden: &self.understanding.forbidden,
        })?)
    }

    /// Render target-neutral intake examples in the support language.
    pub(crate) fn intake(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.understanding.intake)?)
    }

    /// Render localized empty-result messages for sense correction.
    pub(crate) fn sense_messages(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.understanding.messages)?)
    }

    /// Render the support language's understanding length contract.
    pub(crate) fn understanding_length(&self) -> &'static str {
        self.spacing.understanding()
    }

    /// Render localized bad and good hint examples.
    pub(crate) fn hint(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.card.hint)?)
    }

    /// Render the support language's hint length contract.
    pub(crate) fn hint_length(&self) -> &'static str {
        self.spacing.hint()
    }

    /// Render localized bold context headers and rarity wording.
    pub(crate) fn context(&self) -> Result<String> {
        let headers = self
            .card
            .context
            .headers
            .each_ref()
            .map(|header| format!("**{header}**"));
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "headers": headers,
            "rarity": self.card.context.rarity,
        }))?)
    }

    /// Render the target-language focus-in-longer-word example.
    pub(crate) fn recall_focus(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.recall.focus)?)
    }

    /// Render the target-language hidden-sentence fragment example.
    pub(crate) fn recall_fragment(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.recall.fragment)?)
    }

    fn valid(&self) -> bool {
        let intake = [
            &self.understanding.intake.primary,
            &self.understanding.intake.secondary,
            &self.understanding.intake.hinted,
        ];
        let tags = intake.iter().all(|example| {
            example.tag.as_ref().is_none_or(|tag| {
                self.understanding
                    .markers
                    .tags
                    .iter()
                    .any(|marker| marker == tag)
            })
        });
        let lengths = intake
            .iter()
            .all(|example| self.spacing.accepts_understanding(&example.understanding));
        let structural = intake.iter().all(|example| {
            example.understanding.matches('<').count() == 1
                && example.understanding.matches('>').count() == 1
        });
        let focus = self.recall.focus.focus.to_lowercase();
        let longer = self.recall.focus.longer.to_lowercase();
        let fragment = self.recall.fragment.visible.to_uppercase()
            == self.recall.fragment.sentence_end.to_uppercase();
        tags && lengths
            && structural
            && self.card.writing.iter().all(WritingExample::valid)
            && nonempty(self.understanding.markers.tags.iter())
            && nonempty(self.understanding.markers.parts.iter())
            && nonempty(self.understanding.forbidden.starters.iter())
            && nonempty(self.understanding.forbidden.joins.iter())
            && nonempty(self.understanding.forbidden.fillers.iter())
            && nonempty(self.card.hint.bad.iter())
            && !self.card.hint.good.trim().is_empty()
            && self.spacing.accepts_hint(&self.card.hint.good)
            && self
                .card
                .hint
                .contrast
                .matches("<near target word>")
                .count()
                == 1
            && self
                .card
                .context
                .headers
                .iter()
                .all(|header| !header.is_empty())
            && !self.card.context.rarity.is_empty()
            && !self.understanding.intake.invalid.is_empty()
            && !self.understanding.messages.listed.is_empty()
            && !self.understanding.messages.absent.is_empty()
            && !focus.is_empty()
            && focus != longer
            && longer.contains(&focus)
            && !self.recall.fragment.visible.is_empty()
            && !self.recall.fragment.sentence_end.is_empty()
            && fragment
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TextSpacing {
    Words,
    Unspaced,
}

impl TextSpacing {
    fn understanding(self) -> &'static str {
        match self {
            Self::Words => {
                "at most 14 space-delimited words, with no minimum; never pad a complete meaning"
            }
            Self::Unspaced => {
                "one natural sentence of comparable brevity without artificial spaces"
            }
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Words => "hard limit 11 space-delimited words",
            Self::Unspaced => {
                "one natural sentence of comparable brevity without artificial spaces"
            }
        }
    }

    fn accepts_understanding(self, value: &str) -> bool {
        match self {
            Self::Words => (1..=14).contains(&value.split_whitespace().count()),
            Self::Unspaced => !value.contains(' '),
        }
    }

    fn accepts_hint(self, value: &str) -> bool {
        let sample = value.replace("<near target word>", "sample");
        match self {
            Self::Words => sample.split_whitespace().count() <= 11,
            Self::Unspaced => !sample.contains(' '),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct UnderstandingExamples {
    markers: SenseMarkers,
    forbidden: ForbiddenExamples,
    intake: IntakeExamples,
    messages: SenseMessages,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SenseMarkers {
    tags: Vec<String>,
    parts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ForbiddenExamples {
    starters: Vec<String>,
    joins: Vec<String>,
    fillers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct IntakeExamples {
    primary: SenseExample,
    secondary: SenseExample,
    hinted: SenseExample,
    invalid: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SenseExample {
    understanding: String,
    tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SenseMessages {
    listed: String,
    absent: String,
}

#[derive(Serialize)]
struct SenseConventions<'a> {
    markers: &'a SenseMarkers,
    forbidden: &'a ForbiddenExamples,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CardExamples {
    hint: HintExamples,
    context: ContextExamples,
    writing: [WritingExample; 2],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct WritingExample {
    bad: String,
    good: String,
}

impl WritingExample {
    #[must_use]
    fn valid(&self) -> bool {
        !self.bad.trim().is_empty()
            && !self.good.trim().is_empty()
            && self.bad.trim() != self.good.trim()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HintExamples {
    bad: [String; 3],
    good: String,
    contrast: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextExamples {
    headers: [String; 4],
    rarity: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RecallExamples {
    focus: FocusExample,
    fragment: FragmentExample,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FocusExample {
    focus: String,
    longer: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FragmentExample {
    visible: String,
    sentence_end: String,
}

/// Resolve typed prompt examples for one supported language code.
pub(super) fn examples(code: &str) -> LanguagePromptExamples {
    catalog()
        .get(&code.to_ascii_lowercase())
        .cloned()
        .expect("invariant: every supported language must define prompt examples")
}

/// Render only target recall examples for visual policy hashing.
pub(crate) fn recall_document() -> String {
    let recalls = catalog()
        .iter()
        .map(|(code, examples)| (code.as_str(), &examples.recall))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&recalls)
        .expect("invariant: typed recall examples must serialize for policy hashing")
}

/// Recognize the meaning heading shared by supported card explanation formats.
pub(crate) fn is_meaning_header(header: &str) -> bool {
    catalog()
        .values()
        .any(|examples| examples.card.context.headers[0] == header)
}

fn catalog() -> &'static BTreeMap<String, LanguagePromptExamples> {
    EXAMPLES.get_or_init(|| {
        let values = serde_json::from_str::<BTreeMap<String, LanguagePromptExamples>>(DOCUMENT)
            .expect("invariant: embedded prompt examples must be valid JSON");
        let codes = values.keys().map(String::as_str).collect::<Vec<_>>();
        let mut expected = profile_codes().to_vec();
        expected.sort_unstable();
        assert_eq!(
            codes, expected,
            "invariant: prompt examples must exactly cover supported languages"
        );
        if let Some((code, _examples)) = values.iter().find(|(_code, examples)| !examples.valid()) {
            panic!("invariant: prompt examples for '{code}' violate their typed language policy");
        }
        values
    })
}

fn nonempty<'a>(mut values: impl Iterator<Item = &'a String>) -> bool {
    let mut seen = false;
    let filled = values.all(|value| {
        seen = true;
        !value.is_empty()
    });
    seen && filled
}

#[cfg(test)]
mod tests {
    use super::{TextSpacing, examples};
    use crate::languages::catalog;

    #[test]
    fn context_headers_keep_the_original_appropriateness_and_nuance_roles() {
        let expected = [
            (
                "en",
                [
                    "Meaning.",
                    "Where you'll hear it.",
                    "Where it's out of place.",
                    "Subtlety.",
                ],
            ),
            ("zh", ["含义", "常见场景", "不适用的场合", "细微差别"]),
            (
                "es",
                [
                    "Significado.",
                    "Dónde lo oirás.",
                    "Dónde desentona.",
                    "Matiz.",
                ],
            ),
            (
                "ja",
                ["意味", "どこで耳にするか", "不自然な場面", "ニュアンス"],
            ),
            (
                "fr",
                ["Sens.", "Où vous l’entendrez.", "Où il détonne.", "Nuance."],
            ),
            (
                "de",
                [
                    "Bedeutung.",
                    "Wo du es hörst.",
                    "Wo es unpassend ist.",
                    "Nuance.",
                ],
            ),
            (
                "ru",
                ["Значение.", "Где встречается.", "Где неуместно.", "Нюанс."],
            ),
            (
                "it",
                [
                    "Significato.",
                    "Dove lo sentirai.",
                    "Dove stona.",
                    "Sfumatura.",
                ],
            ),
            (
                "pt",
                [
                    "Significado.",
                    "Onde você ouvirá.",
                    "Onde não combina.",
                    "Nuance.",
                ],
            ),
            (
                "el",
                [
                    "Σημασία.",
                    "Πού θα το ακούσεις.",
                    "Πού δεν ταιριάζει.",
                    "Απόχρωση.",
                ],
            ),
            (
                "nl",
                [
                    "Betekenis.",
                    "Waar je het hoort.",
                    "Waar het niet past.",
                    "Nuance.",
                ],
            ),
            (
                "ko",
                [
                    "뜻.",
                    "자연스럽게 쓰는 상황.",
                    "어색해지는 상황.",
                    "말맛과 뉘앙스.",
                ],
            ),
            (
                "tr",
                [
                    "Anlamı.",
                    "Doğal kullanıldığı yer.",
                    "Yadırganacağı yer.",
                    "Söyleyiş inceliği.",
                ],
            ),
            (
                "pl",
                [
                    "Znaczenie.",
                    "Naturalny kontekst.",
                    "Kontekst, w którym razi.",
                    "Odcień znaczeniowy.",
                ],
            ),
            (
                "uk",
                [
                    "Значення.",
                    "Природний контекст.",
                    "Недоречний контекст.",
                    "Смисловий відтінок.",
                ],
            ),
            (
                "id",
                [
                    "Makna.",
                    "Konteks yang wajar.",
                    "Konteks yang terasa janggal.",
                    "Nuansa pemakaian.",
                ],
            ),
            (
                "hi",
                [
                    "अर्थ।",
                    "स्वाभाविक प्रयोग।",
                    "जहाँ प्रयोग अटपटा लगे।",
                    "भाव और लहजा।",
                ],
            ),
            (
                "ar",
                [
                    "المعنى.",
                    "موضع الاستعمال الطبيعي.",
                    "موضع لا يلائمه.",
                    "الإيحاء والأسلوب.",
                ],
            ),
            (
                "th",
                [
                    "ความหมาย",
                    "บริบทที่ใช้ได้เป็นธรรมชาติ",
                    "บริบทที่ฟังดูไม่เข้าที่",
                    "น้ำเสียงและนัย",
                ],
            ),
            (
                "he",
                ["משמעות.", "הקשר טבעי.", "הקשר שבו זה צורם.", "גוון ומשלב."],
            ),
            (
                "vi",
                [
                    "Nghĩa.",
                    "Ngữ cảnh dùng tự nhiên.",
                    "Ngữ cảnh nghe gượng.",
                    "Sắc thái và văn phong.",
                ],
            ),
            (
                "cs",
                [
                    "Význam.",
                    "Přirozený kontext.",
                    "Kontext, kde působí nepatřičně.",
                    "Významový odstín.",
                ],
            ),
        ];
        let actual = expected
            .iter()
            .map(|(code, _)| {
                let value: serde_json::Value = serde_json::from_str(
                    &examples(code)
                        .context()
                        .expect("context examples must render"),
                )
                .expect("context examples must be JSON");
                (*code, value["headers"].clone())
            })
            .collect::<Vec<_>>();
        let rendered = expected
            .iter()
            .map(|(code, headers)| {
                (
                    *code,
                    serde_json::json!(headers.map(|header| format!("**{header}**"))),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            (actual, expected.len()),
            (rendered, catalog().codes().len()),
            "context headers lost their original localized appropriateness or nuance role"
        );
    }

    #[test]
    fn short_accurate_understandings_dont_require_padding() {
        assert!(
            TextSpacing::Words.accepts_understanding("Гл. Двигаться по воздуху."),
            "a concise meaning still needs incidental detail just to reach a word minimum"
        );
    }

    #[test]
    fn writing_examples_cannot_have_an_empty_bad_side() {
        let mut sample = examples("ru");
        sample.card.writing[0].bad = String::from(" \t");
        assert!(
            !sample.valid(),
            "a writing example accepted an empty bad side"
        );
    }

    #[test]
    fn writing_examples_cannot_have_an_empty_good_side() {
        let mut sample = examples("ja");
        sample.card.writing[1].good = String::from(" \t");
        assert!(
            !sample.valid(),
            "a writing example accepted an empty good side"
        );
    }

    #[test]
    fn writing_examples_cannot_treat_padding_as_a_wording_improvement() {
        let mut sample = examples("en");
        sample.card.writing[0].bad = format!(" {} ", sample.card.writing[0].good);
        assert!(
            !sample.valid(),
            "a writing example treated whitespace as a wording improvement"
        );
    }

    #[test]
    fn intake_examples_dont_require_grammar_labels() {
        let mut sample = examples("ru");
        sample.understanding.intake.primary.understanding =
            String::from("<одно понятное основное значение>");
        sample.understanding.intake.secondary.understanding =
            String::from("<другое отдельное значение>");
        sample.understanding.intake.hinted.understanding =
            String::from("<значение из подсказки пользователя>");
        assert!(
            sample.valid(),
            "plain intake examples still require grammar labels before their meanings"
        );
    }

    #[test]
    fn every_supported_language_has_typed_prompt_examples() {
        assert!(
            catalog()
                .codes()
                .into_iter()
                .all(|code| examples(code).sense_conventions().is_ok()),
            "a supported language lost its typed prompt examples"
        );
    }
}
