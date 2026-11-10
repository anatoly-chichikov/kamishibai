//! Reusable synchronous generation with delivery-owned scheduling and publication.

use std::fs::{self, File};

use anyhow::{Context, Result, bail};

use super::CardProduction;
use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardDraft, CardRevision, EngineEvent, SessionEngine,
};

/// Delivery-owned selection and cooperative refusal of new paid work within one artifact.
///
/// A pause reported before `GenerationRun::advance_in` starts prevents all provider
/// requests and leaves attempts, request budgets, and scene indices unchanged,
/// however many times the caller advances while paused.
///
/// A request refused after a step starts may raise the pause without spending an
/// artifact attempt. Costs already incurred remain recorded, but unfinished work
/// is discarded and bought again on resume: one such pause loses at most one
/// composed scene. An index for a possibly sent scene stays reserved; a scene
/// request confirmed not sent returns its index. A not-sent refusal without a
/// reported pause is a failed attempt, bounded by the usual attempt ceiling.
///
/// Experimental embedding API; compatibility is not guaranteed.
pub trait GenerationControl {
    /// Whether this stable card position may start its next artifact.
    fn allows(&self, card: usize) -> bool;
    /// Whether work must stay paused, including after a request was refused mid-step.
    /// Keep this true until work is explicitly resumed.
    fn paused(&self) -> bool;
}

struct Unrestricted;
impl GenerationControl for Unrestricted {
    fn allows(&self, _card: usize) -> bool {
        true
    }
    fn paused(&self) -> bool {
        false
    }
}

/// Drives one batch through the same artifact queue used by the terminal.
///
/// Call `advance` from a worker thread for blocking providers and inspect the
/// updated drafts after each step. Publication is a separate capability.
pub struct GenerationRun {
    engine: SessionEngine,
    cancelled: bool,
}

impl GenerationRun {
    /// Start or resume drafts after the caller has activated all staged rewrites.
    ///
    /// Pending adjustments or unavailable completed files refuse the whole batch
    /// before provider work. Metadata can live only in memory with no file attached.
    pub fn new(drafts: Vec<CardDraft>) -> Result<Self> {
        if drafts.iter().any(|draft| draft.staged_rewrite().is_some()) {
            bail!("activate staged card adjustments before starting generation");
        }
        validate_ready(&drafts)?;
        Ok(Self {
            engine: SessionEngine::start(drafts),
            cancelled: false,
        })
    }

    /// Inspect the next artifact without invoking a provider.
    #[must_use]
    pub fn next(&self) -> Option<(usize, Artifact)> {
        if self.cancelled {
            return None;
        }
        self.engine.next_target()
    }

    /// Produce and settle one artifact, preserving the engine's retries and costs.
    ///
    /// Provider failures become step errors and retry state; a drained or cancelled
    /// run returns `None`. Missing or unreadable completed files return an error
    /// before another provider call; this never silently schedules paid regeneration.
    pub fn advance<P: CardProduction + ?Sized>(
        &mut self,
        production: &P,
    ) -> Result<Option<GenerationStep>> {
        self.advance_in(production, &Unrestricted)
    }

    /// Advance one allowed position unless control already reports a pause.
    ///
    /// While paused at entry, repeated calls return `None` without any provider
    /// request or change to attempts, request budgets, or scene indices.
    /// If a request is refused and raises the pause during this step, return `None`
    /// and retain its costs without spending an artifact attempt. Unfinished work
    /// is discarded and repeated on resume, losing at most one composed scene per
    /// such pause. A possibly sent scene keeps its index; a scene request confirmed
    /// not sent returns its index. A not-sent refusal without a reported pause
    /// spends an attempt and remains bounded by the usual attempt ceiling.
    ///
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn advance_in<P: CardProduction + ?Sized>(
        &mut self,
        production: &P,
        control: &dyn GenerationControl,
    ) -> Result<Option<GenerationStep>> {
        if self.cancelled || control.paused() {
            return Ok(None);
        }
        validate_ready(self.engine.drafts())?;
        let Some((card, artifact)) = self.engine.next_allowed(|card| control.allows(card)) else {
            return Ok(None);
        };
        let outcome = produce_artifact(production, card, artifact, &self.engine.drafts()[card]);
        if outcome.error().is_some() && control.paused() {
            outcome.pause(&mut self.engine, card, artifact);
            return Ok(None);
        }
        let error = outcome.error().map(|error| format!("{error:#}"));
        let event = outcome.apply(&mut self.engine, card, artifact);
        validate_ready(self.engine.drafts())?;
        Ok(Some(GenerationStep {
            card,
            artifact,
            event,
            error,
        }))
    }

    /// Inspect the current drafts, including failed attempts and accumulated costs.
    #[must_use]
    pub fn drafts(&self) -> &[CardDraft] {
        self.engine.drafts()
    }

    /// Report completion only while completed files remain readable and nonempty.
    ///
    /// Missing files are explicit errors even after the final provider step.
    pub fn state(&self) -> Result<Option<EngineEvent>> {
        if self.cancelled {
            return Ok(None);
        }
        validate_ready(self.engine.drafts())?;
        Ok(self.engine.batch_state())
    }

    /// Stop future steps while preserving drafts that can be resumed in a new run.
    ///
    /// Cancellation is checked between blocking provider calls and cannot interrupt
    /// a call already executing. The caller owns that provider's request timeouts.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Consume the run into its current drafts for persistence or publication.
    #[must_use]
    pub fn into_drafts(self) -> Vec<CardDraft> {
        self.engine.drafts().to_vec()
    }
}

fn validate_ready(drafts: &[CardDraft]) -> Result<()> {
    for draft in drafts {
        let artifacts = draft.artifacts();
        if artifacts.meta().ready() && draft.meta().is_none() {
            bail!(
                "card '{}' marks metadata ready without its metadata",
                draft.term()
            );
        }
        for slot in [
            artifacts.meta(),
            artifacts.sound(),
            artifacts.scene(),
            artifacts.picture(),
        ] {
            if !slot.ready() {
                continue;
            }
            let Some(file) = slot.file() else {
                if slot.kind() == Artifact::Meta {
                    continue;
                }
                bail!(
                    "card '{}' marks {} ready without its file",
                    draft.term(),
                    slot.kind().label()
                );
            };
            let context = || {
                format!(
                    "completed {} for card '{}' is unavailable at '{}'",
                    slot.kind().label(),
                    draft.term(),
                    file.path().display()
                )
            };
            let metadata = fs::metadata(file.path()).with_context(context)?;
            if !metadata.is_file() || metadata.len() == 0 {
                bail!("{}: expected a nonempty regular file", context());
            }
            File::open(file.path()).with_context(context)?;
        }
    }
    Ok(())
}

/// One settled artifact with its engine transition and optional provider error.
pub struct GenerationStep {
    card: usize,
    artifact: Artifact,
    event: EngineEvent,
    error: Option<String>,
}

impl GenerationStep {
    /// Consume this step into its card slot, artifact, transition, and failure detail.
    #[must_use]
    pub fn into_parts(self) -> (usize, Artifact, EngineEvent, Option<String>) {
        (self.card, self.artifact, self.event, self.error)
    }
}

/// The result of one artifact operation before the delivery surface settles it.
pub(crate) enum ArtifactOutcome {
    Meta(Box<ArtifactAttempt<(CardRevision, Option<ArtifactFile>)>>),
    Media(Box<ArtifactAttempt<ArtifactFile>>),
}

impl ArtifactOutcome {
    /// Inspect a provider failure before committing the result to the engine.
    pub(crate) fn error(&self) -> Option<&anyhow::Error> {
        match self {
            Self::Meta(attempt) => attempt.error(),
            Self::Media(attempt) => attempt.error(),
        }
    }

    /// Account a cooperative refusal without recording a failed artifact attempt.
    fn pause(self, engine: &mut SessionEngine, card: usize, artifact: Artifact) {
        match self {
            Self::Meta(attempt) => engine.paused(card, artifact, *attempt),
            Self::Media(attempt) => engine.paused(card, artifact, *attempt),
        }
    }

    /// Commit this operation through the engine's shared retry and accounting policy.
    pub(crate) fn apply(
        self,
        engine: &mut SessionEngine,
        card: usize,
        artifact: Artifact,
    ) -> EngineEvent {
        match self {
            Self::Meta(attempt) => engine.applied_revision_attempt(card, *attempt),
            Self::Media(attempt) => engine.applied_media_attempt(card, artifact, *attempt),
        }
    }
}

/// Produce one engine-selected artifact without changing state or scheduling threads.
pub(crate) fn produce_artifact<P: CardProduction + ?Sized>(
    production: &P,
    card: usize,
    artifact: Artifact,
    draft: &CardDraft,
) -> ArtifactOutcome {
    match artifact {
        Artifact::Meta => {
            ArtifactOutcome::Meta(Box::new(production.generate_draft_meta_in(card, draft)))
        }
        Artifact::Sound => {
            ArtifactOutcome::Media(Box::new(production.generate_sound_in(card, draft)))
        }
        Artifact::Scene => {
            ArtifactOutcome::Media(Box::new(production.generate_scene_in(card, draft)))
        }
        Artifact::Picture => {
            ArtifactOutcome::Media(Box::new(production.generate_picture_in(card, draft)))
        }
    }
}
