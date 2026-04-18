//! Word-first session contracts: language pair, target detection, and batch state.

mod bridge;
mod candidate;
mod detection;
mod draft;
mod pair;
mod pass;
mod state;

pub use bridge::{to_document, to_entry};
pub use candidate::{CandidateKind, RawInputBatch, WordCandidate};
pub use detection::{ScriptDetection, TargetDetection, TargetGuess};
pub use draft::{Artifact, ArtifactSlot, AttemptTally, CardArtifacts, CardDraft, CardPayload};
pub use pair::LanguagePair;
pub use pass::{BulkCorrection, CardCorrection, Understanding, Understood};
pub use state::SessionState;
