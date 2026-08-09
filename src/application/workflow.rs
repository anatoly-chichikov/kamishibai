//! Composition of the three stages in the card workflow.

use anyhow::Result;

use super::{
    BulkCorrection, CardCorrection, CardMetaGeneration, CardProduction, LearningTarget,
    PublishProgress, PublishedStudyPackage, StudyPublishing, Understanding, WordUnderstanding,
};
use crate::session::{
    ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, CardRevision, LanguagePair, RawInputBatch,
    SenseCorrection, SentenceLabelSelection, Understood, WordCandidate,
};

/// Full set of card use cases required by interactive and console surfaces.
pub(crate) trait CardUseCases: WordUnderstanding + CardProduction + StudyPublishing {}

impl<T> CardUseCases for T where T: WordUnderstanding + CardProduction + StudyPublishing {}

/// Delegates each use case to one independently testable capability.
#[derive(Clone)]
pub(crate) struct CardWorkflow<U, P, S> {
    understanding: U,
    production: P,
    publishing: S,
}

impl<U, P, S> CardWorkflow<U, P, S> {
    /// Compose understanding, production, and publishing.
    #[must_use]
    pub(crate) fn new(understanding: U, production: P, publishing: S) -> Self {
        Self {
            understanding,
            production,
            publishing,
        }
    }
}

impl<U, P, S> Understanding for CardWorkflow<U, P, S>
where
    U: Understanding,
{
    fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood> {
        self.understanding.understand(raw, known, target)
    }
}

impl<U, P, S> BulkCorrection for CardWorkflow<U, P, S>
where
    U: BulkCorrection,
{
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection> {
        self.understanding.correct_bulk(candidate, comment, pair)
    }
}

impl<U, P, S> CardMetaGeneration for CardWorkflow<U, P, S>
where
    P: CardMetaGeneration,
{
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<CardMeta> {
        self.production
            .generate_card_meta(term, understanding, pair, request)
    }
}

impl<U, P, S> CardCorrection for CardWorkflow<U, P, S>
where
    P: CardCorrection,
{
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.production.correct_card(draft, comment, pair)
    }

    fn correct_card_accounted(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        self.production.correct_card_accounted(draft, comment, pair)
    }
}

impl<U, P, S> CardProduction for CardWorkflow<U, P, S>
where
    U: Clone + Send + 'static,
    P: CardProduction,
    S: Clone + Send + 'static,
{
    fn generate_meta_in(
        &self,
        slot: usize,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        self.production
            .generate_meta_in(slot, term, understanding, pair, request)
    }

    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        self.production.generate_draft_meta_in(slot, draft)
    }

    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_scene_in(slot, draft)
    }

    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_picture_in(slot, draft)
    }

    fn generate_sound_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_sound_in(slot, draft)
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.production
            .store_card_meta(term, understanding, pair, meta)
    }
}

impl<U, P, S> StudyPublishing for CardWorkflow<U, P, S>
where
    U: Clone + Send + 'static,
    P: Clone + Send + 'static,
    S: StudyPublishing,
{
    fn publish(
        &self,
        drafts: &[CardDraft],
        progress: &dyn PublishProgress,
    ) -> Result<PublishedStudyPackage> {
        self.publishing.publish(drafts, progress)
    }
}
