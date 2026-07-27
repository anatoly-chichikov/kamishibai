//! Typed flashcard context and verdicts for image-based answer-leakage review.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::generation::prompts::{picture_recall_judge_prompt, picture_recall_judge_schema};
use crate::languages::{LanguageProfile, catalog};
use crate::prompt::PromptTemplate;

/// Source-language fields already visible on the front of one flashcard.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ShownRecall {
    shown_source_language: String,
    shown_source_sentence: String,
    shown_source_highlight: String,
    shown_hint: String,
}

impl ShownRecall {
    /// Create the source-language context visible before answer reveal.
    pub fn new(
        language: impl Into<String>,
        sentence: impl Into<String>,
        highlight: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            shown_source_language: language.into(),
            shown_source_sentence: sentence.into(),
            shown_source_highlight: highlight.into(),
            shown_hint: hint.into(),
        }
    }
}

/// Target-language fields hidden until the learner reveals the answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HiddenRecall {
    hidden_target_language: String,
    hidden_focus_term: String,
    hidden_target_sentence: String,
}

impl HiddenRecall {
    /// Create the target-language answer the learner must actively recall.
    pub fn new(
        language: impl Into<String>,
        term: impl Into<String>,
        sentence: impl Into<String>,
    ) -> Self {
        Self {
            hidden_target_language: language.into(),
            hidden_focus_term: term.into(),
            hidden_target_sentence: sentence.into(),
        }
    }

    /// Create one target-language recall answer from a resolved profile.
    pub(crate) fn from_profile(
        language: &LanguageProfile,
        term: impl Into<String>,
        sentence: impl Into<String>,
    ) -> Self {
        Self::new(language.prompt.clone(), term, sentence)
    }
}

/// Complete shown-versus-hidden recall contract for one flashcard image.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecallCard {
    shown: ShownRecall,
    hidden: HiddenRecall,
}

impl RecallCard {
    /// Create one recall contract from visible and hidden card fields.
    pub fn new(shown: ShownRecall, hidden: HiddenRecall) -> Self {
        Self { shown, hidden }
    }

    /// Render the injection-resistant image-review prompt with quoted JSON data.
    pub(crate) fn prompt(&self) -> Result<String> {
        let catalog = catalog();
        let language = catalog
            .resolve(self.hidden.hidden_target_language.as_str())
            .or_else(|_| catalog.item("en"))?;
        let examples = catalog.prompts(language.code)?;
        PromptTemplate::new(picture_recall_judge_prompt()).render(&[
            ("{card_json}", serde_json::to_string_pretty(self)?),
            ("{focus_example}", examples.recall_focus()?),
            ("{fragment_example}", examples.recall_fragment()?),
        ])
    }

    /// Return the bounded response schema used by the image-review model.
    pub(crate) fn schema(&self) -> Result<serde_json::Value> {
        Ok(serde_json::from_str(picture_recall_judge_schema())?)
    }

    /// Decode one model verdict with deterministic shown-versus-hidden phrase checks.
    pub(crate) fn review(&self, raw: &str) -> Result<RecallReview> {
        RecallReview::decode_for(self, raw)
    }

    fn shown_contains(&self, words: &[String]) -> bool {
        [
            self.shown.shown_source_sentence.as_str(),
            self.shown.shown_source_highlight.as_str(),
            self.shown.shown_hint.as_str(),
        ]
        .iter()
        .any(|value| contains_words(normalized_words(value).as_slice(), words))
    }

    fn focus_matches(&self, words: &[String]) -> bool {
        words == normalized_words(self.hidden.hidden_focus_term.as_str())
    }

    fn target_contains(&self, words: &[String]) -> bool {
        english(self.hidden.hidden_target_language.as_str())
            && words.len() >= 2
            && !all_function_words(words)
            && contains_words(
                normalized_words(self.hidden.hidden_target_sentence.as_str()).as_slice(),
                words,
            )
    }
}

/// Review one candidate image against a captured flashcard recall contract.
pub trait RecallJudge {
    /// Return the typed answer-leakage verdict for the supplied encoded image.
    fn review(&self, image: &[u8]) -> Result<RecallReview>;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum RecallDecision {
    Allow,
    Reject,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum RecallKind {
    Focus,
    TargetFragment,
    CompetingAnswer,
    VisibleCue,
    Unrelated,
}

impl RecallKind {
    fn rejects(self) -> bool {
        matches!(
            self,
            Self::Focus | Self::TargetFragment | Self::CompetingAnswer
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RecallEvidence {
    reading: String,
    location: String,
    kind: RecallKind,
}

/// Structured and locally validated image answer-leakage verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecallReview {
    decision: RecallDecision,
    evidence: Vec<RecallEvidence>,
    reason: String,
}

impl RecallReview {
    /// Decode one structured model verdict and normalize it from grounded evidence.
    pub(crate) fn decode(raw: &str) -> Result<Self> {
        let mut review = serde_json::from_str::<Self>(raw.trim())?;
        review.normalize();
        review.validate()?;
        Ok(review)
    }

    fn decode_for(card: &RecallCard, raw: &str) -> Result<Self> {
        let mut review = Self::decode(raw)?;
        for item in &mut review.evidence {
            let words = normalized_words(item.reading.as_str());
            if words.is_empty() {
                continue;
            }
            if card.shown_contains(words.as_slice()) {
                item.kind = RecallKind::VisibleCue;
            } else if card.focus_matches(words.as_slice()) {
                item.kind = RecallKind::Focus;
            } else if card.target_contains(words.as_slice()) {
                item.kind = RecallKind::TargetFragment;
            }
        }
        review.normalize();
        review.validate()?;
        Ok(review)
    }

    /// Return whether the candidate image is safe to show before answer reveal.
    #[must_use]
    pub fn allows(&self) -> bool {
        self.decision == RecallDecision::Allow
    }

    /// Return one concise rejection reason grounded in visible evidence.
    pub(crate) fn rejection(&self) -> Option<String> {
        if self.allows() {
            return None;
        }
        let evidence = self
            .evidence
            .iter()
            .filter(|item| item.kind.rejects())
            .map(|item| format!("'{}' at {}", item.reading, item.location))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("{}: {}", self.reason, evidence))
    }

    fn validate(&self) -> Result<()> {
        if self.reason.trim().is_empty() {
            bail!("recall review reason must not be empty");
        }
        if self.evidence.len() > 6 {
            bail!("recall review must contain at most six evidence items");
        }
        if self
            .evidence
            .iter()
            .any(|item| item.reading.trim().is_empty() || item.location.trim().is_empty())
        {
            bail!("recall review evidence must include reading and location");
        }
        Ok(())
    }

    fn normalize(&mut self) {
        self.decision = if self.evidence.iter().any(|item| item.kind.rejects()) {
            RecallDecision::Reject
        } else {
            RecallDecision::Allow
        };
    }
}

fn normalized_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn contains_words(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn english(language: &str) -> bool {
    matches!(language.to_ascii_lowercase().as_str(), "en" | "english")
}

fn all_function_words(words: &[String]) -> bool {
    words.iter().all(|word| {
        matches!(
            word.as_str(),
            "a" | "an"
                | "and"
                | "are"
                | "as"
                | "at"
                | "be"
                | "been"
                | "being"
                | "but"
                | "by"
                | "can"
                | "could"
                | "did"
                | "do"
                | "does"
                | "for"
                | "from"
                | "had"
                | "has"
                | "have"
                | "he"
                | "her"
                | "him"
                | "his"
                | "i"
                | "if"
                | "in"
                | "is"
                | "it"
                | "its"
                | "may"
                | "me"
                | "might"
                | "must"
                | "my"
                | "nor"
                | "not"
                | "of"
                | "on"
                | "or"
                | "she"
                | "shall"
                | "should"
                | "that"
                | "the"
                | "their"
                | "them"
                | "they"
                | "this"
                | "to"
                | "was"
                | "we"
                | "were"
                | "will"
                | "with"
                | "would"
                | "you"
                | "your"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{HiddenRecall, RecallCard, RecallReview, ShownRecall};

    #[test]
    fn answer_bearing_evidence_requires_rejection() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[{"reading":"FRIGHTEN","location":"top sign","kind":"FOCUS"}],"reason":"The focus term is visible"}"#,
        )
        .expect("grounded answer evidence must normalize");
        assert!(
            !review.allows(),
            "answer-bearing evidence was allowed because the model contradicted its own grounding"
        );
    }

    #[test]
    fn safe_grounded_evidence_overrides_an_unsupported_rejection() {
        let review = RecallReview::decode(
            r#"{"decision":"REJECT","evidence":[{"reading":"RUNWAY","location":"airport sign","kind":"VISIBLE_CUE"}],"reason":"The longer word contains the focus characters"}"#,
        )
        .expect("safe grounded evidence must normalize");
        assert!(
            review.allows(),
            "a coincidental substring was rejected because the model contradicted its evidence kind"
        );
    }

    #[test]
    fn unrelated_writing_remains_allowed() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[{"reading":"FARM","location":"barn sign","kind":"UNRELATED"}],"reason":"The label does not reveal the answer"}"#,
        )
        .expect("unrelated writing must decode");
        assert!(
            review.allows(),
            "unrelated visible writing was promoted into answer leakage"
        );
    }

    #[test]
    fn grounded_focus_term_is_rejected() {
        let review = RecallReview::decode(
            r#"{"decision":"REJECT","evidence":[{"reading":"FRIGHTEN","location":"top sign","kind":"FOCUS"}],"reason":"The focus term is visible"}"#,
        )
        .expect("grounded focus evidence must decode");
        assert!(
            !review.allows(),
            "a clearly visible focus term was accepted before answer reveal"
        );
    }

    #[test]
    fn recall_prompt_uses_the_hidden_target_languages_examples() {
        let card = RecallCard::new(
            ShownRecall::new(
                "English",
                "The manager approved the final design.",
                "approved",
                "Think of a confirmed decision.",
            ),
            HiddenRecall::new("zh", "批准", "经理批准了最终设计。"),
        );
        let prompt = card.prompt().expect("chinese recall prompt must render");
        assert!(
            prompt.contains(r#""focus":"马","longer":"马虎""#)
                && prompt.contains(r#""visible":"最终设计","sentence_end":"最终设计""#)
                && !prompt.contains("runway"),
            "recall prompt retained examples from another target language"
        );
    }

    #[test]
    fn every_supported_target_renders_its_recall_examples() {
        let complete = crate::languages::catalog().codes().into_iter().all(|code| {
            let card = RecallCard::new(
                ShownRecall::new(
                    "English",
                    "The manager approved the design.",
                    "approved",
                    "Think of a decision.",
                ),
                HiddenRecall::new(code, "term", "one target sentence"),
            );
            card.prompt().is_ok()
        });
        assert!(
            complete,
            "a supported target language cannot render its recall examples"
        );
    }

    #[test]
    fn arbitrary_recall_language_values_remain_serializable() {
        let card = RecallCard::new(
            ShownRecall::new("English", "A visible sentence.", "visible", "A hint."),
            HiddenRecall::new("Klingon", "qaH", "qaH vIpoQ."),
        );
        let encoded = serde_json::to_value(&card).expect("recall card must serialize");
        let prompt = card.prompt();
        assert_eq!(
            (
                encoded["hidden"]["hidden_target_language"].as_str(),
                prompt.is_ok()
            ),
            (Some("Klingon"), true),
            "the public recall path rejected a previously accepted language string"
        );
    }

    #[test]
    fn literal_hidden_sentence_fragment_overrides_unrelated_model_evidence() {
        let card = RecallCard::new(
            ShownRecall::new(
                "Russian",
                "Руководитель утвердил дизайн до обеда.",
                "утвердил",
                "О принятом решении.",
            ),
            HiddenRecall::new(
                "English",
                "approve",
                "The supervisor approved the final design before lunch.",
            ),
        );
        let review = card
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"BEFORE LUNCH","location":"top banner","kind":"UNRELATED"}],"reason":"The words are unrelated"}"#,
            )
            .expect("literal target fragment must decode");
        assert!(
            !review.allows(),
            "two consecutive hidden target words were allowed because the model mislabeled its transcription"
        );
    }

    #[test]
    fn generic_function_word_pair_does_not_become_a_target_fragment() {
        let card = RecallCard::new(
            ShownRecall::new(
                "Russian",
                "Она дошла до станции.",
                "дошла",
                "О завершенном пути.",
            ),
            HiddenRecall::new(
                "English",
                "walk",
                "She walked to the station before sunset.",
            ),
        );
        let review = card
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"TO THE","location":"station sign","kind":"UNRELATED"}],"reason":"The phrase is generic"}"#,
            )
            .expect("generic function words must decode");
        assert!(
            review.allows(),
            "two generic function words were promoted into answer-bearing target content"
        );
    }

    #[test]
    fn non_english_phrase_keeps_the_models_unrelated_evidence_kind() {
        let card = RecallCard::new(
            ShownRecall::new(
                "Russian",
                "Она находится на станции.",
                "находится",
                "О местонахождении.",
            ),
            HiddenRecall::new("French", "être", "Elle est à la gare avant midi."),
        );
        let review = card
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"À LA","location":"station sign","kind":"UNRELATED"}],"reason":"The phrase is generic"}"#,
            )
            .expect("non-English evidence must decode");
        assert!(
            review.allows(),
            "a non-English phrase was deterministically reclassified without language rules"
        );
    }

    #[test]
    fn phrase_already_shown_to_the_learner_remains_visible_cue() {
        let card = RecallCard::new(
            ShownRecall::new(
                "Russian",
                "Он проверил Google Maps.",
                "проверил",
                "Google Maps уже показано.",
            ),
            HiddenRecall::new("English", "check", "He checked Google Maps before leaving."),
        );
        let review = card
            .review(
                r#"{"decision":"REJECT","evidence":[{"reading":"GOOGLE MAPS","location":"phone","kind":"TARGET_FRAGMENT"}],"reason":"Two target words are visible"}"#,
            )
            .expect("shown phrase must decode");
        assert!(
            review.allows(),
            "text already visible in the shown card fields was treated as new answer leakage"
        );
    }

    #[test]
    fn exact_focus_reading_overrides_unrelated_model_evidence() {
        let card = RecallCard::new(
            ShownRecall::new(
                "Russian",
                "Коробка была хрупкой.",
                "хрупкой",
                "Об осторожном обращении.",
            ),
            HiddenRecall::new("English", "fragile", "The fragile box broke."),
        );
        let review = card
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"FRAGILE","location":"label","kind":"UNRELATED"}],"reason":"The label is unrelated"}"#,
            )
            .expect("focus reading must decode");
        assert!(
            !review.allows(),
            "an exact hidden focus reading was allowed because the model mislabeled its transcription"
        );
    }
}
