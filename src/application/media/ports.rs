use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::domain::entry::NormalizedEntry;
use crate::infrastructure::scene::Progress as SceneProgress;

/// Translate scenes with one prompt language bound to one client.
pub trait SceneSource {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value>;
}

/// Expose one audio generator to the pipeline.
pub trait AudioService {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)>;
    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

/// Expose one illustration generator to the pipeline.
pub trait IllustrationService {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        word: &str,
        target: &str,
        progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)>;
    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

/// Resolve audio and illustration generators for one entry.
pub trait MediaSource {
    /// The returned audio service type.
    type Audio: AudioService;
    /// The returned illustration service type.
    type Illustration: IllustrationService;
    /// Return the audio service for one entry.
    fn audio(&mut self, entry: &NormalizedEntry) -> Result<Self::Audio>;
    /// Return the illustration service for one entry.
    fn illustration(&mut self, entry: &NormalizedEntry) -> Result<Self::Illustration>;
}

/// Record pipeline-level progress events and failures.
pub trait PipelineProgress: SceneProgress {
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str);
    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str);
}

/// Attach generated media and add one note.
pub trait Deck {
    /// Attach one media file path.
    fn attach(&mut self, path: &Path);
    /// Add one note to the deck.
    fn add(&mut self, entry: &NormalizedEntry, audio: &str, image: &str);
}

/// Record one skipped entry and its failure reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    /// The skipped entry word.
    pub word: String,
    /// The captured failure reason.
    pub reason: String,
}

impl Failure {
    /// Create one recorded pipeline failure.
    pub fn new(word: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            reason: reason.into(),
        }
    }
}
