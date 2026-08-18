//! Typed verdicts for the literal-writing gate applied before semantic recall review.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use unicode_script::{Script, UnicodeScript};

use crate::generation::prompts::{picture_text_judge_prompt, picture_text_judge_schema};
use crate::prompt::PromptTemplate;

/// Direct image-text review request for one target language.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextCheck {
    language: String,
}

impl TextCheck {
    /// Create one direct image-text review request.
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            language: language.into(),
        }
    }

    /// Render the injection-resistant direct image-text prompt.
    pub(crate) fn prompt(&self) -> Result<String> {
        PromptTemplate::new(picture_text_judge_prompt())
            .render(&[("{language}", serde_json::to_string(&self.language)?)])
    }

    /// Return the bounded response schema used by the direct image-text model.
    pub(crate) fn schema(&self) -> Result<serde_json::Value> {
        Ok(serde_json::from_str(picture_text_judge_schema())?)
    }

    /// Decode one structured direct image-text verdict.
    pub(crate) fn review(&self, raw: &str) -> Result<TextReview> {
        let _ = self;
        TextReview::decode_llm(raw)
    }
}

/// Route that produced one archived literal-writing verdict.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TextReviewGate {
    /// PP-OCRv5 produced the verdict.
    Ocr,
    /// Gemini vision produced the verdict without OCR.
    LlmJudge,
}

impl TextReviewGate {
    /// Return the archive stage used when this gate cannot produce a verdict.
    pub(crate) fn error_category(self) -> &'static str {
        match self {
            Self::Ocr => "ocr",
            Self::LlmJudge => "text_judge",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum TextDecision {
    Allow,
    Reject,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum TextEvidenceKind {
    Writing,
    Numeral,
    MathematicalNotation,
    TechnicalDiagram,
    PseudoWriting,
    DecorativeGlyphString,
    InterfaceMark,
    SymbolOrEmblem,
    Ambiguous,
}

impl TextEvidenceKind {
    fn rejects(self) -> bool {
        !matches!(self, Self::Ambiguous)
    }

    fn weight(self) -> u32 {
        match self {
            Self::Writing => 12,
            Self::MathematicalNotation | Self::TechnicalDiagram => 10,
            Self::Numeral | Self::InterfaceMark => 8,
            Self::PseudoWriting => 6,
            Self::DecorativeGlyphString | Self::SymbolOrEmblem => 5,
            Self::Ambiguous => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct TextEvidence {
    reading: String,
    location: String,
    kind: TextEvidenceKind,
}

/// Structured and locally normalized literal-writing verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextReview {
    gate: TextReviewGate,
    decision: TextDecision,
    evidence: Vec<TextEvidence>,
    reason: String,
}

impl TextReview {
    /// Build one typed verdict from PP-OCRv5 output.
    pub(crate) fn ocr(found: &str) -> Self {
        let found = found.trim();
        let evidence = if found.is_empty() {
            Vec::new()
        } else {
            vec![TextEvidence {
                reading: String::from(found),
                location: String::from("detected by PP-OCRv5"),
                kind: if significant_literal(found) {
                    if found.chars().any(|character| character.is_numeric()) {
                        TextEvidenceKind::Numeral
                    } else {
                        TextEvidenceKind::Writing
                    }
                } else {
                    TextEvidenceKind::Ambiguous
                },
            }]
        };
        let mut review = Self {
            gate: TextReviewGate::Ocr,
            decision: TextDecision::Allow,
            evidence,
            reason: if found.is_empty() {
                String::from("PP-OCRv5 detected no writing")
            } else {
                format!("PP-OCRv5 detected '{found}'")
            },
        };
        review.normalize();
        review
    }

    /// Return whether this illustration passed the literal-writing gate.
    #[must_use]
    pub fn allows(&self) -> bool {
        self.decision == TextDecision::Allow
    }

    /// Return the route that produced this verdict.
    #[must_use]
    pub fn gate(&self) -> TextReviewGate {
        self.gate
    }

    /// Return the weighted quality penalty for detected non-leaking writing.
    ///
    /// A finding whose transcription holds no alphanumeric run of at least two
    /// characters is recognizer noise, not writing; it keeps only a trace
    /// weight so it can neither burn an attempt nor visibly drag the score.
    #[must_use]
    pub(crate) fn penalty(&self) -> u32 {
        self.evidence
            .iter()
            .map(|item| {
                if legible_run(item.reading.as_str()) {
                    item.kind.weight()
                } else {
                    item.kind.weight().min(NOISE_WEIGHT)
                }
            })
            .sum::<u32>()
            .min(40)
    }

    /// Return one concise rejection reason grounded in detected writing.
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

    fn decode_llm(raw: &str) -> Result<Self> {
        let decoded = serde_json::from_str::<LlmTextReview>(raw.trim())?;
        let mut review = Self {
            gate: TextReviewGate::LlmJudge,
            decision: decoded.decision,
            evidence: decoded.evidence,
            reason: decoded.reason,
        };
        review.normalize();
        review.validate()?;
        Ok(review)
    }

    fn normalize(&mut self) {
        self.decision = if self.evidence.iter().any(|item| item.kind.rejects()) {
            TextDecision::Reject
        } else {
            TextDecision::Allow
        };
    }

    fn validate(&self) -> Result<()> {
        if self.reason.trim().is_empty() {
            bail!("text review reason must not be empty");
        }
        if self.evidence.len() > 6 {
            bail!("text review must contain at most six evidence items");
        }
        if self
            .evidence
            .iter()
            .any(|item| item.reading.trim().is_empty() || item.location.trim().is_empty())
        {
            bail!("text review evidence must include reading and location");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct LlmTextReview {
    decision: TextDecision,
    evidence: Vec<TextEvidence>,
    reason: String,
}

const NOISE_WEIGHT: u32 = 2;

/// Return whether one transcription holds an alphanumeric run of two or more
/// characters — the minimum for a reading to be writing rather than noise.
fn legible_run(reading: &str) -> bool {
    let mut run = 0usize;
    for character in reading.chars() {
        if character.is_alphanumeric() {
            run += 1;
            if run >= 2 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Return whether one grounded reading violates the literal-writing policy.
pub(super) fn significant_literal(found: &str) -> bool {
    if found.chars().any(char::is_numeric) {
        return true;
    }
    if found.chars().any(|character| {
        character.is_alphabetic()
            && !matches!(
                character.script(),
                Script::Latin | Script::Common | Script::Inherited
            )
    }) {
        return true;
    }
    found
        .split(|character: char| !character.is_alphabetic())
        .any(|token| token.chars().count() >= 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_verdicts_keep_significant_and_low_signal_text_distinct() {
        assert_eq!(
            ["", "un", "OPEN", "ש", "7"].map(|found| TextReview::ocr(found).allows()),
            [true, true, false, false, false],
            "OCR significance policy accepted writing or rejected low-signal Latin glyphs"
        );
    }

    #[test]
    fn recognizer_noise_without_a_legible_run_keeps_only_a_trace_penalty() {
        assert_eq!(
            (
                TextReview::ocr("\\,w/4/").penalty(),
                TextReview::ocr("sale").penalty(),
            ),
            (2, 12),
            "recognizer gibberish was weighted like real writing"
        );
    }

    #[test]
    fn grounded_llm_evidence_overrides_a_contradictory_decision() {
        let review = TextReview::decode_llm(
            r#"{"decision":"ALLOW","evidence":[{"reading":"שלום","location":"center sign","kind":"WRITING"}],"reason":"The word is clearly legible"}"#,
        )
        .expect("grounded direct text verdict must decode");
        assert!(
            !review.allows() && review.gate() == TextReviewGate::LlmJudge,
            "direct text evidence was allowed because the model contradicted its grounding"
        );
    }

    #[test]
    fn populated_table_pseudo_writing_overrides_a_contradictory_allow_decision() {
        let review = TextCheck::new("Hebrew")
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"repeated short record marks filling ruled rows","location":"right-panel table","kind":"PSEUDO_WRITING"}],"reason":"The table contains no readable characters"}"#,
            )
            .expect("grounded populated-table evidence must decode");
        assert!(
            !review.allows(),
            "populated table marks were allowed because the model contradicted its grounding"
        );
    }

    #[test]
    fn technical_diagram_overrides_a_contradictory_allow_decision() {
        let review = TextCheck::new("Vietnamese")
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"engineering schematic encoded with conventional lines and symbols","location":"right-panel drafting sheet","kind":"TECHNICAL_DIAGRAM"}],"reason":"The drawing contains no readable characters"}"#,
            )
            .expect("grounded technical-diagram evidence must decode");
        assert!(
            !review.allows(),
            "technical diagram was allowed because the model contradicted its grounding"
        );
    }

    #[test]
    fn symbol_or_emblem_overrides_a_contradictory_allow_decision() {
        let review = TextCheck::new("Hebrew")
            .review(
                r#"{"decision":"ALLOW","evidence":[{"reading":"deliberate white V-like glyph enclosed in a black transit badge","location":"train front","kind":"SYMBOL_OR_EMBLEM"}],"reason":"The badge contains no readable word"}"#,
            )
            .expect("grounded symbol-or-emblem evidence must decode");
        assert!(
            !review.allows(),
            "symbol or emblem was allowed because the model contradicted its grounding"
        );
    }

    #[test]
    fn ordinary_hardware_and_blank_plates_remain_ambiguous() {
        let review = TextCheck::new("Vietnamese")
            .review(
                r#"{"decision":"REJECT","evidence":[{"reading":"blank plate beside headlights, reflectors, lamps, bolts, screws, handles, latches, hinges, vents, grilles, couplers, wipers, door and window seams, and structural contours","location":"train front","kind":"AMBIGUOUS"}],"reason":"No distinct applied inner graphic can be grounded"}"#,
            )
            .expect("ambiguous direct hardware evidence must decode");
        assert!(
            review.allows(),
            "ordinary hardware, blank panels, or enclosure alone became symbol-or-emblem evidence"
        );
    }

    #[test]
    fn grounded_llm_writing_like_evidence_rejects_except_ambiguous_marks() {
        let reviews = [
            ("PSEUDO_WRITING", "dense pseudo-writing"),
            ("MATHEMATICAL_NOTATION", "mathematical formulas"),
            ("NUMERAL", "42"),
            ("DECORATIVE_GLYPH_STRING", "ornamental glyph row"),
            ("INTERFACE_MARK", "button legend"),
            ("AMBIGUOUS", "possible marks in cross-hatching"),
        ]
        .map(|(kind, reading)| {
            TextReview::decode_llm(
                serde_json::json!({
                    "decision": "ALLOW",
                    "evidence": [{
                        "reading": reading,
                        "location": "upper panel",
                        "kind": kind
                    }],
                    "reason": "The observation is grounded"
                })
                .to_string()
                .as_str(),
            )
            .expect("grounded direct text evidence must decode")
        });
        assert_eq!(
            reviews.map(|review| review.allows()),
            [false, false, false, false, false, true],
            "direct text gate accepted writing-like content or rejected ambiguous hatching"
        );
    }

    #[test]
    fn archived_text_verdict_names_the_gate_and_grounded_reading() {
        let review = TextReview::ocr("OPEN");
        let value = serde_json::to_value(review).expect("text verdict must serialize");
        assert_eq!(
            (
                value["gate"].as_str(),
                value["decision"].as_str(),
                value["evidence"][0]["reading"].as_str(),
            ),
            (Some("OCR"), Some("REJECT"), Some("OPEN")),
            "archived text verdict lost its gate, decision, or reading"
        );
    }

    #[test]
    fn literal_significance_counts_scripts_letters_and_numerals_exactly() {
        assert_eq!(
            [
                "", "E", "É", "AB", "éß", "OPEN", "école", "ש", "高校", "7", "A7"
            ]
            .map(significant_literal),
            [
                false, false, false, false, false, true, true, true, true, true, true
            ],
            "literal-writing policy confused short Latin labels with significant writing"
        );
    }
}
