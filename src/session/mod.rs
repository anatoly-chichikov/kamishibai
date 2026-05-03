//! Word-first session contracts: language pair, target detection, and batch state.

mod bridge;
mod candidate;
mod detection;
mod draft;
mod engine;
mod pair;
mod pass;
mod state;

pub use bridge::{to_document, to_entry};
pub use candidate::{RawInputBatch, WordCandidate};
pub use detection::{ScriptDetection, TargetDetection, TargetGuess};
pub use draft::{
    Artifact, ArtifactFile, ArtifactSlot, AttemptTally, CardArtifacts, CardBody, CardDraft,
};
pub use engine::{EngineEvent, SessionEngine};
pub use pair::LanguagePair;
pub use pass::{
    BulkCorrection, CardBodyGeneration, CardCorrection, CardRevision, Understanding, Understood,
};
pub use state::SessionState;

/// Convenience re-export so CLI callers do not need a separate `languages` import.
pub fn catalog_for_detection() -> crate::languages::LanguageCatalog {
    crate::languages::catalog()
}
