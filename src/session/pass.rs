use anyhow::Result;

use super::candidate::{RawInputBatch, Sense, WordCandidate};
use super::detection::LearningGuess;
use super::draft::{CardDraft, CardMeta};
use super::pair::LanguagePair;

/// The outcome of the cheap first-pass understanding step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Understood {
    guess: LearningGuess,
    candidates: Vec<WordCandidate>,
}

impl Understood {
    /// Create one understanding result.
    pub fn new(guess: LearningGuess, candidates: Vec<WordCandidate>) -> Self {
        Self { guess, candidates }
    }

    /// Return the detected learning guess.
    pub fn guess(&self) -> &LearningGuess {
        &self.guess
    }

    /// Return the candidates produced from the blob.
    pub fn candidates(&self) -> &[WordCandidate] {
        self.candidates.as_slice()
    }
}

/// Contract for the cheap human-in-the-loop understanding pass. Real implementation lives in `src/gemini/*`.
pub trait Understanding {
    /// Normalise a raw blob into reviewed rows plus the detected learning language.
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood>;
}

/// Outcome of the focused add-more sense request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SenseCorrection {
    senses: Vec<Sense>,
    message: Option<String>,
}

impl SenseCorrection {
    /// Create one sense correction result.
    pub fn new(senses: Vec<Sense>, message: Option<String>) -> Self {
        Self { senses, message }
    }

    /// Create a result that adds senses.
    pub fn adding(senses: Vec<Sense>) -> Self {
        Self::new(senses, None)
    }

    /// Create a result that only carries an on-screen message.
    pub fn message(message: impl Into<String>) -> Self {
        Self::new(Vec::new(), Some(message.into()))
    }

    /// Return the newly suggested senses.
    pub fn senses(&self) -> &[Sense] {
        self.senses.as_slice()
    }

    /// Return the optional short on-screen message.
    pub fn message_text(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Consume the result into new senses and an optional message.
    pub fn into_parts(self) -> (Vec<Sense>, Option<String>) {
        (self.senses, self.message)
    }
}

/// Contract for the focused sense request fired from add more.
pub trait BulkCorrection {
    /// Apply one comment to the focused candidate and return new senses.
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection>;
}

/// Contract for the rich Gemini card meta generation pass.
///
/// Run once per draft right after the user confirms `what i understood`.
/// Produces the full `CardMeta` consumed by scene/picture/sound and by the
/// `VocabularyEntry` bridge.
pub trait CardMetaGeneration {
    /// Produce one rich card meta for one term plus its understanding.
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardMeta>;
}

/// Outcome of the per-card correction pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardRevision {
    term: String,
    understanding: String,
    meta: CardMeta,
}

impl CardRevision {
    /// Create one card revision.
    pub fn new(term: impl Into<String>, understanding: impl Into<String>, meta: CardMeta) -> Self {
        Self {
            term: term.into(),
            understanding: understanding.into(),
            meta,
        }
    }

    /// Return the (possibly revised) term.
    pub fn term(&self) -> &str {
        self.term.as_str()
    }

    /// Return the (possibly revised) understanding.
    pub fn understanding(&self) -> &str {
        self.understanding.as_str()
    }

    /// Return the rebuilt rich meta.
    pub fn meta(&self) -> &CardMeta {
        &self.meta
    }

    /// Consume the revision and return its parts.
    pub fn into_parts(self) -> (String, String, CardMeta) {
        (self.term, self.understanding, self.meta)
    }
}

/// Contract for the per-card correction pass fired from `Change this card`.
pub trait CardCorrection {
    /// Apply one comment to a single draft, returning the revised payload.
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision>;
}
