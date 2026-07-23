//! Word-first session contracts: language pair, target detection, and batch state.

mod bridge;
mod cache;
mod candidate;
mod cost;
mod detection;
mod draft;
mod engine;
mod pair;
mod pass;
mod state;
mod vault;

pub use bridge::{drafts_from_document, from_entry, to_document, to_entry};
pub(crate) use cache::CandidateRecord;
pub use cache::{CachedUnderstanding, CardMetaCache};
pub use candidate::{RawInputBatch, Sense, WordCandidate};
pub use cost::{CostRecord, GenerationCost};
pub use detection::{LearningDetection, LearningGuess, ScriptDetection};
pub(crate) use draft::ARTIFACT_ATTEMPT_CEILING;
pub use draft::{
    Artifact, ArtifactAttempt, ArtifactCosts, ArtifactFile, ArtifactSlot, AttemptTally,
    CardArtifacts, CardDraft, CardMeta,
};
pub use engine::{EngineEvent, SessionEngine};
pub use pair::LanguagePair;
pub use pass::{
    BulkCorrection, CardCorrection, CardMetaGeneration, CardRevision, SenseCorrection,
    Understanding, Understood,
};
pub use state::SessionState;
pub use vault::CardCell;

/// Convenience re-export so CLI callers do not need a separate `languages` import.
pub fn catalog_for_detection() -> crate::languages::LanguageCatalog {
    crate::languages::catalog()
}
