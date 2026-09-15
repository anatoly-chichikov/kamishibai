//! Background job outcomes owned by the interactive shell.

use anyhow::Result;

use crate::application::{PublishPhase, PublishedStudyPackage};
use crate::session::{SenseCorrection, Understood};

/// Result produced by one background text pass.
pub(super) enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<SenseCorrection>),
    KeyCheck(Result<()>),
}

/// Progress signalled by the background publish job.
pub(super) enum StudyPublishMessage {
    Phase(PublishPhase),
    Done(Result<PublishedStudyPackage>),
}
