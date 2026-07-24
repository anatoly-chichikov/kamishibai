use super::candidate::{Sense, WordCandidate};
use super::detection::LearningGuess;
use super::draft::CardMeta;

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
