//! UI-shaped card workflow contracts used by the interactive shell.

use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::session::{
    ArtifactFile, BulkCorrection, CardCorrection, CardDraft, CardMeta, CardMetaGeneration,
    CardRevision, LanguagePair, Understanding, Understood, WordCandidate,
};
use crate::tui::BusyKind;

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

/// Capability that turns completed cards into deck and report files.
pub(super) trait DeckPublishing: Clone + Send + 'static {
    fn publish_deck(
        &self,
        drafts: &[CardDraft],
        progress: &DeckPublishProgress,
    ) -> Result<(String, String, String)>;
}

/// Full card workflow required by the interactive shell.
pub(super) trait CardWorkflow: WordUnderstanding + CardGeneration + DeckPublishing {}

impl<T> CardWorkflow for T where T: WordUnderstanding + CardGeneration + DeckPublishing {}

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<Vec<WordCandidate>>),
    CardCorrection(Result<Box<(CardRevision, Option<ArtifactFile>)>>),
}

/// Result produced by one background artifact pass.
pub(super) enum ArtifactOutcome {
    Meta(Result<(CardMeta, Option<ArtifactFile>)>),
    Media(Result<ArtifactFile>),
}

/// Progress signalled by the background publish job.
pub(super) enum DeckPublishMessage {
    Phase(BusyKind),
    Done(Result<(String, String, String)>),
}

/// Progress sender handed to publish implementations.
#[derive(Clone)]
pub(super) struct DeckPublishProgress {
    sender: Sender<DeckPublishMessage>,
}

impl DeckPublishProgress {
    /// Build progress reporting around a publish message sender.
    pub(super) fn new(sender: Sender<DeckPublishMessage>) -> Self {
        Self { sender }
    }

    /// Announce the publish job has moved to a new phase.
    pub(super) fn report_phase(&self, kind: BusyKind) {
        let _ = self.sender.send(DeckPublishMessage::Phase(kind));
    }
}
