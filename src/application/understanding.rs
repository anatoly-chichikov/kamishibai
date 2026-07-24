//! Application ports for turning raw words into curatable candidates.

use anyhow::Result;

use crate::session::{LanguagePair, RawInputBatch, SenseCorrection, Understood, WordCandidate};

/// Understand one raw input batch before the learner curates it.
pub trait Understanding {
    /// Normalise a raw batch into candidate senses and a learning-language guess.
    fn understand(&self, raw: &RawInputBatch, known: &str) -> Result<Understood>;
}

/// Refine the senses of one candidate from a learner correction.
pub trait BulkCorrection {
    /// Apply one comment to the focused candidate and return additional senses.
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection>;
}

/// Understand words and refine the candidate senses selected by the learner.
pub(crate) trait WordUnderstanding:
    Understanding + BulkCorrection + Clone + Send + 'static
{
}

impl<T> WordUnderstanding for T where T: Understanding + BulkCorrection + Clone + Send + 'static {}
