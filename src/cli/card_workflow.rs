//! UI-neutral card workflow ports shared by the interactive shell and the
//! console worker.

use anyhow::Result;

use crate::session::{
    ArtifactFile, BulkCorrection, CardCorrection, CardDraft, CardMeta, CardMetaGeneration,
    CardRevision, LanguagePair, SenseCorrection, Understanding, Understood,
};

/// Capability that turns typed words into understood words.
pub(super) trait WordUnderstanding:
    Understanding + BulkCorrection + Clone + Send + 'static
{
}

impl<T> WordUnderstanding for T where T: Understanding + BulkCorrection + Clone + Send + 'static {}

/// Capability that turns understood words into card meta and media.
pub(super) trait CardGeneration:
    CardMetaGeneration + CardCorrection + Clone + Send + 'static
{
    fn generate_scene(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn generate_picture(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn generate_sound(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile>;
}

/// The two stages a publish job moves through, in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublishPhase {
    /// The Anki deck is being written.
    Deck,
    /// The PDF report is being written.
    Report,
}

/// Progress port for one publish job; implementations forward phase changes to
/// whatever surface is watching (the TUI busy modal, or nothing at all).
pub(super) trait PublishProgress {
    /// Announce that the publish job advanced to `phase`.
    fn advance(&self, phase: PublishPhase);
}

/// Capability that turns completed cards into deck and report files.
pub(super) trait DeckPublishing: Clone + Send + 'static {
    fn publish_deck(
        &self,
        drafts: &[CardDraft],
        progress: &dyn PublishProgress,
    ) -> Result<(String, String, String)>;
}

/// Capability that confirms a freshly entered API key is accepted by Gemini.
///
/// Takes the key from the Welcome buffer (not the saved one) so the check
/// happens before anything is written to preferences.
pub(super) trait KeyValidation: Clone + Send + 'static {
    fn check_key(&self, key: &str) -> Result<()>;
}

/// Full card workflow required by the interactive shell.
pub(super) trait CardWorkflow:
    WordUnderstanding + CardGeneration + DeckPublishing + KeyValidation
{
}

impl<T> CardWorkflow for T where
    T: WordUnderstanding + CardGeneration + DeckPublishing + KeyValidation
{
}

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<SenseCorrection>),
    CardCorrection(Result<Box<(CardRevision, Option<ArtifactFile>)>>),
    KeyCheck(Result<()>),
}

/// Result produced by one background artifact pass.
pub(super) enum ArtifactOutcome {
    Meta(Result<(CardMeta, Option<ArtifactFile>)>),
    Media(Result<ArtifactFile>),
}

/// Progress signalled by the background publish job.
pub(super) enum DeckPublishMessage {
    Phase(PublishPhase),
    Done(Result<(String, String, String)>),
}
