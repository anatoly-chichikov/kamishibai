//! Typed flashcard context and verdicts for image-based answer-leakage review.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::generation::prompts::{
    picture_fidelity_judge_prompt, picture_fidelity_judge_schema,
    picture_literal_zoom_judge_prompt, picture_literal_zoom_judge_schema,
    picture_recall_judge_prompt, picture_recall_judge_schema,
};
use crate::languages::{LanguageProfile, catalog};
use crate::prompt::PromptTemplate;

use super::text_gate::significant_literal;

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
    pub(crate) fn prompt(&self, scene: &Value) -> Result<String> {
        let catalog = catalog();
        let language = catalog
            .identify(self.hidden.hidden_target_language.as_str())
            .or_else(|_| catalog.item("en"))?;
        let examples = catalog.prompts(language.code)?;
        let fidelity = SceneFidelityContract::from_scene(scene)?;
        PromptTemplate::new(picture_recall_judge_prompt()).render(&[
            ("{card_json}", serde_json::to_string_pretty(self)?),
            (
                "{scene_fidelity_json}",
                serde_json::to_string_pretty(&fidelity)?,
            ),
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SceneFidelityContract {
    semantic_spine: SceneFidelitySemantic,
    panels: Vec<SceneFidelityPanel>,
}

impl SceneFidelityContract {
    fn from_scene(scene: &Value) -> Result<Self> {
        let root = scene
            .get("manga_panel")
            .ok_or_else(|| anyhow::anyhow!("scene fidelity requires manga_panel"))?;
        let spine = root
            .get("semantic_spine")
            .ok_or_else(|| anyhow::anyhow!("scene fidelity requires semantic_spine"))?;
        let literal_anchor = spine
            .get("literal_anchor")
            .and_then(Value::as_str)
            .filter(|anchor| !anchor.trim().is_empty())
            .or_else(|| {
                spine
                    .pointer("/metaphor/literal_anchor")
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| anyhow::anyhow!("scene fidelity requires literal_anchor"))?;
        let semantic_spine = SceneFidelitySemantic {
            literal_event: required_string(spine, "/literal_event", "literal_event")?,
            literal_anchor: String::from(literal_anchor),
            semantic_focus: required_string(spine, "/semantic_focus", "semantic_focus")?,
            visual_relation: required_string(spine, "/visual_relation", "visual_relation")?,
        };
        let panels = root
            .get("panels")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("scene fidelity requires panels"))?
            .iter()
            .map(SceneFidelityPanel::from_value)
            .collect::<Result<Vec<_>>>()?;
        if panels.is_empty() {
            bail!("scene fidelity requires at least one panel");
        }
        Ok(Self {
            semantic_spine,
            panels,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SceneFidelitySemantic {
    literal_event: String,
    literal_anchor: String,
    semantic_focus: String,
    visual_relation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SceneFidelityPanel {
    id: String,
    semantic_job: String,
    visible_anchor: String,
    scene: SceneFidelityScene,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SceneFidelityScene {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    subjects: Vec<SceneFidelitySubject>,
}

impl SceneFidelityPanel {
    fn from_value(panel: &Value) -> Result<Self> {
        let id = required_string(panel, "/id", "panel id")?;
        let subjects = panel
            .pointer("/scene/subjects")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("scene fidelity panel {id} requires subjects"))?
            .iter()
            .map(SceneFidelitySubject::from_value)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            id,
            semantic_job: required_string(panel, "/semantic_job", "panel semantic_job")?,
            visible_anchor: required_string(
                panel,
                "/shot_contract/visible_anchor",
                "panel visible_anchor",
            )?,
            scene: SceneFidelityScene {
                description: panel
                    .pointer("/scene/description")
                    .and_then(Value::as_str)
                    .filter(|description| !description.trim().is_empty())
                    .map(String::from),
                subjects,
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SceneFidelitySubject {
    id: String,
    figure: String,
    pose: String,
    expression: String,
}

impl SceneFidelitySubject {
    fn from_value(subject: &Value) -> Result<Self> {
        Ok(Self {
            id: required_string(subject, "/id", "subject id")?,
            figure: required_string(subject, "/figure", "subject figure")?,
            pose: required_string(subject, "/pose", "subject pose")?,
            expression: required_string(subject, "/expression", "subject expression")?,
        })
    }
}

fn required_string(value: &Value, pointer: &str, field: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|item| !item.trim().is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("scene fidelity requires non-empty {field}"))
}

/// Visual-only scene-fidelity contract without flashcard answer fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FidelityCheck {
    scene: SceneFidelityContract,
}

impl FidelityCheck {
    /// Create one compact scene-fidelity check from the composed scene.
    pub(crate) fn new(scene: &Value) -> Result<Self> {
        Ok(Self {
            scene: SceneFidelityContract::from_scene(scene)?,
        })
    }

    /// Render the dedicated fidelity prompt with only compact visual requirements.
    pub(crate) fn prompt(&self) -> Result<String> {
        PromptTemplate::new(picture_fidelity_judge_prompt()).render(&[(
            "{scene_fidelity_json}",
            serde_json::to_string_pretty(&self.scene)?,
        )])
    }

    /// Return the bounded dedicated fidelity response schema.
    pub(crate) fn schema(&self) -> Result<Value> {
        Ok(serde_json::from_str(picture_fidelity_judge_schema())?)
    }

    /// Decode one grounded dedicated fidelity verdict.
    pub(crate) fn review(&self, raw: &str) -> Result<FidelityReview> {
        FidelityReview::decode(raw)
    }
}

/// Review one candidate image against a captured flashcard recall contract.
pub trait RecallJudge {
    /// Return the typed answer-leakage verdict for the supplied encoded image.
    fn review(&self, scene: &Value, image: &[u8]) -> Result<RecallReview>;
}

/// Literal-only scale-aware review contract without flashcard answer strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiteralZoomCheck;

impl LiteralZoomCheck {
    /// Create one literal-only scale-aware review contract.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Return the prompt that explains the ordered overlapping crop set.
    pub(crate) fn prompt(self) -> &'static str {
        picture_literal_zoom_judge_prompt()
    }

    /// Return the bounded literal-only response schema.
    pub(crate) fn schema(self) -> Result<serde_json::Value> {
        Ok(serde_json::from_str(picture_literal_zoom_judge_schema())?)
    }

    /// Decode one grounded scale-aware literal verdict.
    pub(crate) fn review(self, raw: &str) -> Result<LiteralZoomReview> {
        LiteralZoomReview::decode(raw)
    }
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum SceneFidelityDecision {
    #[default]
    Allow,
    Reject,
}

/// How the page presents its panel-frame structure.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PageFrame {
    #[default]
    Framed,
    Bleed,
    Breakout,
    Torn,
    Borderless,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum SceneFidelityKind {
    #[serde(rename = "MISSING_REQUIRED_SUBJECT")]
    Subject,
    #[serde(rename = "MISSING_REQUIRED_RELATION")]
    Relation,
    #[serde(rename = "MISSING_LITERAL_ANCHOR")]
    LiteralAnchor,
    #[serde(rename = "BROKEN_SUBJECT_CONTINUITY")]
    SubjectContinuity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum LiteralEvidenceKind {
    Writing,
    Numeral,
    MathematicalNotation,
    TechnicalDiagram,
    PseudoWriting,
    DecorativeGlyphString,
    InterfaceMark,
    SymbolOrEmblem,
    AmbiguousMark,
}

impl LiteralEvidenceKind {
    fn rejects(self) -> bool {
        !matches!(self, Self::AmbiguousMark)
    }

    fn weight(self) -> u32 {
        match self {
            Self::Writing => 12,
            Self::MathematicalNotation | Self::TechnicalDiagram => 10,
            Self::Numeral | Self::InterfaceMark => 8,
            Self::PseudoWriting => 6,
            Self::DecorativeGlyphString | Self::SymbolOrEmblem => 5,
            Self::AmbiguousMark => 0,
        }
    }
}

impl SceneFidelityKind {
    fn weight(self) -> u32 {
        match self {
            Self::SubjectContinuity => 20,
            Self::Subject | Self::Relation => 15,
            Self::LiteralAnchor => 10,
        }
    }
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SceneFidelityEvidence {
    requirement: String,
    observed: String,
    location: String,
    kind: SceneFidelityKind,
}

/// Grounded visual-only scene-fidelity verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FidelityReview {
    scene_fidelity_decision: SceneFidelityDecision,
    scene_fidelity_evidence: Vec<SceneFidelityEvidence>,
    reason: String,
}

impl FidelityReview {
    fn decode(raw: &str) -> Result<Self> {
        let mut review = serde_json::from_str::<Self>(raw.trim())?;
        review.scene_fidelity_decision = if review.scene_fidelity_evidence.is_empty() {
            SceneFidelityDecision::Allow
        } else {
            SceneFidelityDecision::Reject
        };
        if review.reason.trim().is_empty() {
            bail!("fidelity review reason must not be empty");
        }
        if review.scene_fidelity_evidence.len() > 6 {
            bail!("fidelity review must contain at most six evidence items");
        }
        if review.scene_fidelity_evidence.iter().any(|item| {
            item.requirement.trim().is_empty()
                || item.observed.trim().is_empty()
                || item.location.trim().is_empty()
        }) {
            bail!("fidelity review evidence must include requirement, observation, and location");
        }
        Ok(review)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LiteralEvidence {
    description: String,
    location: String,
    kind: LiteralEvidenceKind,
}

/// Grounded literal-only verdict from the ordered enlarged crop set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LiteralZoomReview {
    literal_writing_present: bool,
    literal_evidence: Vec<LiteralEvidence>,
    reason: String,
}

impl LiteralZoomReview {
    fn decode(raw: &str) -> Result<Self> {
        let mut review = serde_json::from_str::<Self>(raw.trim())?;
        review.normalize();
        review.validate()?;
        Ok(review)
    }

    fn normalize(&mut self) {
        self.literal_writing_present = self.literal_evidence.iter().any(|item| item.kind.rejects());
    }

    fn validate(&self) -> Result<()> {
        if self.reason.trim().is_empty() {
            bail!("literal zoom review reason must not be empty");
        }
        if self.literal_evidence.len() > 6 {
            bail!("literal zoom review must contain at most six evidence items");
        }
        if self
            .literal_evidence
            .iter()
            .any(|item| item.description.trim().is_empty() || item.location.trim().is_empty())
        {
            bail!("literal zoom review evidence must include description and location");
        }
        if self.literal_evidence.iter().any(|item| {
            !item.location.starts_with("crop ") || !item.location.contains("; original ")
        }) {
            bail!("literal zoom review location must identify one crop and original region");
        }
        Ok(())
    }
}

/// Structured and locally validated image answer-leakage verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecallReview {
    decision: RecallDecision,
    evidence: Vec<RecallEvidence>,
    #[serde(default)]
    scene_fidelity_decision: SceneFidelityDecision,
    #[serde(default)]
    scene_fidelity_evidence: Vec<SceneFidelityEvidence>,
    #[serde(default)]
    literal_writing_present: bool,
    #[serde(default)]
    literal_evidence: Vec<LiteralEvidence>,
    #[serde(default)]
    fidelity_inspected: bool,
    #[serde(default)]
    zoom_inspected: bool,
    #[serde(default)]
    page_frame: PageFrame,
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

    /// Return whether a leak-free full-frame verdict needs dedicated fidelity
    /// inspection. Fidelity and literal findings no longer stop the chain —
    /// they only lower the score, so the inspections that guard acceptance
    /// must complete regardless of them.
    pub(crate) fn needs_fidelity(&self) -> bool {
        self.allows() && !self.fidelity_inspected
    }

    /// Return whether a leak-free full-frame verdict still needs enlarged
    /// literal inspection.
    pub(crate) fn needs_zoom(&self) -> bool {
        self.allows() && self.fidelity_inspected && !self.zoom_inspected
    }

    /// Return whether every downstream defense required for acceptance ran.
    pub(crate) fn inspections_complete(&self) -> bool {
        self.fidelity_inspected && self.zoom_inspected
    }

    /// Return whether the judge saw no panel-frame structure anywhere on the page.
    #[must_use]
    pub fn frame_borderless(&self) -> bool {
        self.page_frame == PageFrame::Borderless
    }

    /// Return whether the frame lines themselves carry a generation artifact.
    #[must_use]
    pub fn frame_torn(&self) -> bool {
        self.page_frame == PageFrame::Torn
    }

    /// Return whether panel content deliberately crosses its frame as a device.
    #[must_use]
    pub fn frame_breakout(&self) -> bool {
        self.page_frame == PageFrame::Breakout
    }

    /// Merge dedicated fidelity evidence while preserving semantic and literal verdicts.
    pub(crate) fn merged_fidelity(mut self, fidelity: FidelityReview) -> Result<Self> {
        let mut evidence = fidelity.scene_fidelity_evidence;
        for item in self.scene_fidelity_evidence {
            if evidence.len() == 6 {
                break;
            }
            if !evidence.contains(&item) {
                evidence.push(item);
            }
        }
        self.scene_fidelity_evidence = evidence;
        self.fidelity_inspected = true;
        self.normalize();
        self.validate()?;
        Ok(self)
    }

    /// Merge grounded enlarged-crop evidence while preserving semantic recall evidence.
    pub(crate) fn merged_zoom(mut self, zoom: LiteralZoomReview) -> Result<Self> {
        let (zoom_rejecting, zoom_ambiguous): (Vec<_>, Vec<_>) = zoom
            .literal_evidence
            .into_iter()
            .partition(|item| item.kind.rejects());
        let (full_rejecting, full_ambiguous): (Vec<_>, Vec<_>) = self
            .literal_evidence
            .into_iter()
            .partition(|item| item.kind.rejects());
        let mut literal = Vec::new();
        for item in zoom_rejecting
            .into_iter()
            .chain(full_rejecting)
            .chain(zoom_ambiguous)
            .chain(full_ambiguous)
        {
            if literal.len() == 6 {
                break;
            }
            if !literal.contains(&item) {
                literal.push(item);
            }
        }
        self.literal_evidence = literal;
        self.zoom_inspected = true;
        self.normalize();
        self.validate()?;
        Ok(self)
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

    /// Return one concise rejection grounded in a missing required scene element.
    pub(crate) fn scene_fidelity_rejection(&self) -> Option<String> {
        if self.scene_fidelity_evidence.is_empty() {
            return None;
        }
        Some(
            self.scene_fidelity_evidence
                .iter()
                .map(|item| {
                    format!(
                        "{} at {} (observed: {})",
                        item.requirement, item.location, item.observed
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// Return the weighted quality penalty for missing required scene content.
    #[must_use]
    pub(crate) fn fidelity_penalty(&self) -> u32 {
        self.scene_fidelity_evidence
            .iter()
            .map(|item| item.kind.weight())
            .sum::<u32>()
            .min(40)
    }

    /// Return the weighted quality penalty for non-leaking literal writing.
    #[must_use]
    pub(crate) fn literal_penalty(&self) -> u32 {
        let zoomed = self
            .literal_evidence
            .iter()
            .map(|item| item.kind.weight())
            .sum::<u32>();
        let transcribed = self
            .evidence
            .iter()
            .filter(|item| !item.kind.rejects() && significant_literal(item.reading.as_str()))
            .count();
        let transcribed = u32::try_from(transcribed)
            .unwrap_or(u32::MAX)
            .saturating_mul(8);
        zoomed.saturating_add(transcribed).min(40)
    }

    /// Return significant literal writing transcribed by the downstream review.
    pub(crate) fn literal_rejection(&self) -> Option<String> {
        let mut literal = self
            .literal_evidence
            .iter()
            .filter(|item| item.kind.rejects())
            .map(|item| format!("{} at {}", item.description, item.location))
            .collect::<Vec<_>>();
        if literal.is_empty() {
            literal.extend(
                self.evidence
                    .iter()
                    .filter(|item| significant_literal(item.reading.as_str()))
                    .map(|item| format!("'{}' at {}", item.reading, item.location)),
            );
        }
        let literal = literal.join(", ");
        if literal.is_empty() {
            return None;
        }
        Some(literal)
    }

    fn validate(&self) -> Result<()> {
        if self.reason.trim().is_empty() {
            bail!("recall review reason must not be empty");
        }
        if self.evidence.len() > 6 {
            bail!("recall review must contain at most six evidence items");
        }
        if self.scene_fidelity_evidence.len() > 6 {
            bail!("recall review must contain at most six scene fidelity evidence items");
        }
        if self.literal_evidence.len() > 6 {
            bail!("recall review must contain at most six literal evidence items");
        }
        if self
            .evidence
            .iter()
            .any(|item| item.reading.trim().is_empty() || item.location.trim().is_empty())
        {
            bail!("recall review evidence must include reading and location");
        }
        if self
            .literal_evidence
            .iter()
            .any(|item| item.description.trim().is_empty() || item.location.trim().is_empty())
        {
            bail!("recall review literal evidence must include description and location");
        }
        if self.scene_fidelity_evidence.iter().any(|item| {
            item.requirement.trim().is_empty()
                || item.observed.trim().is_empty()
                || item.location.trim().is_empty()
        }) {
            bail!(
                "recall review scene fidelity evidence must include requirement, observation, and location"
            );
        }
        Ok(())
    }

    fn normalize(&mut self) {
        self.decision = if self.evidence.iter().any(|item| item.kind.rejects()) {
            RecallDecision::Reject
        } else {
            RecallDecision::Allow
        };
        self.scene_fidelity_decision = if self.scene_fidelity_evidence.is_empty() {
            SceneFidelityDecision::Allow
        } else {
            SceneFidelityDecision::Reject
        };
        self.literal_writing_present = self.literal_evidence.iter().any(|item| item.kind.rejects());
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
    use super::{
        FidelityReview, HiddenRecall, LiteralZoomReview, RecallCard, RecallReview,
        SceneFidelityContract, ShownRecall,
    };

    fn fidelity_scene() -> serde_json::Value {
        serde_json::json!({
            "manga_panel": {
                "semantic_spine": {
                    "literal_event": "one visitor opens an unmarked door",
                    "semantic_focus": "a required visitor entering",
                    "visual_relation": "approach",
                    "metaphor": {"literal_anchor": "the unmarked door"}
                },
                "panels": [{
                    "id": "p1",
                    "semantic_job": "show the visitor opening the door",
                    "shot_contract": {"visible_anchor": "one visitor at an open door"},
                    "scene": {"subjects": [{
                        "id": "visitor",
                        "figure": "a visitor in a plain coat",
                        "pose": "opening the door",
                        "expression": "attentive"
                    }]}
                }]
            }
        })
    }

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
    fn semantic_allow_verdict_preserves_grounded_literal_fallback_evidence() {
        let reviews = [
            r#"{"decision":"ALLOW","evidence":[{"reading":"高校","location":"gate pillar","kind":"UNRELATED"}],"reason":"The school label is unrelated"}"#,
            r#"{"decision":"ALLOW","evidence":[{"reading":"Google Maps","location":"phone screen","kind":"VISIBLE_CUE"}],"reason":"The label repeats a shown cue"}"#,
            r#"{"decision":"ALLOW","evidence":[{"reading":"7","location":"door","kind":"UNRELATED"}],"reason":"The numeral is unrelated"}"#,
        ]
        .map(|raw| RecallReview::decode(raw).expect("grounded allow review must decode"));
        assert_eq!(
            reviews.map(|review| (review.allows(), review.literal_rejection().is_some())),
            [(true, true); 3],
            "semantic allow evidence discarded a literal-writing fallback signal"
        );
    }

    #[test]
    fn literal_fallback_ignores_short_latin_and_ungrounded_reason_text() {
        let reviews = [
            r#"{"decision":"ALLOW","evidence":[{"reading":"E","location":"buckle","kind":"UNRELATED"}],"reason":"The label is unrelated"}"#,
            r#"{"decision":"ALLOW","evidence":[{"reading":"Éé","location":"texture","kind":"UNRELATED"}],"reason":"The marks are ambiguous"}"#,
            r#"{"decision":"ALLOW","evidence":[],"reason":"Possible pseudo-writing appears in the background"}"#,
        ]
        .map(|raw| RecallReview::decode(raw).expect("safe allow review must decode"));
        assert_eq!(
            reviews.map(|review| review.literal_rejection()),
            [None, None, None],
            "literal fallback promoted short Latin or ungrounded prose into writing"
        );
    }

    #[test]
    fn grounded_literal_policy_rejects_writing_like_content_without_changing_semantic_recall() {
        let reviews = [
            ("PSEUDO_WRITING", "rows of CJK-like pseudo-glyphs"),
            ("MATHEMATICAL_NOTATION", "x squared plus y squared"),
            ("NUMERAL", "42"),
            (
                "DECORATIVE_GLYPH_STRING",
                "repeated ornamental glyph string",
            ),
        ]
        .map(|(kind, description)| {
            RecallReview::decode(
                serde_json::json!({
                    "decision": "ALLOW",
                    "evidence": [],
                    "literal_writing_present": false,
                    "literal_evidence": [{
                        "description": description,
                        "location": "upper panel",
                        "kind": kind
                    }],
                    "reason": "No answer-bearing writing is visible"
                })
                .to_string()
                .as_str(),
            )
            .expect("grounded literal evidence must decode")
        });
        assert_eq!(
            reviews.map(|review| (review.allows(), review.literal_rejection().is_some())),
            [(true, true); 4],
            "writing-like content escaped or contaminated the independent semantic recall verdict"
        );
    }

    #[test]
    fn populated_ledger_pseudo_writing_rejects_literal_policy_without_changing_semantic_recall() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[{"description":"populated ledger rows containing repeated short entry-like strokes","location":"right-panel ledger","kind":"PSEUDO_WRITING"}],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded populated-ledger evidence must decode");
        assert_eq!(
            (review.allows(), review.literal_rejection().is_some()),
            (true, true),
            "populated ledger marks escaped literal rejection or contaminated semantic recall"
        );
    }

    #[test]
    fn technical_diagram_rejects_literal_policy_without_changing_semantic_recall() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[{"description":"architectural floor plan encoding rooms with conventional lines and symbols","location":"right-panel drafting table","kind":"TECHNICAL_DIAGRAM"}],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded technical-diagram evidence must decode");
        assert_eq!(
            (review.allows(), review.literal_rejection().is_some()),
            (true, true),
            "technical diagram escaped literal rejection or contaminated semantic recall"
        );
    }

    #[test]
    fn symbol_or_emblem_rejects_literal_policy_without_changing_semantic_recall() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[{"description":"deliberate white V-like glyph enclosed in a black transit badge","location":"train front in right panel","kind":"SYMBOL_OR_EMBLEM"}],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded symbol-or-emblem evidence must decode");
        assert_eq!(
            (review.allows(), review.literal_rejection().is_some()),
            (true, true),
            "symbol or emblem escaped literal rejection or contaminated semantic recall"
        );
    }

    #[test]
    fn missing_required_subject_rejects_scene_fidelity_without_changing_semantic_recall() {
        let card = RecallCard::new(
            ShownRecall::new("Thai", "ฉันทนไม่ไหวแล้ว", "ทน", "คนหนึ่งกำลังอดทน"),
            HiddenRecall::new(
                "English",
                "put up with",
                "I cannot put up with him anymore.",
            ),
        );
        let mut scene = fidelity_scene();
        scene["manga_panel"]["semantic_spine"] = serde_json::json!({
            "literal_event": "one weary speaker endures an agitated companion",
            "semantic_focus": "endurance of another person's bad temper",
            "visual_relation": "opposition",
            "metaphor": {"literal_anchor": "the hostile companion confronting the speaker"}
        });
        scene["manga_panel"]["panels"][0] = serde_json::json!({
            "id": "p1",
            "semantic_job": "show the speaker enduring the companion's hostile behavior",
            "shot_contract": {"visible_anchor": "two individuals sharing a tense room"},
            "scene": {"subjects": [{
                "id": "speaker",
                "figure": "a weary seated man",
                "pose": "sitting stiffly",
                "expression": "strained"
            }, {
                "id": "agitated_companion",
                "figure": "a tall man in a rumpled shirt",
                "pose": "leaning forward and gesturing aggressively",
                "expression": "shouting angrily"
            }]}
        });
        let prompt = card
            .prompt(&scene)
            .expect("two-subject fidelity contract must render");
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[{"requirement":"panel p1 requires agitated_companion, a tall man shouting while leaning forward","observed":"only the weary seated speaker is visible and no second person appears","location":"both panels","kind":"MISSING_REQUIRED_SUBJECT"}],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded missing-subject evidence must decode");
        let archived = serde_json::to_value(&review).expect("fidelity review must serialize");
        assert_eq!(
            (
                prompt.contains("\"id\": \"speaker\"")
                    && prompt.contains("\"id\": \"agitated_companion\""),
                review.allows(),
                review.scene_fidelity_rejection().is_some(),
                archived["scene_fidelity_decision"].as_str(),
            ),
            (true, true, true, Some("REJECT")),
            "a missing required subject escaped fidelity rejection or contaminated semantic recall"
        );
    }

    #[test]
    fn broken_subject_continuity_rejects_scene_fidelity_without_changing_semantic_recall() {
        let card = RecallCard::new(
            ShownRecall::new(
                "English",
                "The neighbor has been trying to convince me for an hour.",
                "neighbor",
                "One person keeps talking while another grows exhausted.",
            ),
            HiddenRecall::new("Hebrew", "נודניק", "The neighbor is a real pest."),
        );
        let mut scene = fidelity_scene();
        scene["manga_panel"]["semantic_spine"] = serde_json::json!({
            "literal_event": "a persistent neighbor talks while an exhausted listener rubs his eyes",
            "semantic_focus": "the neighbor's persistence burdening the listener",
            "visual_relation": "burden",
            "metaphor": {"literal_anchor": "the listener exhausted by the persistent neighbor"}
        });
        scene["manga_panel"]["panels"] = serde_json::json!([{
            "id": "p1",
            "semantic_job": "show the neighbor talking continuously to the drained speaker",
            "shot_contract": {"visible_anchor": "two distinct men, with the neighbor gesturing toward the speaker"},
            "scene": {"subjects": [{
                "id": "neighbor",
                "figure": "a middle-aged man in a casual sweater",
                "pose": "leaning forward and gesturing with open hands",
                "expression": "eager and energetic"
            }, {
                "id": "speaker",
                "figure": "a younger man in a plain shirt",
                "pose": "sitting stiffly with hands on his knees",
                "expression": "polite but drained"
            }]}
        }, {
            "id": "p2",
            "semantic_job": "show the same speaker rubbing his eyes in exhaustion",
            "shot_contract": {"visible_anchor": "the exhausted speaker slumped and rubbing his eyes"},
            "scene": {"subjects": [{
                "id": "speaker",
                "figure": "the younger man in a plain shirt",
                "pose": "slumped while rubbing the bridge of his nose",
                "expression": "eyes closed in quiet defeat"
            }]}
        }]);
        let prompt = card
            .prompt(&scene)
            .expect("subject-continuity contract must render");
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[{"requirement":"subject id speaker must remain the drained listener from panel p1 into panel p2","observed":"panel p2 repeats the gesturing talkative neighbor while the distinct back-facing listener from panel p1 disappears","location":"speaker in left panel p1 and figure in right panel p2","kind":"BROKEN_SUBJECT_CONTINUITY"}],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded broken-continuity evidence must decode");
        let archived = serde_json::to_value(&review).expect("continuity review must serialize");
        assert_eq!(
            (
                prompt.matches("\"id\": \"speaker\"").count(),
                review.allows(),
                review.scene_fidelity_rejection().is_some(),
                archived["scene_fidelity_decision"].as_str(),
            ),
            (2, true, true, Some("REJECT")),
            "a swapped repeated subject escaped fidelity rejection or contaminated semantic recall"
        );
    }

    #[test]
    fn harmless_view_and_clothing_variation_does_not_break_subject_continuity() {
        let review = RecallReview::decode(
            r#"{"decision":"REJECT","evidence":[],"scene_fidelity_decision":"REJECT","scene_fidelity_evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"The same speaker remains identifiable from role and stable figure cues despite a rear camera angle and an ordinary clothing-fold variation"}"#,
        )
        .expect("benign continuity variation must decode");
        assert_eq!(
            (review.allows(), review.scene_fidelity_rejection()),
            (true, None),
            "camera or ordinary clothing variation alone became a scene-fidelity rejection"
        );
    }

    #[test]
    fn blank_calendar_cannot_prove_an_overdue_deadline_from_the_untrusted_scene() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[{"requirement":"panel p2 requires physical evidence that the deadline is severely overdue","observed":"the calendar page is completely blank and no independent pixel cue shows any date, deadline, or missed state","location":"blank calendar in right panel","kind":"MISSING_LITERAL_ANCHOR"}],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("grounded blank-calendar omission must decode");
        let archived = serde_json::to_value(&review).expect("blank-calendar review must serialize");
        assert_eq!(
            (
                review.allows(),
                review.scene_fidelity_rejection().is_some(),
                review.literal_rejection(),
                archived["scene_fidelity_decision"].as_str(),
            ),
            (true, true, None, Some("REJECT")),
            "a blank carrier borrowed invisible deadline data or contaminated semantic and literal gates"
        );
    }

    #[test]
    fn ordinary_hardware_and_blank_plates_remain_ambiguous_in_full_and_zoom_reviews() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":true,"literal_evidence":[{"description":"blank plate beside headlights, reflectors, lamps, bolts, screws, handles, latches, hinges, vents, grilles, couplers, wipers, door and window seams, and structural contours","location":"train front","kind":"AMBIGUOUS_MARK"}],"reason":"No distinct applied inner graphic can be grounded"}"#,
        )
        .expect("ambiguous full-frame hardware evidence must decode");
        let zoom = LiteralZoomReview::decode(
            r#"{"literal_writing_present":true,"literal_evidence":[{"description":"blank geometric display enclosed and centered among ordinary mechanical hardware and lights","location":"crop 6 lower-left; original center-right","kind":"AMBIGUOUS_MARK"}],"reason":"Enclosure alone does not establish an emblem"}"#,
        )
        .expect("ambiguous zoom hardware evidence must decode");
        let merged = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("full recall allow must decode")
        .merged_zoom(zoom)
        .expect("ambiguous zoom hardware evidence must merge");
        assert_eq!(
            [full.literal_rejection(), merged.literal_rejection()],
            [None, None],
            "ordinary hardware, blank panels, or enclosure alone became symbol-or-emblem evidence"
        );
    }

    #[test]
    fn zoom_literal_evidence_merges_without_changing_semantic_recall() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("full recall allow must decode");
        let zoom = LiteralZoomReview::decode(
            r#"{"literal_writing_present":false,"literal_evidence":[{"description":"four organized pseudo-writing rows on a distant station board","location":"crop 8 lower-center; original lower-center","kind":"PSEUDO_WRITING"}],"reason":"The enlarged board contains writing-like rows"}"#,
        )
        .expect("zoom literal evidence must decode");
        let merged = full
            .merged_zoom(zoom)
            .expect("zoom literal evidence must merge");
        let archived = serde_json::to_value(&merged).expect("merged recall must serialize");
        assert_eq!(
            (
                merged.allows(),
                merged.literal_rejection().is_some(),
                archived["zoom_inspected"].as_bool(),
                archived["literal_writing_present"].as_bool(),
                archived["literal_evidence"][0]["kind"].as_str(),
            ),
            (true, true, Some(true), Some(true), Some("PSEUDO_WRITING")),
            "zoom evidence changed semantic recall, escaped literal rejection, or lost archive proof"
        );
    }

    #[test]
    fn zoom_symbol_or_emblem_overrides_a_contradictory_literal_allow() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("full recall allow must decode");
        let zoom = LiteralZoomReview::decode(
            r#"{"literal_writing_present":false,"literal_evidence":[{"description":"deliberate white V-like glyph enclosed in a black transit badge","location":"crop 6 lower-left; original center-right","kind":"SYMBOL_OR_EMBLEM"}],"reason":"The badge does not contain readable text"}"#,
        )
        .expect("grounded zoom symbol-or-emblem evidence must decode");
        let merged = full
            .merged_zoom(zoom)
            .expect("zoom symbol-or-emblem evidence must merge");
        assert_eq!(
            (merged.allows(), merged.literal_rejection().is_some()),
            (true, true),
            "zoom symbol or emblem escaped literal rejection or changed semantic recall"
        );
    }

    #[test]
    fn empty_zoom_allow_still_marks_the_scale_scan_as_inspected() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("full recall allow must decode");
        let zoom = LiteralZoomReview::decode(
            r#"{"literal_writing_present":true,"literal_evidence":[],"reason":"No literal writing appears in any enlarged crop"}"#,
        )
        .expect("empty zoom allow must decode");
        let merged = full.merged_zoom(zoom).expect("empty zoom allow must merge");
        let archived = serde_json::to_value(&merged).expect("merged recall must serialize");
        assert_eq!(
            (
                merged.allows(),
                merged.literal_rejection(),
                archived["zoom_inspected"].as_bool(),
                archived["literal_writing_present"].as_bool(),
            ),
            (true, None, Some(true), Some(false)),
            "an empty scale scan was not archived or trusted contradictory prose instead of evidence"
        );
    }

    #[test]
    fn rejecting_zoom_evidence_precedes_ambiguous_items_before_the_merge_cap() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[{"description":"uncertain fold","location":"lower panel","kind":"AMBIGUOUS_MARK"}],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("ambiguous full recall must decode");
        let zoom = LiteralZoomReview::decode(
            r#"{"literal_writing_present":false,"literal_evidence":[{"description":"uncertain rail mark","location":"crop 9 center; original lower-right","kind":"AMBIGUOUS_MARK"},{"description":"four organized rows","location":"crop 8 upper-left; original lower-center","kind":"PSEUDO_WRITING"}],"reason":"The distant board contains organized rows"}"#,
        )
        .expect("mixed zoom evidence must decode");
        let archived = serde_json::to_value(
            full.merged_zoom(zoom)
                .expect("mixed zoom evidence must merge"),
        )
        .expect("prioritized zoom evidence must serialize");
        assert_eq!(
            (
                archived["literal_evidence"][0]["kind"].as_str(),
                archived["literal_evidence"].as_array().map(Vec::len),
            ),
            (Some("PSEUDO_WRITING"), Some(3)),
            "rejecting zoom evidence was displaced by ambiguous marks before merge deduplication and capping"
        );
    }

    #[test]
    fn ambiguous_marks_and_ungrounded_reason_prose_remain_allowed() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":true,"literal_evidence":[{"description":"possible marks in cross-hatching","location":"lower panel","kind":"AMBIGUOUS_MARK"}],"reason":"Pseudo-writing might appear elsewhere"}"#,
        )
        .expect("ambiguous grounded marks must decode");
        assert_eq!(
            (review.allows(), review.literal_rejection()),
            (true, None),
            "ambiguous marks or ungrounded reason prose became prohibited literal writing"
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
        let prompt = card
            .prompt(&fidelity_scene())
            .expect("chinese recall prompt must render");
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
            card.prompt(&fidelity_scene()).is_ok()
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
        let prompt = card.prompt(&fidelity_scene());
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
    fn compact_scene_fidelity_contract_accepts_object_only_panels_without_forwarding_full_scene() {
        let card = RecallCard::new(
            ShownRecall::new("English", "A visible sentence.", "visible", "A hint."),
            HiddenRecall::new("English", "enter", "The visitor enters."),
        );
        let mut scene = fidelity_scene();
        scene["manga_panel"]["panels"][0]["scene"]["subjects"] = serde_json::json!([]);
        scene["manga_panel"]["panels"][0]["scene"]["description"] =
            serde_json::json!("PANEL_DESCRIPTION_REQUIRED_VISIBLE_CUE");
        scene["manga_panel"]["meta"] =
            serde_json::json!({"title": "FULL_SCENE_SENTINEL_MUST_NOT_BE_FORWARDED"});
        let contract = serde_json::to_string(
            &SceneFidelityContract::from_scene(&scene)
                .expect("compact scene fidelity contract must decode"),
        )
        .expect("compact scene fidelity contract must serialize");
        let prompt = card
            .prompt(&scene)
            .expect("object-only fidelity contract must render");
        assert_eq!(
            (
                prompt.contains("one visitor opens an unmarked door"),
                prompt.contains("show the visitor opening the door"),
                prompt.contains("\"subjects\": []"),
                contract.contains("PANEL_DESCRIPTION_REQUIRED_VISIBLE_CUE"),
                prompt.contains("FULL_SCENE_SENTINEL_MUST_NOT_BE_FORWARDED"),
                contract.contains("The visitor enters"),
            ),
            (true, true, true, true, false, false),
            "compact fidelity prompt lost panel description, rejected an object-only panel, or forwarded unrelated scene/card data"
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

    #[test]
    fn dedicated_broken_continuity_overrides_allow_without_changing_semantic_recall() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("clean full recall must decode");
        let fidelity = FidelityReview::decode(
            r#"{"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[{"requirement":"touchy_man must remain the same person in panels p1 and p2","observed":"p1 shows an older heavy square-faced man in a crewneck while p2 shows a younger slim soft-faced man in a collared sweater","location":"touchy_man in left p1 and listener in right p2","kind":"BROKEN_SUBJECT_CONTINUITY"}],"reason":"The required repeated subject is visibly substituted"}"#,
        )
        .expect("grounded dedicated fidelity review must decode");
        let merged = full
            .merged_fidelity(fidelity)
            .expect("dedicated fidelity evidence must merge");
        let archived = serde_json::to_value(&merged).expect("merged review must serialize");
        assert_eq!(
            (
                merged.allows(),
                merged.scene_fidelity_rejection().is_some(),
                merged.needs_zoom(),
                archived["scene_fidelity_decision"].as_str(),
                archived["scene_fidelity_evidence"][0]["kind"].as_str(),
                archived["fidelity_inspected"].as_bool(),
            ),
            (
                true,
                true,
                true,
                Some("REJECT"),
                Some("BROKEN_SUBJECT_CONTINUITY"),
                Some(true),
            ),
            "dedicated continuity evidence escaped its finding, changed semantic recall, or stopped the zoom stage"
        );
    }

    #[test]
    fn clean_dedicated_fidelity_review_enables_the_later_zoom_stage() {
        let full = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"No answer-bearing writing is visible"}"#,
        )
        .expect("clean full recall must decode");
        let fidelity = FidelityReview::decode(
            r#"{"scene_fidelity_decision":"REJECT","scene_fidelity_evidence":[],"reason":"Every required subject and relation is visible"}"#,
        )
        .expect("clean dedicated fidelity review must decode");
        let needs_fidelity = full.needs_fidelity();
        let merged = full
            .merged_fidelity(fidelity)
            .expect("clean dedicated fidelity review must merge");
        assert_eq!(
            (needs_fidelity, merged.needs_zoom()),
            (true, true),
            "clean full and dedicated reviews did not advance through the ordered gates"
        );
    }

    #[test]
    fn old_recall_sidecar_defaults_to_uninspected_fidelity() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"zoom_inspected":true,"reason":"Archived before the dedicated fidelity stage"}"#,
        )
        .expect("old recall sidecar must decode");
        let archived = serde_json::to_value(review).expect("old recall sidecar must serialize");
        assert_eq!(
            archived["fidelity_inspected"].as_bool(),
            Some(false),
            "old recall sidecar falsely claimed dedicated fidelity inspection"
        );
    }

    #[test]
    fn old_recall_sidecar_without_page_frame_reads_as_framed() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"reason":"Archived before the page-frame classification"}"#,
        )
        .expect("old recall sidecar must decode");
        assert!(
            !review.frame_borderless(),
            "a legacy sidecar without page_frame was read as a borderless verdict"
        );
    }

    #[test]
    fn borderless_page_frame_verdict_is_reported() {
        let review = RecallReview::decode(
            r#"{"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"page_frame":"BORDERLESS","reason":"No frame structure is visible"}"#,
        )
        .expect("borderless recall verdict must decode");
        assert!(
            review.frame_borderless(),
            "a judged borderless page was not reported by the review"
        );
    }
}
