//! Application ports for turning raw words into curatable candidates.

use anyhow::Result;
use thiserror::Error;

use crate::languages::LanguageCode;
use crate::session::{
    CardDraft, LanguagePair, RawInputBatch, SenseCorrection, Understood, WordCandidate,
};

/// Ask for a destination when the input cannot establish the learning language.
#[derive(Debug, Error)]
#[error("Which language would you like to translate these words into?")]
pub struct LearningLanguageRequired;

/// How the understanding pass chooses the language being learned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LearningTarget {
    /// Detect one target language from the raw batch.
    Detect,
    /// Require the supplied supported language without detecting another.
    Explicit(LanguageCode),
}

/// Understand one raw input batch before the learner curates it.
pub trait Understanding {
    /// Normalise a raw batch under one required target-language policy.
    fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood>;
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

/// Create canonical drafts for selected senses while excluding rejected candidates.
/// Experimental embedding API; compatibility is not guaranteed.
#[must_use]
pub fn drafts_for(candidates: &[WordCandidate], pair: &LanguagePair) -> Vec<CardDraft> {
    candidates
        .iter()
        .filter(|candidate| candidate.ok())
        .flat_map(|candidate| {
            let indices: Vec<usize> = candidate.selected_senses().to_vec();
            indices.into_iter().filter_map(move |index| {
                candidate
                    .senses()
                    .get(index)
                    .map(|_| CardDraft::from_candidate(candidate, index, pair.clone()))
            })
        })
        .collect()
}
