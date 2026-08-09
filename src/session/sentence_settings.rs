//! Batch-level sentence preferences expanded into per-card label requests.

use serde::{Deserialize, Serialize};

use super::{SentenceAxis, SentenceKind, SentenceLabelSelection, SentenceLevel};

const TYPE_MIX_SEED: u64 = 0x6b61_6d69_7368_6962;
const TYPE_MIX_BAG: [SentenceKind; 5] = [
    SentenceKind::Statement,
    SentenceKind::Statement,
    SentenceKind::Statement,
    SentenceKind::Question,
    SentenceKind::Dialogue,
];

/// Batch policy for assigning communicative sentence types.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SentenceTypeMix {
    /// Let Gemini choose and describe the natural sentence type.
    #[default]
    Natural,
    /// Pin a deterministic weighted mix of statements, questions, and dialogues.
    Varied,
}

impl SentenceTypeMix {
    /// Return the stable lowercase token used by the TUI, CLI, and session JSON.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Varied => "varied",
        }
    }
}

/// Optional sentence preferences chosen once for a reviewed batch.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SentenceBatchSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<SentenceLevel>,
    #[serde(default)]
    types: SentenceTypeMix,
}

impl SentenceBatchSettings {
    /// Create one batch preference from an optional level and type-mix policy.
    #[must_use]
    pub fn new(level: Option<SentenceLevel>, types: SentenceTypeMix) -> Self {
        Self { level, types }
    }

    /// Return the explicitly requested surrounding-language level.
    #[must_use]
    pub fn level(self) -> Option<SentenceLevel> {
        self.level
    }

    /// Return the communicative-type allocation policy.
    #[must_use]
    pub fn types(self) -> SentenceTypeMix {
        self.types
    }

    /// Return a copy carrying a different optional level.
    #[must_use]
    pub fn with_level(self, level: Option<SentenceLevel>) -> Self {
        Self { level, ..self }
    }

    /// Return a copy carrying a different communicative-type policy.
    #[must_use]
    pub fn with_types(self, types: SentenceTypeMix) -> Self {
        Self { types, ..self }
    }

    /// Expand the batch preference into one optional pinned request per card.
    #[must_use]
    pub fn selections(self, count: usize) -> Vec<Option<SentenceLabelSelection>> {
        (0..count).map(|index| self.selection(index)).collect()
    }

    fn selection(self, index: usize) -> Option<SentenceLabelSelection> {
        let selection = self
            .level
            .map(level_index)
            .map(|choice| SentenceLabelSelection::empty().choosing(SentenceAxis::Level, choice))
            .unwrap_or_default();
        let selection = match self.types {
            SentenceTypeMix::Natural => selection,
            SentenceTypeMix::Varied => {
                selection.choosing(SentenceAxis::Type, kind_index(varied_kind(index)))
            }
        };
        (!selection.pinned().is_empty()).then_some(selection)
    }
}

fn varied_kind(index: usize) -> SentenceKind {
    let cycle = u64::try_from(index / TYPE_MIX_BAG.len()).unwrap_or(u64::MAX);
    let mixed = seeded(cycle ^ TYPE_MIX_SEED);
    let offset = usize::try_from(mixed % 5).unwrap_or(0);
    let stride = [1, 2, 3, 4][usize::try_from(mixed / 5 % 4).unwrap_or(0)];
    TYPE_MIX_BAG[(offset + index % TYPE_MIX_BAG.len() * stride) % TYPE_MIX_BAG.len()]
}

fn seeded(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ value >> 31
}

fn level_index(level: SentenceLevel) -> usize {
    match level {
        SentenceLevel::A1 => 0,
        SentenceLevel::A2 => 1,
        SentenceLevel::B1 => 2,
        SentenceLevel::B2 => 3,
        SentenceLevel::C1 => 4,
        SentenceLevel::C2 => 5,
    }
}

fn kind_index(kind: SentenceKind) -> usize {
    match kind {
        SentenceKind::Statement => 0,
        SentenceKind::Question => 1,
        SentenceKind::Dialogue => 4,
        SentenceKind::Request | SentenceKind::Exclamation => {
            panic!("varied sentence type must be statement, question, or dialogue")
        }
    }
}
