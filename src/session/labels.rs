//! Closed sentence-attribution values and client-owned label constraints.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Stylistic register attributed to one generated sentence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Register {
    /// Stylistically unmarked language suitable in ordinary contexts.
    Neutral,
    /// Conversational language that may include mild slang.
    Casual,
    /// Official language with full forms and no conversational contractions.
    Formal,
    /// Bookish written language that may use imagery.
    Literary,
    /// Understandable language with an obsolete or historical sound.
    Archaic,
}

impl Register {
    /// Return the stable lowercase token used by Gemini, cache, and TUI.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Casual => "casual",
            Self::Formal => "formal",
            Self::Literary => "literary",
            Self::Archaic => "archaic",
        }
    }
}

/// Lowercase operational CEFR band for the language surrounding the target term.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SentenceLevel {
    /// One complete proposition in the most basic high-frequency surrounding language.
    A1,
    /// One familiar everyday situation with a simple explicit extension.
    #[serde(alias = "easy")]
    A2,
    /// One connected adult idea in ordinary concrete language.
    #[serde(alias = "takes practice", alias = "balanced")]
    B1,
    /// Denser nonspecialist language with a precise relation or useful collocation.
    #[serde(alias = "challenging", alias = "stretch")]
    B2,
    /// Flexible advanced language with nuanced or implicit linkage.
    C1,
    /// Layered near-native language with subtle pragmatic relationships.
    C2,
}

impl SentenceLevel {
    /// Return the stable lowercase CEFR token used by the cache and TUI.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::A1 => "a1",
            Self::A2 => "a2",
            Self::B1 => "b1",
            Self::B2 => "b2",
            Self::C1 => "c1",
            Self::C2 => "c2",
        }
    }

    /// Return the lowercase CEFR token used in the strict Gemini contract.
    #[must_use]
    pub(crate) fn prompt_token(self) -> &'static str {
        self.token()
    }
}

/// Communicative type attributed to one generated sentence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SentenceKind {
    /// A declarative statement.
    Statement,
    /// A direct question.
    Question,
    /// An instruction or polite request.
    Request,
    /// An exclamation.
    Exclamation,
    /// Two short linked utterances spoken by one voice.
    Dialogue,
}

impl SentenceKind {
    /// Return the stable lowercase token used by Gemini, cache, and TUI.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::Question => "question",
            Self::Request => "request",
            Self::Exclamation => "exclamation",
            Self::Dialogue => "dialogue",
        }
    }
}

/// One independently selectable sentence-label axis.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SentenceAxis {
    /// The stylistic register axis.
    Register,
    /// The operational CEFR band of the surrounding language.
    Level,
    /// The communicative-type axis.
    Type,
}

impl SentenceAxis {
    /// Return the stable lowercase token used in requests and model reports.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Level => "level",
            Self::Type => "type",
        }
    }
}

/// An immutable set of explicitly pinned or approximately fulfilled axes.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AxisSet(BTreeSet<SentenceAxis>);

impl AxisSet {
    /// Create a set from the supplied axes.
    #[must_use]
    pub fn from_axes(axes: impl IntoIterator<Item = SentenceAxis>) -> Self {
        Self(axes.into_iter().collect())
    }

    /// Return whether the set contains an axis.
    #[must_use]
    pub fn contains(&self, axis: SentenceAxis) -> bool {
        self.0.contains(&axis)
    }

    /// Return whether no axis is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the number of distinct axes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Return a set with one axis included.
    #[must_use]
    pub fn including(&self, axis: SentenceAxis) -> Self {
        let mut axes = self.0.clone();
        axes.insert(axis);
        Self(axes)
    }

    /// Return a set with one axis removed.
    #[must_use]
    pub fn excluding(&self, axis: SentenceAxis) -> Self {
        let mut axes = self.0.clone();
        axes.remove(&axis);
        Self(axes)
    }

    /// Return only axes shared with another set.
    #[must_use]
    pub fn intersecting(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).copied().collect())
    }

    /// Iterate over axes in stable declaration order.
    pub fn iter(&self) -> impl Iterator<Item = SentenceAxis> + '_ {
        self.0.iter().copied()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct SentenceLabelValues {
    register: Register,
    level: SentenceLevel,
    #[serde(rename = "type")]
    kind: SentenceKind,
}

impl SentenceLabelValues {
    fn new(register: Register, level: SentenceLevel, kind: SentenceKind) -> Self {
        Self {
            register,
            level,
            kind,
        }
    }
}

/// Complete attribution attached to generated card metadata.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SentenceLabels {
    #[serde(flatten)]
    values: SentenceLabelValues,
    #[serde(default)]
    pinned: AxisSet,
    #[serde(default)]
    approx: AxisSet,
    #[serde(default, skip_serializing_if = "SentenceLabelChoices::is_empty")]
    requested: SentenceLabelChoices,
}

impl SentenceLabels {
    /// Create complete sentence labels with client pins and approximation flags.
    #[must_use]
    pub fn new(
        register: Register,
        level: SentenceLevel,
        kind: SentenceKind,
        pinned: AxisSet,
        approx: AxisSet,
    ) -> Self {
        Self {
            values: SentenceLabelValues::new(register, level, kind),
            pinned,
            approx,
            requested: SentenceLabelChoices::default(),
        }
    }

    /// Return the attributed register.
    #[must_use]
    pub fn register(&self) -> Register {
        self.values.register
    }

    /// Return the attributed CEFR band of the surrounding language.
    #[must_use]
    pub fn level(&self) -> SentenceLevel {
        self.values.level
    }

    /// Return the attributed communicative type.
    #[must_use]
    pub fn kind(&self) -> SentenceKind {
        self.values.kind
    }

    /// Return the axes explicitly controlled by the learner.
    #[must_use]
    pub fn pinned(&self) -> &AxisSet {
        &self.pinned
    }

    /// Return hard axes fulfilled only as a best effort.
    #[must_use]
    pub fn approx(&self) -> &AxisSet {
        &self.approx
    }

    /// Return the stable value token for one axis.
    #[must_use]
    pub fn token(&self, axis: SentenceAxis) -> Option<&'static str> {
        match axis {
            SentenceAxis::Register => Some(self.register().token()),
            SentenceAxis::Level => Some(self.level().token()),
            SentenceAxis::Type => Some(self.kind().token()),
        }
    }

    /// Return the requested token for one pinned axis, including legacy fallback.
    #[must_use]
    pub fn requested_token(&self, axis: SentenceAxis) -> Option<&'static str> {
        if !self.pinned.contains(axis) {
            return None;
        }
        self.requested.token(axis).or_else(|| self.token(axis))
    }

    /// Return the explicitly recorded requested token for one pinned axis.
    #[must_use]
    pub fn recorded_request_token(&self, axis: SentenceAxis) -> Option<&'static str> {
        if !self.pinned.contains(axis) {
            return None;
        }
        self.requested.token(axis)
    }

    /// Return labels carrying a client-owned pin set and valid approximation subset.
    #[must_use]
    pub fn with_axis_state(self, pinned: AxisSet, approx: AxisSet) -> Self {
        let requested = self.requested.retaining(&pinned);
        Self {
            values: self.values,
            approx: approx.intersecting(&pinned),
            pinned,
            requested,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct SentenceLabelChoices {
    register: Option<Register>,
    level: Option<SentenceLevel>,
    kind: Option<SentenceKind>,
}

impl SentenceLabelChoices {
    fn from_labels(labels: &SentenceLabels) -> Self {
        Self {
            register: Some(if labels.pinned.contains(SentenceAxis::Register) {
                labels.requested.register.unwrap_or(labels.register())
            } else {
                labels.register()
            }),
            level: Some(if labels.pinned.contains(SentenceAxis::Level) {
                labels.requested.level.unwrap_or(labels.level())
            } else {
                labels.level()
            }),
            kind: Some(if labels.pinned.contains(SentenceAxis::Type) {
                labels.requested.kind.unwrap_or(labels.kind())
            } else {
                labels.kind()
            }),
        }
    }

    fn from_selection(selection: &SentenceLabelSelection) -> Self {
        Self {
            register: if selection.pinned.contains(SentenceAxis::Register) {
                selection.values.register
            } else {
                None
            },
            level: if selection.pinned.contains(SentenceAxis::Level) {
                selection.values.level
            } else {
                None
            },
            kind: if selection.pinned.contains(SentenceAxis::Type) {
                selection.values.kind
            } else {
                None
            },
        }
    }

    fn token(&self, axis: SentenceAxis) -> Option<&'static str> {
        match axis {
            SentenceAxis::Register => self.register.map(Register::token),
            SentenceAxis::Level => self.level.map(SentenceLevel::token),
            SentenceAxis::Type => self.kind.map(SentenceKind::token),
        }
    }

    fn retaining(&self, axes: &AxisSet) -> Self {
        Self {
            register: if axes.contains(SentenceAxis::Register) {
                self.register
            } else {
                None
            },
            level: if axes.contains(SentenceAxis::Level) {
                self.level
            } else {
                None
            },
            kind: if axes.contains(SentenceAxis::Type) {
                self.kind
            } else {
                None
            },
        }
    }

    fn is_empty(&self) -> bool {
        self.register.is_none() && self.level.is_none() && self.kind.is_none()
    }
}

/// Partial label values carried by an inline rewrite request.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SentenceLabelSelection {
    values: SentenceLabelChoices,
    pinned: AxisSet,
    approx: AxisSet,
}

impl SentenceLabelSelection {
    /// Create an empty selection for legacy metadata without labels.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a complete working selection from generated labels.
    #[must_use]
    pub fn from_labels(labels: &SentenceLabels) -> Self {
        Self {
            values: SentenceLabelChoices::from_labels(labels),
            pinned: labels.pinned().clone(),
            approx: labels.approx().clone(),
        }
    }

    /// Return whether all mandatory axes already have attributed values.
    #[must_use]
    pub fn attributed(&self) -> bool {
        self.values.register.is_some() && self.values.level.is_some() && self.values.kind.is_some()
    }

    /// Return the selected register when known.
    #[must_use]
    pub fn register(&self) -> Option<Register> {
        self.values.register
    }

    /// Return the selected CEFR band of the surrounding language when known.
    #[must_use]
    pub fn level(&self) -> Option<SentenceLevel> {
        self.values.level
    }

    /// Return the selected communicative type when known.
    #[must_use]
    pub fn kind(&self) -> Option<SentenceKind> {
        self.values.kind
    }

    /// Return the explicitly pinned axes.
    #[must_use]
    pub fn pinned(&self) -> &AxisSet {
        &self.pinned
    }

    /// Return approximation flags inherited from the current metadata.
    #[must_use]
    pub fn approx(&self) -> &AxisSet {
        &self.approx
    }

    /// Return the visible token selected on one axis.
    #[must_use]
    pub fn token(&self, axis: SentenceAxis) -> Option<&'static str> {
        match axis {
            SentenceAxis::Register => self.register().map(Register::token),
            SentenceAxis::Level => self.level().map(SentenceLevel::token),
            SentenceAxis::Type => self.kind().map(SentenceKind::token),
        }
    }

    /// Return the number of selectable chips on one axis.
    #[must_use]
    pub fn choice_count(&self, axis: SentenceAxis) -> usize {
        match axis {
            SentenceAxis::Register => 5,
            SentenceAxis::Level => 6,
            SentenceAxis::Type => 5,
        }
    }

    /// Return the token of one chip on an axis.
    #[must_use]
    pub fn choice_token(&self, axis: SentenceAxis, index: usize) -> Option<&'static str> {
        match (axis, index) {
            (SentenceAxis::Register, 0) => Some("neutral"),
            (SentenceAxis::Register, 1) => Some("casual"),
            (SentenceAxis::Register, 2) => Some("formal"),
            (SentenceAxis::Register, 3) => Some("literary"),
            (SentenceAxis::Register, 4) => Some("archaic"),
            (SentenceAxis::Level, 0) => Some("a1"),
            (SentenceAxis::Level, 1) => Some("a2"),
            (SentenceAxis::Level, 2) => Some("b1"),
            (SentenceAxis::Level, 3) => Some("b2"),
            (SentenceAxis::Level, 4) => Some("c1"),
            (SentenceAxis::Level, 5) => Some("c2"),
            (SentenceAxis::Type, 0) => Some("statement"),
            (SentenceAxis::Type, 1) => Some("question"),
            (SentenceAxis::Type, 2) => Some("request"),
            (SentenceAxis::Type, 3) => Some("exclamation"),
            (SentenceAxis::Type, 4) => Some("dialogue"),
            _ => None,
        }
    }

    /// Return a selection with one chip chosen and the corresponding pin updated.
    #[must_use]
    pub fn choosing(&self, axis: SentenceAxis, index: usize) -> Self {
        if self.choice_token(axis, index).is_none() {
            return self.clone();
        }
        let mut next = self.clone();
        match (axis, index) {
            (SentenceAxis::Register, 0) => next.values.register = Some(Register::Neutral),
            (SentenceAxis::Register, 1) => next.values.register = Some(Register::Casual),
            (SentenceAxis::Register, 2) => next.values.register = Some(Register::Formal),
            (SentenceAxis::Register, 3) => next.values.register = Some(Register::Literary),
            (SentenceAxis::Register, 4) => next.values.register = Some(Register::Archaic),
            (SentenceAxis::Level, 0) => next.values.level = Some(SentenceLevel::A1),
            (SentenceAxis::Level, 1) => next.values.level = Some(SentenceLevel::A2),
            (SentenceAxis::Level, 2) => next.values.level = Some(SentenceLevel::B1),
            (SentenceAxis::Level, 3) => next.values.level = Some(SentenceLevel::B2),
            (SentenceAxis::Level, 4) => next.values.level = Some(SentenceLevel::C1),
            (SentenceAxis::Level, 5) => next.values.level = Some(SentenceLevel::C2),
            (SentenceAxis::Type, 0) => next.values.kind = Some(SentenceKind::Statement),
            (SentenceAxis::Type, 1) => next.values.kind = Some(SentenceKind::Question),
            (SentenceAxis::Type, 2) => next.values.kind = Some(SentenceKind::Request),
            (SentenceAxis::Type, 3) => next.values.kind = Some(SentenceKind::Exclamation),
            (SentenceAxis::Type, 4) => next.values.kind = Some(SentenceKind::Dialogue),
            _ => return self.clone(),
        }
        next.approx = next.approx.excluding(axis);
        next.pinned = next.pinned.including(axis);
        next
    }

    /// Return a selection with one axis restored to its generated baseline state.
    #[must_use]
    pub fn restoring(&self, axis: SentenceAxis, baseline: &Self) -> Self {
        let mut next = self.clone();
        match axis {
            SentenceAxis::Register => next.values.register = baseline.values.register,
            SentenceAxis::Level => next.values.level = baseline.values.level,
            SentenceAxis::Type => next.values.kind = baseline.values.kind,
        }
        next.pinned = if baseline.pinned.contains(axis) {
            next.pinned.including(axis)
        } else {
            next.pinned.excluding(axis)
        };
        next.approx = if baseline.approx.contains(axis) {
            next.approx.including(axis)
        } else {
            next.approx.excluding(axis)
        };
        next
    }

    /// Return a selection moved to the adjacent chip on one axis.
    #[must_use]
    pub fn advanced(&self, axis: SentenceAxis, forward: bool) -> Self {
        let count = self.choice_count(axis);
        let current = self.token(axis).and_then(|token| {
            (0..count).find(|index| self.choice_token(axis, *index) == Some(token))
        });
        let index = match (current, forward) {
            (Some(index), true) => index.saturating_add(1).min(count.saturating_sub(1)),
            (Some(index), false) => index.saturating_sub(1),
            (None, true) => 0,
            (None, false) => count.saturating_sub(1),
        };
        self.choosing(axis, index)
    }

    /// Return whether a complete response preserves the requested preset.
    #[must_use]
    pub fn accepts(&self, labels: &SentenceLabels) -> bool {
        [
            SentenceAxis::Register,
            SentenceAxis::Level,
            SentenceAxis::Type,
        ]
        .into_iter()
        .all(|axis| {
            self.token(axis).is_none_or(|requested| {
                labels.token(axis) == Some(requested)
                    || self.pinned.contains(axis) && labels.approx().contains(axis)
            })
        })
    }

    /// Return response labels with actual values and requested targets retained.
    #[must_use]
    pub fn reconciled(&self, labels: SentenceLabels) -> SentenceLabels {
        let approx = labels.approx.intersecting(&self.pinned);
        SentenceLabels {
            values: labels.values,
            pinned: self.pinned.clone(),
            approx,
            requested: SentenceLabelChoices::from_selection(self),
        }
    }
}
