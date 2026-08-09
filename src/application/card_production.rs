//! Application port for producing the cached artifacts of one card.

use anyhow::Result;

use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, CardRevision, GenerationCost,
    LanguagePair, SentenceLabelSelection,
};

/// Generate the rich metadata consumed by all card artifacts.
pub trait CardMetaGeneration {
    /// Produce metadata for one term and selected understanding.
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<CardMeta>;
}

/// Revise one card from a learner correction.
pub trait CardCorrection {
    /// Apply one comment and return the revised card payload.
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision>;

    /// Apply one comment and return the exact cost of the provider call.
    fn correct_card_accounted(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        ArtifactAttempt::unmetered(self.correct_card(draft, comment, pair))
    }
}

/// Records provider spend before an artifact is settled downstream.
pub(crate) trait GenerationCostLedger: Send + Sync {
    /// Charge one provider delta to a stable card slot.
    fn charge(&self, slot: usize, artifact: Artifact, delta: GenerationCost) -> Result<()>;
}

/// Produce metadata, sound, scene, and picture artifacts for cards.
pub(crate) trait CardProduction:
    CardMetaGeneration + CardCorrection + Clone + Send + 'static
{
    /// Generate metadata attributed to one stable card slot.
    fn generate_meta_in(
        &self,
        slot: usize,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)>;
    /// Generate or rewrite metadata for the complete draft at one stable slot.
    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        let term = draft.term().to_string();
        let understanding = draft.understanding().to_string();
        self.generate_meta_in(
            slot,
            draft.term(),
            draft.understanding(),
            draft.pair(),
            draft.meta_request(),
        )
        .map(|(meta, file)| (CardRevision::new(term, understanding, meta), file))
    }
    /// Generate a scene attributed to one stable card slot.
    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile>;
    /// Generate a picture attributed to one stable card slot.
    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile>;
    /// Generate sound attributed to one stable card slot.
    fn generate_sound_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile>;
    /// Persist supplied metadata under the stable card identity.
    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile>;
}
