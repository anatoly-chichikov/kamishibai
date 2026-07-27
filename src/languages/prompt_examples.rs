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
        let parts = intake.iter().all(|example| {
            self.understanding
                .markers
                .parts
                .iter()
                .any(|part| starts_with_case_folded(&example.understanding, part))
        });
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
        parts
            && tags
            && lengths
            && structural
            && nonempty(self.understanding.markers.tags.iter())
            && nonempty(self.understanding.markers.parts.iter())
            && nonempty(self.understanding.forbidden.starters.iter())
            && nonempty(self.understanding.forbidden.joins.iter())
            && nonempty(self.understanding.forbidden.fillers.iter())
            && nonempty(self.card.hint.bad.iter())
            && self.card.hint.good.matches("<near target word>").count() == 1
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
            Self::Words => "between 6 and 14 space-delimited words",
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
            Self::Words => (6..=14).contains(&value.split_whitespace().count()),
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

fn starts_with_case_folded(value: &str, prefix: &str) -> bool {
    value.to_lowercase().starts_with(&prefix.to_lowercase())
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
    use super::examples;
    use crate::languages::catalog;

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
