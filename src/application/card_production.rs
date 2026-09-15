//! Application port for producing the cached artifacts of one card.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, CardRevision, CostRecord,
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

/// Identifies the workflow operation that incurred a provider request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum GenerationScope {
    /// Understand input before committed card slots exist.
    Intake,
    /// Refine candidate senses before card generation.
    Senses,
    /// Produce or rewrite one artifact, with a stable slot when assigned.
    Card {
        /// Position in the committed batch, absent for standalone card operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        slot: Option<usize>,
        /// Artifact to which this provider request belongs.
        artifact: Artifact,
    },
}

/// Records provider requests and usage before downstream decoding or settlement.
pub trait GenerationCostLedger: Send + Sync {
    /// Persist a request record, including counters when its price is unknown.
    fn record(&self, scope: GenerationScope, usage: &CostRecord) -> Result<()>;
}

/// Produce metadata, sound, scene, and picture artifacts for cards.
///
/// Implement this port to supply another provider or prompt policy. Each method
/// performs one blocking attempt; `GenerationRun` owns scheduling order and retries.
/// Returned files belong to the caller's configured storage, and attempt costs are
/// incremental provider spend rather than cumulative totals.
pub trait CardProduction {
    /// Generate metadata for a plain single-sense card at one stable slot.
    ///
    /// This convenience call constructs a draft and uses the complete-draft
    /// operation. Call `generate_draft_meta_in` when reviewed context already exists.
    fn generate_meta_in(
        &self,
        slot: usize,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        let draft = CardDraft::new(term, understanding, pair.clone());
        let draft = match request {
            Some(request) => draft.requesting_meta(request.clone()),
            None => draft,
        };
        self.generate_draft_meta_in(slot, &draft)
            .map(|(revision, file)| (revision.into_parts().2, file))
    }
    /// Generate or rewrite metadata for the complete draft at one stable slot.
    ///
    /// Read the full reviewed sense list, tags, original priorities, and pending
    /// label request from `draft`. An active rewrite also carries its note and
    /// previous metadata; implement that operation or explicitly return an error.
    /// There is no scalar fallback that can silently discard this context.
    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)>;
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
