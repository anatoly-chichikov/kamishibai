use anyhow::Result;

use super::candidate::{RawInputBatch, WordCandidate};
use super::detection::TargetGuess;
use super::draft::CardDraft;
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

/// Contract for the cheap understanding pass. Real implementation lives in `src/gemini/*`.
pub trait Understanding {
    /// Normalise a raw blob into candidate rows plus the detected target language.
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

/// Contract for the per-card correction pass fired from `Change this card`.
pub trait CardCorrection {
    /// Apply one comment to a single draft.
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardDraft>;
}
