use super::candidate::{RawInputBatch, WordCandidate};
use super::draft::CardDraft;
use super::pair::LanguagePair;

/// The portion of session state that flows between screens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    pair: LanguagePair,
    raw: RawInputBatch,
    confirmed: Vec<WordCandidate>,
    drafts: Vec<CardDraft>,
}

impl SessionState {
    /// Create a fresh session for a batch that has just arrived at `YourWords`.
    pub fn starting(pair: LanguagePair, raw: RawInputBatch) -> Self {
        Self {
            pair,
            raw,
            confirmed: Vec::new(),
            drafts: Vec::new(),
        }
    }

    /// Return the batch language pair.
    pub fn pair(&self) -> &LanguagePair {
        &self.pair
    }

    /// Return the raw input the user pasted.
    pub fn raw(&self) -> &RawInputBatch {
        &self.raw
    }

    /// Return the confirmed candidates after the understanding pass.
    pub fn confirmed(&self) -> &[WordCandidate] {
        self.confirmed.as_slice()
    }

    /// Return the current card drafts.
    pub fn drafts(&self) -> &[CardDraft] {
        self.drafts.as_slice()
    }

    /// Return the session after a new candidate list has been accepted.
    pub fn confirming(mut self, candidates: Vec<WordCandidate>) -> Self {
        self.confirmed = candidates;
        self.drafts.clear();
        self
    }

    /// Return the session after card drafts have been produced.
    pub fn producing(mut self, drafts: Vec<CardDraft>) -> Self {
        self.drafts = drafts;
        self
    }

    /// Return the session with a different language pair (target or my language change).
    pub fn reframed(mut self, pair: LanguagePair) -> Self {
        self.pair = pair;
        self
    }
}
