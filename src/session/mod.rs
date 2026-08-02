//! Word-first session contracts: language pair, target detection, and batch state.

mod attempt;
mod bridge;
mod cache;
mod candidate;
mod cost;
mod detection;
mod draft;
mod engine;
mod labels;
mod pair;
mod pass;
mod state;
mod vault;

#[doc(inline)]
pub use crate::application::{
    BulkCorrection, CardCorrection, CardMetaGeneration, LearningTarget, Understanding,
};
pub(crate) use attempt::ARTIFACT_ATTEMPT_CEILING;
pub use attempt::{AttemptFault, AttemptLog, AttemptTally};
pub use bridge::{drafts_from_document, from_entry, to_document, to_entry};
pub(crate) use cache::CandidateRecord;
pub use cache::{CachedUnderstanding, CardMetaCache};
pub use candidate::{RawInputBatch, Sense, WordCandidate};
pub use cost::{CostRecord, GenerationCost};
pub use detection::{LearningDetection, LearningGuess, ScriptDetection};
pub use draft::{
    Artifact, ArtifactAttempt, ArtifactCosts, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft,
    CardMeta, CardRewrite,
};
pub use engine::{EngineEvent, SessionEngine};
pub use labels::{
    AxisSet, Register, SentenceAxis, SentenceKind, SentenceLabelSelection, SentenceLabels,
    SentenceLevel,
};
pub use pair::LanguagePair;
pub use pass::{CardRevision, SenseCorrection, Understood};
pub use state::SessionState;
pub use vault::CardCell;

/// Convenience re-export so CLI callers do not need a separate `languages` import.
pub fn catalog_for_detection() -> crate::languages::LanguageCatalog {
    crate::languages::catalog()
}
