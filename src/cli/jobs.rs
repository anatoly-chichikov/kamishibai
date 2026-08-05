//! Background job outcomes owned by the interactive shell.

use anyhow::Result;

use crate::application::{PublishPhase, PublishedStudyPackage};
use crate::session::{ArtifactAttempt, ArtifactFile, CardRevision, SenseCorrection, Understood};

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<SenseCorrection>),
    KeyCheck(Result<()>),
}

/// Result produced by one background artifact pass.
pub(super) enum ArtifactOutcome {
    Meta(Box<ArtifactAttempt<(CardRevision, Option<ArtifactFile>)>>),
    Media(ArtifactAttempt<ArtifactFile>),
}

/// Progress signalled by the background publish job.
pub(super) enum StudyPublishMessage {
    Phase(PublishPhase),
    Done(Result<PublishedStudyPackage>),
}
