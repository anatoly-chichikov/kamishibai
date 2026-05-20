//! Background pass contracts shared by the CLI shell and production adapter.

use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::session::{
    ArtifactFile, BulkCorrection, CardBody, CardBodyGeneration, CardCorrection, CardDraft,
    CardRevision, LanguagePair, Understanding, Understood, WordCandidate,
};
use crate::tui::BusyKind;

/// Text-oriented Gemini passes delegated by the interactive shell.
pub(super) trait TextPasses:
    Understanding + BulkCorrection + CardBodyGeneration + CardCorrection + Clone + Send + 'static
{
}

impl<T> TextPasses for T where
    T: Understanding
        + BulkCorrection
        + CardBodyGeneration
        + CardCorrection
        + Clone
        + Send
        + 'static
{
}

/// Media-oriented Gemini passes plus deck and report finalization.
pub(super) trait MediaPasses: Clone + Send + 'static {
    fn produce_scene(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn produce_picture(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn produce_sound(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn persist_body(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        body: &CardBody,
    ) -> Result<ArtifactFile>;
    fn publish(
        &self,
        drafts: &[CardDraft],
        progress: &PublishProgress,
    ) -> Result<(String, String, String)>;
}

/// Full lifecycle port required by the interactive shell.
pub(super) trait Lifecycle: TextPasses + MediaPasses {}

impl<T> Lifecycle for T where T: TextPasses + MediaPasses {}

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<Vec<WordCandidate>>),
    CardCorrection(Result<Box<(CardRevision, Option<ArtifactFile>)>>),
}

/// Result produced by one background artifact pass.
pub(super) enum ArtifactOutcome {
    Body(Result<(CardBody, Option<ArtifactFile>)>),
    Media(Result<ArtifactFile>),
}

/// Progress signalled by the background publish job.
pub(super) enum PublishMessage {
    Phase(BusyKind),
    Done(Result<(String, String, String)>),
}

/// Progress sender handed to publish implementations.
#[derive(Clone)]
pub(super) struct PublishProgress {
    sender: Sender<PublishMessage>,
}

impl PublishProgress {
    /// Build progress reporting around a publish message sender.
    pub(super) fn new(sender: Sender<PublishMessage>) -> Self {
        Self { sender }
    }

    /// Announce the publish job has moved to a new phase.
    pub(super) fn report_phase(&self, kind: BusyKind) {
        let _ = self.sender.send(PublishMessage::Phase(kind));
    }
}
