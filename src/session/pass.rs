use anyhow::Result;

use super::candidate::{RawInputBatch, WordCandidate};
use super::detection::TargetGuess;
use super::draft::{CardBody, CardDraft};
use super::pair::LanguagePair;

/// The outcome of the cheap first-pass understanding step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Understood {
    guess: TargetGuess,
    candidates: Vec<WordCandidate>,
}

impl Understood {
    /// Create one understanding result.
    pub fn new(guess: TargetGuess, candidates: Vec<WordCandidate>) -> Self {
        Self { guess, candidates }
    }

    /// Return the detected target guess.
    pub fn guess(&self) -> &TargetGuess {
        &self.guess
    }

    /// Return the candidates produced from the blob.
    pub fn candidates(&self) -> &[WordCandidate] {
        self.candidates.as_slice()
    }
}

/// Contract for the cheap human-in-the-loop understanding pass. Real implementation lives in `src/gemini/*`.
pub trait Understanding {
    /// Normalise a raw blob into reviewed rows plus the detected target language.
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood>;
}

/// Contract for the bulk correction pass fired from `Change something`.
pub trait BulkCorrection {
    /// Apply one comment to the whole candidate list.
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>>;
}

/// Contract for the rich Pro-tier card body generation pass.
///
/// Run once per draft right after the user confirms `what i understood`.
/// Produces the full `CardBody` consumed by scene/picture/sound and by the
/// `VocabularyEntry` bridge.
pub trait CardBodyGeneration {
    /// Produce one rich card body for one term plus its understanding.
    fn generate_card_body(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardBody>;
}

/// Outcome of the per-card correction pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardRevision {
    term: String,
    understanding: String,
    body: CardBody,
}

impl CardRevision {
    /// Create one card revision.
    pub fn new(term: impl Into<String>, understanding: impl Into<String>, body: CardBody) -> Self {
        Self {
            term: term.into(),
            understanding: understanding.into(),
            body,
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

    /// Return the rebuilt rich body.
    pub fn body(&self) -> &CardBody {
        &self.body
    }

    /// Consume the revision and return its parts.
    pub fn into_parts(self) -> (String, String, CardBody) {
        (self.term, self.understanding, self.body)
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
