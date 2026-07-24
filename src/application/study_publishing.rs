//! Application port for publishing completed cards as study artifacts.

use anyhow::Result;

use crate::session::CardDraft;

/// Paths created by publishing one completed study package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublishedStudyPackage {
    deck: String,
    report: String,
    directory: String,
}

impl PublishedStudyPackage {
    /// Create one published package from its learner-facing paths.
    #[must_use]
    pub(crate) fn new(deck: String, report: String, directory: String) -> Self {
        Self {
            deck,
            report,
            directory,
        }
    }

    /// Consume the package into the paths expected by delivery surfaces.
    #[must_use]
    pub(crate) fn into_paths(self) -> (String, String, String) {
        (self.deck, self.report, self.directory)
    }
}

/// The two stages of publishing a study package.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublishPhase {
    /// The Anki deck is being written.
    Deck,
    /// The printable report is being written.
    Report,
}

/// Receives publishing phase changes.
pub(crate) trait PublishProgress {
    /// Announce that publishing advanced to `phase`.
    fn advance(&self, phase: PublishPhase);
}

/// Publish completed cards as an Anki deck and printable report.
pub(crate) trait StudyPublishing: Clone + Send + 'static {
    /// Publish the completed subset as one named study package.
    fn publish(
        &self,
        drafts: &[CardDraft],
        progress: &dyn PublishProgress,
    ) -> Result<PublishedStudyPackage>;
}
