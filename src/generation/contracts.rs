use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::generation::manga::Progress as SceneProgress;
use crate::vocabulary::VocabularyEntry;

/// Translate scenes with one prompt language bound to one client.
pub trait SceneSource {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value>;
}

/// Expose one audio generator to the pipeline.
pub trait SpeechGenerator {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)>;
    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

/// Expose one illustration generator to the pipeline.
pub trait IllustrationGenerator {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)>;
    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

/// Resolve audio and illustration generators for one entry.
pub trait GeneratorSource {
    /// The returned audio service type.
    type Audio: SpeechGenerator;
    /// The returned illustration service type.
    type Illustration: IllustrationGenerator;
    /// Return the audio service for one entry.
    fn audio(&mut self, entry: &VocabularyEntry) -> Result<Self::Audio>;
    /// Return the illustration service for one entry.
    fn illustration(&mut self, entry: &VocabularyEntry) -> Result<Self::Illustration>;
}

/// Record pipeline-level progress events and failures.
pub trait BuildProgress: SceneProgress {
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
    fn add(&mut self, entry: &VocabularyEntry, audio: &str, image: &str);
}

/// Record one skipped entry and its skip reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedCard {
    /// The skipped entry word.
    pub word: String,
    /// The captured skip reason.
    pub reason: String,
}

impl SkippedCard {
    /// Create one recorded skipped card.
    pub fn new(word: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            reason: reason.into(),
        }
    }
}
