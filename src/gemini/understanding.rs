//! Gemini implementation of the word-understanding use case.

use std::path::PathBuf;

use anyhow::Result;

use super::GeminiAccess;
use crate::application::{BulkCorrection, Understanding};
use crate::session::{
    CachedUnderstanding, LanguagePair, RawInputBatch, SenseCorrection, Understood, WordCandidate,
};

/// Understands words through Gemini while reusing the understanding cache.
#[derive(Clone, Debug)]
pub(crate) struct GeminiUnderstanding {
    access: GeminiAccess,
    cache: PathBuf,
}

impl GeminiUnderstanding {
    /// Bind Gemini access to the shared cache root.
    #[must_use]
    pub(crate) fn new(access: GeminiAccess, cache: PathBuf) -> Self {
        Self { access, cache }
    }
}

impl Understanding for GeminiUnderstanding {
    fn understand(&self, raw: &RawInputBatch, known: &str) -> Result<Understood> {
        CachedUnderstanding::new(self.access.client()?, self.cache.clone()).understand(raw, known)
    }
}

impl BulkCorrection for GeminiUnderstanding {
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection> {
        self.access.client()?.correct_bulk(candidate, comment, pair)
    }
}
