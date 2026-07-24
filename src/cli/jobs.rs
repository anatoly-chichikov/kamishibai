//! Background job outcomes owned by the interactive shell.

use anyhow::Result;

use crate::application::{PublishPhase, PublishedStudyPackage};
use crate::session::{
    ArtifactAttempt, ArtifactFile, CardMeta, CardRevision, GenerationCost, SenseCorrection,
    Understood,
};

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<SenseCorrection>),
    CardCorrection(
        Result<Box<(CardRevision, Option<ArtifactFile>)>>,
        Option<GenerationCost>,
    ),
    KeyCheck(Result<()>),
}

/// Result produced by one background artifact pass.
pub(super) enum ArtifactOutcome {
    Meta(ArtifactAttempt<(CardMeta, Option<ArtifactFile>)>),
    Media(ArtifactAttempt<ArtifactFile>),
}

/// Progress signalled by the background publish job.
pub(super) enum StudyPublishMessage {
    Phase(PublishPhase),
    Done(Result<PublishedStudyPackage>),
}
