//! Interactive CLI shell that coordinates app state and background jobs.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use super::bridge::TuiSession;
use super::jobs::{ArtifactOutcome, StudyPublishMessage, TextOutcome};
use super::wiring::{GeminiCardWorkflow, GeminiKeyValidation, interactive_application};
use crate::application::{CardUseCases, KeyValidation, PublishPhase, PublishProgress};
use crate::config::{PreferenceStore, Preferences, default_store};
use crate::gemini::rejects_key;
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactAttempt, CardDraft, LearningTarget, RawInputBatch, SessionEngine,
};
use crate::tui::{App, AppEvent, BusyKind, KeySource, Screen, Side, WelcomeStage, transit};

const ANIMATION_FRAME_MILLIS: u64 = 250;
const IDLE_POLL: Duration = Duration::from_millis(ANIMATION_FRAME_MILLIS);
const BACKGROUND_POLL: Duration = Duration::from_millis(ANIMATION_FRAME_MILLIS);
const FAST_JOB_POLL: Duration = Duration::from_millis(25);
const FAST_JOB_WINDOW: Duration = Duration::from_millis(50);
const CONFIRMATION_WINDOW: Duration = Duration::from_millis(1000);
const KEY_REJECTED_MESSAGE: &str = "Gemini rejected this API key; saved key was cleared";

struct PendingJob<T> {
    receiver: Receiver<T>,
    handle: JoinHandle<()>,
    started: Instant,
}

impl<T> PendingJob<T>
where
    T: Send + 'static,
{
    fn spawn<F>(run: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let (sender, receiver) = channel();
        let handle = thread::spawn(move || {
            let _ = sender.send(run());
        });
        Self {
            receiver,
            handle,
            started: Instant::now(),
        }
    }

    fn fresh(&self) -> bool {
        self.started.elapsed() <= FAST_JOB_WINDOW
    }
}

struct PendingArtifactJob {
    job: PendingJob<ArtifactOutcome>,
    card: usize,
    artifact: Artifact,
}

struct PendingPublishJob {
    job: PendingJob<StudyPublishMessage>,
    cards: usize,
    failed: usize,
    stopped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestructiveEscape {
    ClearWords,
    StopGeneration,
}

/// Channel adapter that forwards publish-phase changes into the shell's
/// publish-job mailbox, implementing the UI-neutral [`PublishProgress`] port.
struct StudyPublishProgress {
    sender: Sender<StudyPublishMessage>,
}

impl StudyPublishProgress {
    /// Build progress reporting around a publish message sender.
    fn new(sender: Sender<StudyPublishMessage>) -> Self {
        Self { sender }
    }
}

impl PublishProgress for StudyPublishProgress {
    fn advance(&self, phase: PublishPhase) {
        let _ = self.sender.send(StudyPublishMessage::Phase(phase));
    }
}

/// Stateful coordinator for TUI app transitions and background card workflow execution.
pub(super) struct Shell<P, K> {
    app: App,
    engine: Option<SessionEngine>,
    text: Option<PendingJob<TextOutcome>>,
    artifact_job: Option<PendingArtifactJob>,
    publish_job: Option<PendingPublishJob>,
    regeneration_pending: bool,
    started: Option<Instant>,
    quit_armed_at: Option<Instant>,
    new_batch_armed_at: Option<Instant>,
    destructive_escape_armed_at: Option<(DestructiveEscape, Instant)>,
    workflow: P,
    keys: K,
    store: PreferenceStore,
    session: Option<TuiSession>,
    output: PathBuf,
}

impl Shell<GeminiCardWorkflow, GeminiKeyValidation> {
    /// Build a live card shell for an interactive empty session.
    pub(super) fn new(app: App, session: Option<TuiSession>) -> Result<Self> {
        let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
        crate::report::warm_fonts_async();
        let session = match session {
            Some(session) => session,
            None => TuiSession::fresh()?,
        };
        let output = match session.stored_output() {
            Some(output) => output,
            None => Locations::new(LocationArgs::default(), SystemContext).output()?,
        };
        let costs = session.cost_scope();
        let (workflow, keys) = interactive_application(cache, output.clone(), costs).into_parts();
        Ok(Self {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow,
            keys,
            store: default_store(&SystemContext)?,
            session: Some(session),
            output,
        })
    }

    /// Build a live card shell that starts with generation already running.
    pub(super) fn startup(
        app: App,
        drafts: Vec<CardDraft>,
        session: Option<TuiSession>,
    ) -> Result<Self> {
        let mut shell = Self::new(app.cards_started(drafts), session)?;
        if !shell.start_engine() {
            bail!("the session could not persist its committed plan before generation");
        }
        Ok(shell)
    }
}

impl<P, K> Shell<P, K>
where
    P: CardUseCases,
    K: KeyValidation,
{
    /// Borrow the app model for rendering and pointer geometry.
    pub(super) fn app(&self) -> &App {
        &self.app
    }

    /// Persist the live app to its on-disk session when the durable state has
    /// changed. While the engine or publish job runs the session is marked
    /// generating under this process's own pid, so concurrent CLI commands refuse
    /// to touch it. A persistence failure surfaces as a dismissable error instead
    /// of aborting the interactive run; the next edit retries the save.
    pub(super) fn persist(&mut self) {
        let generating =
            self.engine.is_some() || self.publish_job.is_some() || self.app.generation_stopping();
        let failure = match self.session.as_mut() {
            Some(session) => session
                .save(&self.app, self.output.as_path(), generating)
                .err(),
            None => None,
        };
        if let Some(error) = failure {
            self.app = self
                .app
                .clone()
                .error_shown(format!("session not saved: {error:#}"));
        }
    }

    /// Persist the committed generating state under its liveness lock before work starts.
    fn claim_session(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return true;
        };
        match session.claim_and_save(&self.app, self.output.as_path()) {
            Ok(true) => true,
            Ok(false) => {
                self.app = self.app.clone().error_shown(
                    "this session is being generated by another process; cancel it or wait",
                );
                false
            }
            Err(error) => {
                self.app = self
                    .app
                    .clone()
                    .error_shown(format!("could not claim the session: {error:#}"));
                false
            }
        }
    }

    /// Crossterm poll budget for the next loop iteration.
    pub(super) fn poll_timeout(&self) -> Duration {
        if self.has_engine_work() || self.has_fresh_job() {
            return FAST_JOB_POLL;
        }
        if self.has_background_work() {
            return BACKGROUND_POLL;
        }
        IDLE_POLL
    }

    fn has_fresh_job(&self) -> bool {
        self.text.as_ref().map(PendingJob::fresh).unwrap_or(false)
            || self
                .artifact_job
                .as_ref()
                .map(|job| job.job.fresh())
                .unwrap_or(false)
            || self
                .publish_job
                .as_ref()
                .map(|job| job.job.fresh())
                .unwrap_or(false)
    }

    fn has_background_work(&self) -> bool {
        self.text.is_some() || self.artifact_job.is_some() || self.publish_job.is_some()
    }

    fn has_engine_work(&self) -> bool {
        self.engine
            .as_ref()
            .map(|engine| engine.next_target().is_some())
            .unwrap_or(false)
    }

    /// Scroll the app body by one terminal wheel delta.
    pub(super) fn scroll(&mut self, delta: i32, viewport: u16, body_width: u16) -> bool {
        let before = self.app.body_scroll();
        self.app = self.app.clone().body_scrolled(delta, viewport, body_width);
        self.app.body_scroll() != before
    }

    /// Clamp the app body scroll to the current terminal viewport.
    pub(super) fn reclamp_scroll(&mut self, viewport: u16, body_width: u16) -> bool {
        let before = self.app.body_scroll();
        self.app = self.app.clone().body_scroll_clamped(viewport, body_width);
        self.app.body_scroll() != before
    }

    /// Move scroll so keyboard focus remains visible.
    pub(super) fn snap_scroll_to_selection(&mut self, viewport: u16, body_width: u16) -> bool {
        let before = self.app.body_scroll();
        self.app = self
            .app
            .clone()
            .body_scroll_to_selection(viewport, body_width);
        self.app.body_scroll() != before
    }

    /// Arm or confirm the two-step quit gesture.
    pub(super) fn arm_quit(&mut self) -> bool {
        let now = Instant::now();
        if let Some(armed) = self.quit_armed_at
            && now.duration_since(armed) <= CONFIRMATION_WINDOW
        {
            return true;
        }
        self.quit_armed_at = Some(now);
        if !self.app.quit_pending() {
            self.app = self.app.clone().with_quit_pending(true);
        }
        false
    }

    /// Clear any pending quit confirmation state.
    pub(super) fn disarm_quit(&mut self) -> bool {
        self.quit_armed_at = None;
        if self.app.quit_pending() {
            self.app = self.app.clone().with_quit_pending(false);
            return true;
        }
        false
    }

    /// Expire the quit confirmation window when the user pauses too long.
    pub(super) fn refresh_quit_pending(&mut self) -> bool {
        if let Some(armed) = self.quit_armed_at
            && armed.elapsed() > CONFIRMATION_WINDOW
        {
            return self.disarm_quit();
        }
        false
    }

    /// Consume Escape on a finished final screen, arming or confirming a fresh batch.
    pub(super) fn handle_new_batch_escape(&mut self) -> Result<bool> {
        if !self.can_start_new_batch() {
            return Ok(false);
        }
        let now = Instant::now();
        if let Some(armed) = self.new_batch_armed_at
            && now.duration_since(armed) <= CONFIRMATION_WINDOW
        {
            self.start_new_batch()?;
            return Ok(true);
        }
        self.new_batch_armed_at = Some(now);
        if !self.app.new_batch_pending() {
            self.app = self.app.clone().with_new_batch_pending(true);
        }
        Ok(true)
    }

    /// Clear any pending new-batch confirmation state.
    pub(super) fn disarm_new_batch(&mut self) -> bool {
        self.new_batch_armed_at = None;
        if self.app.new_batch_pending() {
            self.app = self.app.clone().with_new_batch_pending(false);
            return true;
        }
        false
    }

    /// Expire the final-screen Escape confirmation window after a pause.
    pub(super) fn refresh_new_batch_pending(&mut self) -> bool {
        if let Some(armed) = self.new_batch_armed_at
            && armed.elapsed() > CONFIRMATION_WINDOW
        {
            return self.disarm_new_batch();
        }
        false
    }

    /// Expire a guarded screen action after a pause or eligibility change.
    pub(super) fn refresh_destructive_escape_pending(&mut self) -> bool {
        let Some((action, armed)) = self.destructive_escape_armed_at else {
            return false;
        };
        if armed.elapsed() > CONFIRMATION_WINDOW || !self.can_confirm(action) {
            return self.disarm_destructive_escape();
        }
        false
    }

    /// Clear a pending words-clear or generation-stop confirmation.
    pub(super) fn disarm_destructive_escape(&mut self) -> bool {
        let changed = self.destructive_escape_armed_at.take().is_some()
            || self.app.word_clear_pending()
            || self.app.generation_stop_pending();
        self.app = self
            .app
            .clone()
            .with_word_clear_pending(false)
            .with_generation_stop_pending(false);
        changed
    }

    fn handle_destructive_escape(&mut self, action: DestructiveEscape) {
        if !self.can_confirm(action) {
            self.disarm_destructive_escape();
            return;
        }
        let now = Instant::now();
        let confirmed = self.destructive_escape_armed_at.is_some_and(|(armed, at)| {
            armed == action && now.duration_since(at) <= CONFIRMATION_WINDOW
        });
        if confirmed {
            self.destructive_escape_armed_at = None;
            match action {
                DestructiveEscape::ClearWords => {
                    self.clear_words();
                }
                DestructiveEscape::StopGeneration => {
                    self.regeneration_pending = false;
                    self.app = self
                        .app
                        .clone()
                        .with_generation_stop_pending(false)
                        .generation_stop_started();
                }
            }
            return;
        }
        self.disarm_destructive_escape();
        self.destructive_escape_armed_at = Some((action, now));
        self.app = match action {
            DestructiveEscape::ClearWords => self.app.clone().with_word_clear_pending(true),
            DestructiveEscape::StopGeneration => {
                self.app.clone().with_generation_stop_pending(true)
            }
        };
    }

    fn can_confirm(&self, action: DestructiveEscape) -> bool {
        match action {
            DestructiveEscape::ClearWords => {
                self.app.screen() == Screen::YourWords && !self.app.blob().is_empty()
            }
            DestructiveEscape::StopGeneration => {
                self.app.screen() == Screen::YourCards
                    && self.engine.is_some()
                    && self.publish_job.is_none()
                    && self.app.busy().is_none()
                    && !self.app.generation_stopping()
            }
        }
    }

    fn clear_words(&mut self) {
        let established = !self.app.candidates().is_empty() || !self.app.cards().is_empty();
        if established
            && let Some(session) = self.session.as_mut()
            && let Err(error) = session.cancel_and_start_next(&self.app, self.output.as_path())
        {
            self.app = self
                .app
                .clone()
                .with_word_clear_pending(false)
                .error_shown(format!("session not cancelled: {error:#}"));
            return;
        }
        self.app = self.app.clone().starting_new_batch();
        self.regeneration_pending = false;
        self.started = None;
    }

    fn can_start_new_batch(&self) -> bool {
        self.app.can_start_new_batch()
            && self.engine.is_none()
            && self.text.is_none()
            && self.artifact_job.is_none()
            && self.publish_job.is_none()
    }

    fn start_new_batch(&mut self) -> Result<()> {
        if let Some(session) = self.session.as_mut() {
            session.start_next_batch()?;
        }
        self.app = self.app.clone().starting_new_batch();
        self.regeneration_pending = false;
        self.started = None;
        self.quit_armed_at = None;
        self.new_batch_armed_at = None;
        Ok(())
    }

    /// Apply one app event and start any resulting side effect.
    pub(super) fn handle(&mut self, event: AppEvent) -> Result<Side> {
        if !matches!(event, AppEvent::Cancel | AppEvent::Redraw) {
            self.disarm_destructive_escape();
        }
        if self.text.is_some() {
            return Ok(Side::None);
        }
        let cancelled = event == AppEvent::Cancel;
        let (next, side) = transit(self.app.clone(), event);
        self.app = next;
        if cancelled && !matches!(side, Side::ClearWords | Side::StopGeneration) {
            self.disarm_destructive_escape();
        }
        self.apply(side.clone())?;
        Ok(side)
    }

    /// Advance background work and report whether the terminal should redraw.
    pub(super) fn tick(&mut self) -> Result<bool> {
        let mut changed = self.refresh_generation_elapsed();
        changed |= self.poll_text()?;
        if self.text.is_some() {
            return Ok(changed);
        }
        changed |= self.poll_artifact()?;
        if self.artifact_job.is_some() {
            return Ok(changed);
        }
        if self.app.generation_cancelling() {
            if self.app.error().is_none() {
                changed |= self.cancel_stopped_generation(None);
            }
            return Ok(changed);
        }
        if self.app.generation_stopping() && self.publish_job.is_none() {
            if self.app.error().is_none() {
                changed |= self.finish_generation_stop()?;
            }
            return Ok(changed);
        }
        changed |= self.poll_publish()?;
        if self.publish_job.is_some() {
            return Ok(changed);
        }
        changed |= self.resume_regeneration()?;
        changed |= self.advance_engine()?;
        Ok(changed)
    }

    fn refresh_generation_elapsed(&mut self) -> bool {
        let Some(started) = self.started else {
            return false;
        };
        let elapsed = animation_elapsed(started.elapsed());
        if self.app.elapsed() == elapsed {
            return false;
        }
        self.app = self.app.clone().with_elapsed(elapsed);
        true
    }

    fn refresh_busy_elapsed(&mut self, started: Instant) -> bool {
        let Some(current) = self.app.busy().map(|busy| busy.elapsed()) else {
            return false;
        };
        let elapsed = animation_elapsed(started.elapsed());
        if current == elapsed {
            return false;
        }
        self.app = self.app.clone().busy_elapsed(elapsed);
        true
    }

    fn advance_engine(&mut self) -> Result<bool> {
        let Some(engine) = self.engine.as_ref() else {
            return Ok(false);
        };
        if let Some((card, kind)) = engine.next_target() {
            self.spawn_artifact(card, kind)?;
            return Ok(true);
        }
        if self.app.cards_pending() > 0 {
            return Ok(false);
        }
        if let Some(event) = engine.batch_state() {
            let app_event = match event {
                crate::session::EngineEvent::BatchReady => Some(AppEvent::BatchReady),
                crate::session::EngineEvent::BatchDone { failed_cards } => {
                    Some(AppEvent::BatchDone {
                        failed: failed_cards,
                    })
                }
                _ => None,
            };
            if let Some(app_event) = app_event {
                self.handle(app_event)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn spawn_artifact(&mut self, card: usize, artifact: Artifact) -> Result<()> {
        if self.artifact_job.is_some() {
            return Ok(());
        }
        let Some(engine) = self.engine.as_ref() else {
            return Ok(());
        };
        let draft = engine.drafts()[card].clone();
        let workflow = self.workflow.clone();
        let job = PendingJob::spawn(move || match artifact {
            Artifact::Meta => {
                ArtifactOutcome::Meta(Box::new(workflow.generate_draft_meta_in(card, &draft)))
            }
            Artifact::Scene => ArtifactOutcome::Media(workflow.generate_scene_in(card, &draft)),
            Artifact::Picture => ArtifactOutcome::Media(workflow.generate_picture_in(card, &draft)),
            Artifact::Sound => ArtifactOutcome::Media(workflow.generate_sound_in(card, &draft)),
        });
        self.artifact_job = Some(PendingArtifactJob {
            job,
            card,
            artifact,
        });
        self.app = self.app.clone().cards_running(Some((card, artifact)));
        Ok(())
    }

    fn poll_artifact(&mut self) -> Result<bool> {
        let Some(job) = self.artifact_job.as_ref() else {
            return Ok(false);
        };
        match job.job.receiver.try_recv() {
            Ok(outcome) => {
                let job = self
                    .artifact_job
                    .take()
                    .expect("invariant: artifact job must exist");
                let _ = join_thread(job.job.handle);
                self.apply_artifact_outcome(job.card, job.artifact, outcome);
                Ok(true)
            }
            Err(TryRecvError::Empty) => Ok(false),
            Err(TryRecvError::Disconnected) => {
                let job = self
                    .artifact_job
                    .take()
                    .expect("invariant: artifact job must exist");
                let _ = join_thread(job.job.handle);
                let synthetic = anyhow!("background artifact task disconnected");
                let outcome = match job.artifact {
                    Artifact::Meta => {
                        ArtifactOutcome::Meta(Box::new(ArtifactAttempt::unmetered(Err(synthetic))))
                    }
                    _ => ArtifactOutcome::Media(ArtifactAttempt::unmetered(Err(synthetic))),
                };
                self.apply_artifact_outcome(job.card, job.artifact, outcome);
                Ok(true)
            }
        }
    }

    fn apply_artifact_outcome(
        &mut self,
        card: usize,
        artifact: Artifact,
        outcome: ArtifactOutcome,
    ) {
        if artifact_rejects_key(&outcome) {
            if self.app.generation_stopping() {
                self.app = self.app.clone().cards_running(None);
            } else {
                self.recover_key_rejection();
            }
            return;
        }
        let requests = self
            .app
            .cards()
            .iter()
            .map(|draft| draft.staged_rewrite().cloned())
            .collect::<Vec<_>>();
        let Some(engine) = self.engine.as_mut() else {
            self.app = self.app.clone().cards_running(None);
            return;
        };
        let _event = match outcome {
            ArtifactOutcome::Meta(attempt) => engine.applied_revision_attempt(card, *attempt),
            ArtifactOutcome::Media(attempt) => {
                engine.applied_media_attempt(card, artifact, attempt)
            }
        };
        let drafts = engine
            .drafts()
            .iter()
            .cloned()
            .zip(requests)
            .map(|(draft, request)| match request {
                Some(request) => draft.staging_rewrite(request.selection().clone(), request.note()),
                None => draft,
            })
            .collect();
        self.app = self.app.clone().cards_replaced(drafts).cards_running(None);
    }

    fn poll_text(&mut self) -> Result<bool> {
        let Some(started) = self.text.as_ref().map(|job| job.started) else {
            return Ok(false);
        };
        let mut changed = self.refresh_busy_elapsed(started);
        let Some(job) = self.text.as_ref() else {
            return Ok(changed);
        };
        match job.receiver.try_recv() {
            Ok(outcome) => {
                let job = self.text.take().expect("invariant: text job must exist");
                self.app = self.app.clone().busy_finished();
                changed = true;
                if let Err(error) = join_thread(job.handle) {
                    self.app = self.app.clone().error_shown(error.to_string());
                    return Ok(true);
                }
                self.finish_text(outcome);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let job = self.text.take().expect("invariant: text job must exist");
                let message = join_thread(job.handle)
                    .map(|()| String::from("background text pass disconnected"))
                    .unwrap_or_else(|error| error.to_string());
                self.app = self.app.clone().busy_finished().error_shown(message);
                changed = true;
            }
        }
        Ok(changed)
    }

    fn finish_text(&mut self, outcome: TextOutcome) {
        match outcome {
            TextOutcome::Understanding(result) => match result {
                Ok(understood) => {
                    self.app = self
                        .app
                        .clone()
                        .with_screen(Screen::WhatIUnderstood)
                        .confirmed_learning(understood.guess().code())
                        .understood_preserving_senses(understood.candidates().to_vec());
                }
                Err(error) => {
                    if rejects_key(&error) {
                        self.recover_key_rejection();
                        return;
                    }
                    self.app = self
                        .app
                        .clone()
                        .with_screen(Screen::YourWords)
                        .error_shown(error.to_string());
                }
            },
            TextOutcome::BulkCorrection(result) => match result {
                Ok(update) => {
                    let (senses, message) = update.into_parts();
                    self.app = self
                        .app
                        .clone()
                        .close_modal()
                        .senses_appended_to_selected(senses, message);
                }
                Err(error) => {
                    if rejects_key(&error) {
                        self.recover_key_rejection();
                        return;
                    }
                    self.app = self.app.clone().error_shown(error.to_string());
                }
            },
            TextOutcome::KeyCheck(result) => match result {
                Ok(()) => {
                    let language = self.app.pair().known().to_string();
                    let key = self.app.welcome().key.clone();
                    if let Err(error) = self
                        .persist_preferences(move |prefs| prefs.adopt(language).with_api_key(key))
                    {
                        self.app = self.app.clone().welcome_notice(error.to_string());
                        return;
                    }
                    if self.app.cards().is_empty() {
                        self.app = self.app.clone().with_screen(Screen::YourWords);
                    } else {
                        self.app = self.app.clone().with_screen(Screen::YourCards);
                        self.start_engine();
                    }
                }
                Err(error) => {
                    let message = if rejects_key(&error) {
                        "key invalid"
                    } else {
                        "couldn't reach gemini"
                    };
                    self.app = self.app.clone().welcome_notice(message);
                }
            },
        }
    }

    fn apply(&mut self, side: Side) -> Result<()> {
        match side {
            Side::RunUnderstanding => {
                let raw = RawInputBatch::new(self.app.blob());
                let known = self.app.pair().known().to_string();
                let workflow = self.workflow.clone();
                self.start_text(BusyKind::Understanding, move || {
                    TextOutcome::Understanding(workflow.understand(
                        &raw,
                        known.as_str(),
                        &LearningTarget::Detect,
                    ))
                })?;
            }
            Side::StartGeneration => {
                let drafts = drafts_from(&self.app);
                self.app = self.app.clone().cards_started(drafts);
                self.start_engine();
            }
            Side::ClearWords => {
                self.handle_destructive_escape(DestructiveEscape::ClearWords);
            }
            Side::StopGeneration => {
                self.handle_destructive_escape(DestructiveEscape::StopGeneration);
            }
            Side::RegenerateFailed => {
                self.app = self.app.clone().cards_reset_failures();
                self.start_engine();
            }
            Side::RegenerateCards => {
                self.regenerate_cards()?;
            }
            Side::RunBulkCorrection(comment) => {
                let Some(focused) = self.app.candidates().get(self.app.selected()).cloned() else {
                    return Ok(());
                };
                let pair = self.app.pair().clone();
                let workflow = self.workflow.clone();
                self.start_text(BusyKind::BulkCorrection, move || {
                    TextOutcome::BulkCorrection(workflow.correct_bulk(
                        &focused,
                        comment.as_str(),
                        &pair,
                    ))
                })?;
            }
            Side::PersistMyLanguageAndRunUnderstanding(code) => {
                self.persist_preferences(|prefs| prefs.adopt(code))?;
                let raw = RawInputBatch::new(self.app.blob());
                let known = self.app.pair().known().to_string();
                let workflow = self.workflow.clone();
                self.start_text(BusyKind::Understanding, move || {
                    TextOutcome::Understanding(workflow.understand(
                        &raw,
                        known.as_str(),
                        &LearningTarget::Detect,
                    ))
                })?;
            }
            Side::StartPublish => {
                let _ = self.start_publish(false)?;
            }
            Side::PersistMyLanguage(code) => {
                self.persist_preferences(|prefs| prefs.adopt(code))?;
            }
            Side::ValidateKey(key) => {
                let keys = self.keys.clone();
                self.start_text(BusyKind::CheckingKey, move || {
                    TextOutcome::KeyCheck(keys.check_key(key.as_str()))
                })?;
            }
            Side::LoadEnvKey => {
                self.load_env_key();
            }
            Side::ExitApp | Side::None => {}
        }
        Ok(())
    }

    fn load_env_key(&mut self) {
        let key = std::env::var("GEMINI_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        self.apply_env_key(key);
    }

    fn apply_env_key(&mut self, key: Option<String>) {
        self.app = match key {
            Some(value) => self.app.clone().welcome_env_key(value),
            None => self.app.clone().welcome_notice("GEMINI_API_KEY is not set"),
        };
    }

    /// Persist a preference update to this shell's own store. Tests inject a
    /// throwaway store, so the suite never mutates the real user preferences.
    fn persist_preferences(&self, update: impl FnOnce(Preferences) -> Preferences) -> Result<()> {
        self.store.update(update)?;
        Ok(())
    }

    fn recover_key_rejection(&mut self) {
        self.regeneration_pending = false;
        self.destructive_escape_armed_at = None;
        if let Err(error) = clear_saved_key_in(&self.store) {
            self.app = self.app.clone().error_shown(error.to_string());
            return;
        }
        self.engine = None;
        self.started = None;
        let env_available = super::terminal::env_has_gemini_key();
        self.app = self
            .app
            .clone()
            .busy_finished()
            .cards_running(None)
            .with_word_clear_pending(false)
            .generation_stop_finished()
            .close_modal()
            .opening_welcome_at(
                WelcomeStage::EnterKey,
                KeySource::Empty,
                String::new(),
                env_available,
            )
            .welcome_notice(KEY_REJECTED_MESSAGE);
    }

    fn start_text<F>(&mut self, kind: BusyKind, run: F) -> Result<()>
    where
        F: FnOnce() -> TextOutcome + Send + 'static,
    {
        if self.text.is_some() {
            bail!("background text pass already running");
        }
        self.text = Some(PendingJob::spawn(run));
        self.app = self.app.clone().busy_started(kind);
        Ok(())
    }

    fn regenerate_cards(&mut self) -> Result<()> {
        if self.app.cards().is_empty() {
            return Ok(());
        }
        if self.artifact_job.is_some() || self.publish_job.is_some() {
            self.regeneration_pending = true;
            return Ok(());
        }
        self.restart_regeneration()
    }

    fn resume_regeneration(&mut self) -> Result<bool> {
        if !self.regeneration_pending {
            return Ok(false);
        }
        self.regeneration_pending = false;
        self.restart_regeneration()?;
        Ok(true)
    }

    fn restart_regeneration(&mut self) -> Result<()> {
        let previous_app = self.app.clone();
        let previous_engine = self.engine.clone();
        let previous_started = self.started;
        self.engine = None;
        self.started = None;
        let staged = self
            .app
            .cards()
            .iter()
            .any(|draft| draft.staged_rewrite().is_some());
        if staged {
            let drafts = self
                .app
                .cards()
                .iter()
                .cloned()
                .map(CardDraft::starting_rewrite)
                .collect();
            self.app = self.app.clone().cards_replaced(drafts);
        }
        self.app = self
            .app
            .clone()
            .busy_finished()
            .cards_running(None)
            .publication_cleared()
            .error_cleared();
        if staged {
            if !self.start_engine() {
                self.rollback(previous_app, previous_engine, previous_started);
            }
            return Ok(());
        }
        if self.app.cards_failed() > 0 {
            self.app = self.app.clone().cards_reset_failures();
            if !self.start_engine() {
                self.rollback(previous_app, previous_engine, previous_started);
            }
            return Ok(());
        }
        if self
            .app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready())
        {
            let _ = self.start_publish(false)?;
            if self.publish_job.is_none() {
                self.rollback(previous_app, previous_engine, previous_started);
            }
        } else if !self.start_engine() {
            self.rollback(previous_app, previous_engine, previous_started);
        }
        Ok(())
    }

    fn rollback(
        &mut self,
        previous_app: App,
        previous_engine: Option<SessionEngine>,
        previous_started: Option<Instant>,
    ) {
        let error = self.app.error().map(String::from);
        self.app = match error {
            Some(error) => previous_app.error_shown(error),
            None => previous_app,
        };
        self.engine = previous_engine;
        self.started = previous_started;
    }

    fn start_engine(&mut self) -> bool {
        if !self.hydrate_session_costs() {
            return false;
        }
        if !self.claim_session() {
            return false;
        }
        self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
        self.started = Some(Instant::now());
        true
    }

    fn hydrate_session_costs(&mut self) -> bool {
        let Some(session) = self.session.as_ref() else {
            return true;
        };
        match session.hydrate_app(&self.app) {
            Ok(app) => {
                self.app = app;
                true
            }
            Err(error) => {
                self.app = self
                    .app
                    .clone()
                    .error_shown(format!("could not restore session costs: {error:#}"));
                false
            }
        }
    }

    fn start_publish(&mut self, stopped: bool) -> Result<bool> {
        if self.publish_job.is_some() {
            bail!("background publish job already running");
        }
        if !self.claim_session() {
            return Ok(false);
        }
        let total = self.app.cards().len();
        let drafts = self
            .app
            .cards()
            .iter()
            .filter(|draft| draft.artifacts().all_ready() && draft.staged_rewrite().is_none())
            .cloned()
            .collect::<Vec<_>>();
        let cards = drafts.len();
        let failed = total.saturating_sub(cards);
        let workflow = self.workflow.clone();
        let (sender, receiver) = channel();
        let progress = StudyPublishProgress::new(sender.clone());
        let handle = thread::spawn(move || {
            let outcome = workflow.publish(&drafts, &progress);
            let _ = sender.send(StudyPublishMessage::Done(outcome));
        });
        self.publish_job = Some(PendingPublishJob {
            job: PendingJob {
                receiver,
                handle,
                started: Instant::now(),
            },
            cards,
            failed,
            stopped,
        });
        self.app = self.app.clone().busy_started(BusyKind::PublishingDeck);
        Ok(true)
    }

    fn poll_publish(&mut self) -> Result<bool> {
        let Some(started) = self.publish_job.as_ref().map(|job| job.job.started) else {
            return Ok(false);
        };
        let mut changed = self.refresh_busy_elapsed(started);
        let Some(job) = self.publish_job.as_ref() else {
            return Ok(changed);
        };
        loop {
            let message = match job.job.receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return Ok(changed),
                Err(TryRecvError::Disconnected) => {
                    let job = self
                        .publish_job
                        .take()
                        .expect("invariant: publish job must exist");
                    let message = join_thread(job.job.handle)
                        .map(|()| String::from("background publish job disconnected"))
                        .unwrap_or_else(|error| error.to_string());
                    self.publish_failed(job.stopped, message);
                    return Ok(true);
                }
            };
            match message {
                StudyPublishMessage::Phase(phase) => {
                    let kind = match phase {
                        PublishPhase::Deck => BusyKind::PublishingDeck,
                        PublishPhase::Report => BusyKind::PublishingReport,
                    };
                    self.app = self.app.clone().busy_kind_swapped(kind);
                    changed = true;
                }
                StudyPublishMessage::Done(result) => {
                    let job = self
                        .publish_job
                        .take()
                        .expect("invariant: publish job must exist");
                    if let Err(error) = join_thread(job.job.handle) {
                        self.publish_failed(job.stopped, error.to_string());
                        return Ok(true);
                    }
                    self.app = self.app.clone().busy_finished();
                    match result {
                        Ok(package) => {
                            let (deck, report, output) = package.into_paths();
                            self.app = self.app.clone().done_published_counted(
                                deck, report, output, job.cards, job.failed,
                            );
                            self.engine = None;
                            self.started = None;
                        }
                        Err(error) => {
                            self.publish_failed(job.stopped, error.to_string());
                        }
                    }
                    return Ok(true);
                }
            }
        }
    }

    fn finish_generation_stop(&mut self) -> Result<bool> {
        self.engine = None;
        self.started = None;
        self.regeneration_pending = false;
        self.app = self.app.clone().cards_running(None);
        if self
            .app
            .cards()
            .iter()
            .any(|draft| draft.artifacts().all_ready() && draft.staged_rewrite().is_none())
        {
            let _ = self.start_publish(true)?;
            return Ok(true);
        }
        self.app = self.app.clone().generation_cancellation_started();
        self.cancel_stopped_generation(None);
        Ok(true)
    }

    fn publish_failed(&mut self, stopped: bool, message: String) {
        self.app = self.app.clone().busy_finished();
        self.engine = None;
        self.started = None;
        if stopped {
            self.app = self.app.clone().generation_cancellation_started();
            self.cancel_stopped_generation(Some(message));
        } else {
            self.app = self.app.clone().error_shown(message);
        }
    }

    fn cancel_stopped_generation(&mut self, notice: Option<String>) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.cancel_and_start_next(&self.app, self.output.as_path())
        {
            let message = match notice {
                Some(notice) => format!("{notice}; session not cancelled: {error:#}"),
                None => format!("session not cancelled: {error:#}"),
            };
            self.app = self.app.clone().error_shown(message);
            return true;
        }
        self.app = self.app.clone().generation_cancelled_to_review();
        if let Some(notice) = notice {
            self.app = self.app.clone().error_shown(notice);
        }
        true
    }
}

fn join_thread(handle: JoinHandle<()>) -> Result<()> {
    if handle.join().is_err() {
        bail!("background task panicked");
    }
    Ok(())
}

fn animation_elapsed(elapsed: Duration) -> Duration {
    let frame = u128::from(ANIMATION_FRAME_MILLIS);
    let millis = (elapsed.as_millis() / frame).saturating_mul(frame);
    let Ok(milliseconds) = u64::try_from(millis) else {
        return Duration::from_millis(u64::MAX);
    };
    Duration::from_millis(milliseconds)
}

fn drafts_from(app: &App) -> Vec<CardDraft> {
    let drafts = app
        .candidates()
        .iter()
        .filter(|candidate| candidate.ok())
        .flat_map(|candidate| {
            candidate
                .selected_senses()
                .iter()
                .filter_map(|index| candidate.senses().get(*index))
                .map(|sense| {
                    CardDraft::new(candidate.term(), sense.understanding(), app.pair().clone())
                })
        })
        .collect::<Vec<_>>();
    let count = drafts.len();
    drafts
        .into_iter()
        .zip(app.sentence_settings().selections(count))
        .map(|(draft, request)| match request {
            Some(request) => draft.requesting_meta(request),
            None => draft,
        })
        .collect()
}

fn artifact_rejects_key(outcome: &ArtifactOutcome) -> bool {
    match outcome {
        ArtifactOutcome::Meta(attempt) => attempt.error().is_some_and(rejects_key),
        ArtifactOutcome::Media(attempt) => attempt.error().is_some_and(rejects_key),
    }
}

fn clear_saved_key_in(store: &PreferenceStore) -> Result<()> {
    store.update(|prefs| prefs.without_api_key())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::application::{
        BulkCorrection, CardCorrection, CardMetaGeneration, CardProduction, CardUseCases,
        KeyValidation, PublishedStudyPackage, StudyPublishing, Understanding,
    };

    use super::super::jobs::TextOutcome;
    use super::*;
    use crate::cli::session::{DraftRecord, Phase, ResultRecord, SessionRecord, SessionStore};
    use crate::session::{
        ARTIFACT_ATTEMPT_CEILING, ArtifactCosts, ArtifactFile, ArtifactSlot, CandidateRecord,
        CardArtifacts, CardMeta, CardRevision, GenerationCost, LanguagePair, LearningDetection,
        RawInputBatch, ScriptDetection, Sense, SenseCorrection, SentenceBatchSettings,
        SentenceLabelSelection, SentenceLevel, SentenceTypeMix, Understood, WordCandidate,
        catalog_for_detection,
    };
    use crate::tui::{LabelEditorRow, ModalKind};
    use anyhow::Result;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestFailure {
        Internal,
        Key,
    }

    #[derive(Clone, Debug, Default)]
    struct TestWorkflow {
        failure: Option<TestFailure>,
        publish_fails: bool,
        calls: Option<Arc<AtomicUsize>>,
        _directory: Option<Arc<tempfile::TempDir>>,
    }

    impl TestWorkflow {
        fn local() -> Self {
            Self {
                failure: None,
                publish_fails: false,
                calls: None,
                _directory: None,
            }
        }

        fn publish_failing() -> Self {
            Self {
                failure: None,
                publish_fails: true,
                calls: None,
                _directory: None,
            }
        }

        fn failing() -> Self {
            Self {
                failure: Some(TestFailure::Internal),
                publish_fails: false,
                calls: None,
                _directory: None,
            }
        }

        fn key_rejecting() -> Self {
            Self {
                failure: Some(TestFailure::Key),
                publish_fails: false,
                calls: None,
                _directory: None,
            }
        }

        fn counting(calls: Arc<AtomicUsize>) -> Self {
            Self {
                failure: None,
                publish_fails: false,
                calls: Some(calls),
                _directory: None,
            }
        }

        fn local_meta(term: &str, understanding: &str) -> CardMeta {
            CardMeta::new(
                format!("/{term}/"),
                format!("/{term} sentence/"),
                format!("local meaning of {term}"),
                5,
                format!("local source for {term} ({understanding})"),
                term,
                format!("vivid cue for {term}"),
                format!("usage notes for {term}"),
                format!("Example with {term}."),
            )
        }

        fn failed<T>(&self) -> Result<T> {
            match self.failure {
                Some(TestFailure::Key) => Err(anyhow::anyhow!(crate::gemini::GeminiApiError::new(
                    "UNAUTHENTICATED",
                    Some(String::from("API key not valid")),
                    Vec::new(),
                ))),
                _ => Err(anyhow::anyhow!("INTERNAL: boom")),
            }
        }

        fn ready(&self) -> Result<()> {
            if let Some(calls) = self.calls.as_ref() {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if self.failure.is_some() {
                return self.failed();
            }
            Ok(())
        }
    }

    impl Understanding for TestWorkflow {
        fn understand(
            &self,
            raw: &RawInputBatch,
            my: &str,
            _target: &LearningTarget,
        ) -> Result<Understood> {
            self.ready()?;
            let guess = ScriptDetection.detect(raw.text(), &catalog_for_detection())?;
            let candidates = raw
                .text()
                .lines()
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(|entry| {
                    WordCandidate::new(
                        entry,
                        format!("local-fake understanding for {entry} (support {my})"),
                        true,
                    )
                })
                .collect();
            Ok(Understood::new(guess, candidates))
        }
    }

    impl BulkCorrection for TestWorkflow {
        fn correct_bulk(
            &self,
            candidate: &WordCandidate,
            comment: &str,
            _pair: &LanguagePair,
        ) -> Result<SenseCorrection> {
            self.ready()?;
            Ok(SenseCorrection::adding(vec![Sense::plain(format!(
                "{} · {}",
                candidate.understanding(),
                comment
            ))]))
        }
    }

    impl CardMetaGeneration for TestWorkflow {
        fn generate_card_meta(
            &self,
            term: &str,
            understanding: &str,
            _pair: &LanguagePair,
            _request: Option<&SentenceLabelSelection>,
        ) -> Result<CardMeta> {
            self.ready()?;
            Ok(Self::local_meta(term, understanding))
        }
    }

    impl CardCorrection for TestWorkflow {
        fn correct_card(
            &self,
            draft: &CardDraft,
            comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            self.ready()?;
            let understanding = format!("{} · change: {comment}", draft.understanding());
            let meta = Self::local_meta(draft.term(), understanding.as_str());
            Ok(CardRevision::new(draft.term(), understanding, meta))
        }

        fn correct_card_accounted(
            &self,
            draft: &CardDraft,
            comment: &str,
            pair: &LanguagePair,
        ) -> ArtifactAttempt<CardRevision> {
            ArtifactAttempt::new(
                self.correct_card(draft, comment, pair),
                Some(GenerationCost::from_nanos(123_000)),
            )
        }
    }

    impl CardProduction for TestWorkflow {
        fn generate_meta_in(
            &self,
            _slot: usize,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            request: Option<&SentenceLabelSelection>,
        ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
            let result = self
                .generate_card_meta(term, understanding, pair, request)
                .and_then(|meta| {
                    self.store_card_meta(term, understanding, pair, &meta)
                        .map(|file| (meta, Some(file)))
                });
            ArtifactAttempt::unmetered(result)
        }

        fn generate_draft_meta_in(
            &self,
            slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
            let Some(rewrite) = draft.rewrite() else {
                let term = draft.term().to_string();
                let understanding = draft.understanding().to_string();
                return self
                    .generate_meta_in(
                        slot,
                        draft.term(),
                        draft.understanding(),
                        draft.pair(),
                        draft.meta_request(),
                    )
                    .map(|(meta, file)| (CardRevision::new(term, understanding, meta), file));
            };
            let (result, cost) = self
                .correct_card_accounted(draft, rewrite.note(), draft.pair())
                .into_parts();
            let result = result.and_then(|revision| {
                let file = self.store_card_meta(
                    revision.term(),
                    revision.understanding(),
                    draft.pair(),
                    revision.meta(),
                )?;
                Ok((revision, Some(file)))
            });
            ArtifactAttempt::new(result, cost)
        }

        fn generate_scene_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(
                self.ready()
                    .and_then(|()| local_artifact(draft, Artifact::Scene)),
            )
        }

        fn generate_picture_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(
                self.ready()
                    .and_then(|()| local_artifact(draft, Artifact::Picture)),
            )
        }

        fn generate_sound_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(
                self.ready()
                    .and_then(|()| local_artifact(draft, Artifact::Sound)),
            )
        }

        fn store_card_meta(
            &self,
            term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
            _meta: &CardMeta,
        ) -> Result<ArtifactFile> {
            self.ready()?;
            let name = format!("{}-meta.local.json", slug(term));
            let path = std::env::temp_dir().join(&name);
            Ok(ArtifactFile::new(name, path, "1 B", false))
        }
    }

    impl StudyPublishing for TestWorkflow {
        fn publish(
            &self,
            drafts: &[CardDraft],
            progress: &dyn PublishProgress,
        ) -> Result<PublishedStudyPackage> {
            self.ready()?;
            if self.publish_fails {
                anyhow::bail!("publish boom");
            }
            progress.advance(PublishPhase::Report);
            Ok(PublishedStudyPackage::new(
                format!("local-{}-cards.apkg", drafts.len()),
                format!("local-{}-cards.pdf", drafts.len()),
                String::from("/tmp/local-out"),
            ))
        }
    }

    impl KeyValidation for TestWorkflow {
        fn check_key(&self, _key: &str) -> Result<()> {
            self.ready()
        }
    }

    fn local_artifact(draft: &CardDraft, artifact: Artifact) -> Result<ArtifactFile> {
        let name = format!("{}-{}.local", slug(draft.term()), artifact.label());
        let path = std::env::temp_dir().join(&name);
        Ok(ArtifactFile::new(name, path, "1 B", false))
    }

    fn slug(value: &str) -> String {
        let mut out = String::new();
        for character in value.chars() {
            if character.is_ascii_alphanumeric() {
                out.push(character.to_ascii_lowercase());
            } else if !out.ends_with('-') {
                out.push('-');
            }
        }
        let trimmed = out.trim_matches('-');
        if trimmed.is_empty() {
            return String::from("card");
        }
        String::from(trimmed)
    }

    fn shell(app: App) -> Shell<TestWorkflow, TestWorkflow> {
        shell_with(app, TestWorkflow::local())
    }

    fn failing_shell(app: App) -> Shell<TestWorkflow, TestWorkflow> {
        shell_with(app, TestWorkflow::failing())
    }

    fn key_rejecting_shell(app: App) -> Shell<TestWorkflow, TestWorkflow> {
        shell_with(app, TestWorkflow::key_rejecting())
    }

    fn shell_with(app: App, workflow: TestWorkflow) -> Shell<TestWorkflow, TestWorkflow> {
        let directory = Arc::new(tempfile::tempdir().expect("preference tempdir must exist"));
        let store = PreferenceStore::at(directory.path().join("preferences.json"));
        let mut workflow = workflow;
        workflow._directory = Some(directory);
        Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow: workflow.clone(),
            keys: workflow,
            store,
            session: None,
            output: std::env::temp_dir(),
        }
    }

    fn pair() -> LanguagePair {
        LanguagePair::new("en", "ru")
    }

    fn candidate(term: &str) -> WordCandidate {
        WordCandidate::new(term, format!("local-fake understanding for {term}"), true)
    }

    fn skipped(term: &str) -> WordCandidate {
        WordCandidate::new(term, "not in target language", false)
    }

    fn review() -> App {
        App::new(pair())
            .with_screen(Screen::WhatIUnderstood)
            .confirmed_learning("en")
            .understood(vec![candidate("whilst")])
    }

    #[test]
    fn drafts_expand_batch_sentence_settings_after_candidate_selection() {
        let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Varied);
        let app = App::new(pair())
            .with_screen(Screen::WhatIUnderstood)
            .confirmed_learning("en")
            .understood(vec![
                candidate("alpha"),
                skipped("skip"),
                candidate("beta"),
                candidate("gamma"),
                candidate("delta"),
                candidate("epsilon"),
            ])
            .with_sentence_settings(settings);
        let drafts = drafts_from(&app);
        let requests = drafts
            .iter()
            .map(|draft| draft.meta_request().cloned())
            .collect::<Vec<_>>();
        assert_eq!(
            requests,
            settings.selections(5),
            "generation drafts must receive the exact per-card allocation after excluded rows are removed"
        );
    }

    #[test]
    fn default_batch_sentence_settings_leave_generation_requests_empty() {
        let requests = drafts_from(&review())
            .iter()
            .map(|draft| draft.meta_request().cloned())
            .collect::<Vec<_>>();
        assert_eq!(
            requests,
            vec![None],
            "natural unlevelled batches must preserve the existing unconstrained metadata request"
        );
    }

    fn finished(screen: Screen) -> App {
        let pair = pair();
        App::new(pair.clone())
            .confirmed_learning("en")
            .with_screen(screen)
            .cards_started(vec![CardDraft::new("whilst", "although", pair)])
            .done_published("/tmp/cards.apkg", "/tmp/cards.pdf", "/tmp")
    }

    fn failed_without_publication() -> App {
        let pair = pair();
        let mut picture = ArtifactSlot::fresh(Artifact::Picture);
        for _ in 0..ARTIFACT_ATTEMPT_CEILING {
            picture = picture.attempted();
        }
        let artifacts = CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded(),
            ArtifactSlot::fresh(Artifact::Scene).succeeded(),
            picture,
            ArtifactSlot::fresh(Artifact::Sound).succeeded(),
        );
        App::new(pair.clone())
            .confirmed_learning("en")
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("whilst", "although", pair).with_artifacts(artifacts),
            ])
    }

    fn ready_artifacts() -> CardArtifacts {
        CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded(),
            ArtifactSlot::fresh(Artifact::Scene).succeeded(),
            ArtifactSlot::fresh(Artifact::Picture).succeeded(),
            ArtifactSlot::fresh(Artifact::Sound).succeeded(),
        )
    }

    fn settle_shell<P, K>(shell: &mut Shell<P, K>, max_ticks: usize)
    where
        P: CardUseCases,
        K: KeyValidation,
    {
        for _ in 0..max_ticks {
            shell.tick().expect("shell tick must succeed");
            if shell.engine.is_none()
                && shell.artifact_job.is_none()
                && shell.publish_job.is_none()
                && shell.text.is_none()
            {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("shell did not settle before the deadline");
    }

    #[test]
    fn first_pass_keeps_commas_inside_lines() {
        let mut shell = shell(App::new(pair()).seeded_blob("whilst, in the end\nwreck"));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must run understanding");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.candidates().len(),
                shell.app.candidates()[0].term(),
            ),
            (Screen::WhatIUnderstood, 2, "whilst, in the end"),
            "first pass must split only by lines and keep commas literal"
        );
    }

    #[test]
    fn first_escape_on_published_cards_only_arms_a_new_batch() {
        let mut shell = shell(finished(Screen::YourCards));
        let consumed = shell
            .handle_new_batch_escape()
            .expect("first Escape must be handled");
        assert_eq!(
            (
                consumed,
                shell.app.screen(),
                shell.app.new_batch_pending(),
                shell.app.cards().len(),
                shell.app.done_artifacts().deck.as_str(),
            ),
            (true, Screen::YourCards, true, 1, "/tmp/cards.apkg"),
            "first Escape reset a completed batch without confirmation"
        );
    }

    #[test]
    fn second_escape_on_reopened_done_starts_clean_words_in_place() {
        let mut shell = shell(finished(Screen::Done));
        let output = shell.output.clone();
        shell
            .handle_new_batch_escape()
            .expect("first Escape must arm the reset");
        shell
            .handle_new_batch_escape()
            .expect("second Escape must confirm the reset");
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.pair().known(),
                shell.app.learning_pending(),
                shell.app.blob(),
                shell.app.cards().len(),
                shell.app.done_artifacts().deck.as_str(),
                shell.app.new_batch_pending(),
                shell.output.as_path(),
            ),
            (
                Screen::YourWords,
                "ru",
                true,
                "",
                0,
                "",
                false,
                output.as_path()
            ),
            "confirmed Escape did not replace Done with a clean batch in the same output"
        );
    }

    #[test]
    fn new_batch_confirmation_expires_after_the_quit_window() {
        let mut shell = shell(finished(Screen::YourCards));
        shell
            .handle_new_batch_escape()
            .expect("first Escape must arm the reset");
        shell.new_batch_armed_at =
            Some(Instant::now() - CONFIRMATION_WINDOW - Duration::from_millis(1));
        let changed = shell.refresh_new_batch_pending();
        assert_eq!(
            (
                changed,
                shell.app.new_batch_pending(),
                shell.new_batch_armed_at
            ),
            (true, false, None),
            "an expired Escape confirmation remained armed"
        );
    }

    #[test]
    fn another_action_disarms_the_new_batch_confirmation() {
        let mut shell = shell(finished(Screen::YourCards));
        shell
            .handle_new_batch_escape()
            .expect("first Escape must arm the reset");
        let changed = shell.disarm_new_batch();
        assert_eq!(
            (changed, shell.app.new_batch_pending(), shell.app.screen()),
            (true, false, Screen::YourCards),
            "another action left the destructive Escape confirmation armed"
        );
    }

    #[test]
    fn first_escape_on_words_only_arms_the_clear() {
        let mut shell = shell(App::new(pair()).seeded_blob("whilst\nwreck"));
        let side = shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must be handled");
        assert_eq!(
            (
                side,
                shell.app.blob(),
                shell.app.word_clear_pending(),
                shell.destructive_escape_armed_at.map(|(action, _)| action),
            ),
            (
                Side::ClearWords,
                "whilst\nwreck",
                true,
                Some(DestructiveEscape::ClearWords),
            ),
            "first Escape erased words instead of arming their clear"
        );
    }

    #[test]
    fn second_escape_on_words_clears_the_field() {
        let mut shell = shell(App::new(pair()).seeded_blob("whilst\nwreck"));
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the clear");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the clear");
        assert_eq!(
            (
                shell.app.blob(),
                shell.app.word_clear_pending(),
                shell.destructive_escape_armed_at,
            ),
            ("", false, None),
            "confirmed Escape left words or their clear intent behind"
        );
    }

    #[test]
    fn confirmed_words_clear_forgets_the_hidden_review() {
        let mut shell = shell(
            review()
                .seeded_blob("whilst")
                .with_screen(Screen::YourWords),
        );
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the clear");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the clear");
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.blob(),
                shell.app.candidates().len(),
                shell.app.cards().len(),
                shell.app.learning_pending(),
            ),
            (Screen::YourWords, "", 0, 0, true),
            "confirmed words clear kept hidden review state for the next batch"
        );
    }

    #[test]
    fn words_clear_confirmation_expires_after_the_confirmation_window() {
        let mut shell = shell(App::new(pair()).seeded_blob("whilst"));
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the clear");
        shell.destructive_escape_armed_at = Some((
            DestructiveEscape::ClearWords,
            Instant::now() - CONFIRMATION_WINDOW - Duration::from_millis(1),
        ));
        let changed = shell.refresh_destructive_escape_pending();
        assert_eq!(
            (
                changed,
                shell.app.blob(),
                shell.app.word_clear_pending(),
                shell.destructive_escape_armed_at,
            ),
            (true, "whilst", false, None),
            "an expired words-clear confirmation stayed armed"
        );
    }

    #[test]
    fn another_key_disarms_words_clear_before_editing() {
        let mut shell = shell(App::new(pair()).seeded_blob("whilst"));
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the clear");
        shell
            .handle(AppEvent::KeyChar('x'))
            .expect("typing must stay available");
        assert_eq!(
            (
                shell.app.blob(),
                shell.app.word_clear_pending(),
                shell.destructive_escape_armed_at,
            ),
            ("whilstx", false, None),
            "typing inherited a stale words-clear confirmation"
        );
    }

    #[test]
    fn first_escape_during_generation_only_arms_the_stop() {
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        let side = shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must be handled");
        assert_eq!(
            (
                side,
                shell.app.generation_stop_pending(),
                shell.app.generation_stopping(),
                shell.engine.is_some(),
            ),
            (Side::StopGeneration, true, false, true),
            "first Escape stopped generation without confirmation"
        );
    }

    #[test]
    fn second_escape_starts_draining_without_dropping_the_engine() {
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        assert_eq!(
            (
                shell.app.generation_stop_pending(),
                shell.app.generation_stopping(),
                shell.engine.is_some(),
                shell.regeneration_pending,
            ),
            (false, true, true, false),
            "confirmed stop discarded the engine before its active request drained"
        );
    }

    #[test]
    fn generation_stop_confirmation_expires_without_stopping_the_engine() {
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell.destructive_escape_armed_at = Some((
            DestructiveEscape::StopGeneration,
            Instant::now() - CONFIRMATION_WINDOW - Duration::from_millis(1),
        ));
        let changed = shell.refresh_destructive_escape_pending();
        assert_eq!(
            (
                changed,
                shell.app.generation_stop_pending(),
                shell.app.generation_stopping(),
                shell.engine.is_some(),
            ),
            (true, false, false, true),
            "an expired stop confirmation stayed armed or stopped the engine"
        );
    }

    #[test]
    fn another_key_disarms_generation_stop_without_stopping_the_engine() {
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::NavNext)
            .expect("navigation must stay available");
        assert_eq!(
            (
                shell.app.generation_stop_pending(),
                shell.app.generation_stopping(),
                shell.destructive_escape_armed_at,
                shell.engine.is_some(),
            ),
            (false, false, None, true),
            "navigation inherited a stale stop confirmation or stopped the engine"
        );
    }

    #[test]
    fn confirmed_stop_without_ready_cards_returns_to_review() {
        let mut shell = shell(review().seeded_blob("whilst"));
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        shell
            .tick()
            .expect("stop must finish between artifact jobs");
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.blob(),
                shell.app.candidates().len(),
                shell.app.cards().len(),
                shell.app.generation_stopping(),
                shell.engine.is_none(),
            ),
            (Screen::WhatIUnderstood, "whilst", 1, 0, false, true),
            "zero-ready stop lost review state or left a committed engine behind"
        );
    }

    #[test]
    fn stop_applies_the_inflight_outcome_then_publishes_only_ready_cards() {
        let calls = Arc::new(AtomicUsize::new(0));
        let language_pair = pair();
        let first = CardDraft::new("alpha", "first understanding", language_pair.clone())
            .with_artifacts(ready_artifacts());
        let second = CardDraft::new("beta", "second understanding", language_pair.clone());
        let app = App::new(language_pair)
            .with_screen(Screen::YourCards)
            .cards_started(vec![first.clone(), second.clone()])
            .cards_running(Some((1, Artifact::Meta)));
        let mut shell = shell_with(app, TestWorkflow::counting(calls.clone()));
        shell.engine = Some(SessionEngine::start(vec![first, second]));
        let settled = TestWorkflow::local_meta("beta", "second understanding");
        let (release, waiting) = channel();
        shell.artifact_job = Some(PendingArtifactJob {
            job: PendingJob::spawn(move || {
                waiting
                    .recv_timeout(Duration::from_secs(1))
                    .expect("in-flight artifact must be released");
                ArtifactOutcome::Meta(Box::new(ArtifactAttempt::new(
                    Ok((
                        CardRevision::new("beta", "second understanding", settled),
                        None,
                    )),
                    Some(GenerationCost::from_nanos(91_000)),
                )))
            }),
            card: 1,
            artifact: Artifact::Meta,
        });
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        release.send(()).expect("in-flight artifact must resume");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.cards()[1].meta().is_some(),
                shell.app.done_artifacts().cards,
                shell.app.done_artifacts().failed,
                shell.app.generation_stopping(),
                calls.load(Ordering::SeqCst),
            ),
            (true, 1, 1, false, 1),
            "stop lost the in-flight result, started another artifact, or published the wrong tally"
        );
    }

    #[test]
    fn stopped_inflight_key_rejection_does_not_resume_generation() {
        let language_pair = pair();
        let draft = CardDraft::new("alpha", "first understanding", language_pair.clone());
        let app = App::new(language_pair)
            .with_screen(Screen::YourCards)
            .understood(vec![candidate("alpha")])
            .cards_started(vec![draft.clone()])
            .cards_running(Some((0, Artifact::Meta)));
        let mut shell = shell(app);
        shell.engine = Some(SessionEngine::start(vec![draft]));
        let (release, waiting) = channel();
        shell.artifact_job = Some(PendingArtifactJob {
            job: PendingJob::spawn(move || {
                waiting
                    .recv_timeout(Duration::from_secs(1))
                    .expect("in-flight artifact must be released");
                let error = anyhow::anyhow!(crate::gemini::GeminiApiError::new(
                    "UNAUTHENTICATED",
                    Some(String::from("API key not valid")),
                    Vec::new(),
                ));
                ArtifactOutcome::Meta(Box::new(ArtifactAttempt::unmetered(Err(error))))
            }),
            card: 0,
            artifact: Artifact::Meta,
        });
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        release.send(()).expect("in-flight artifact must resume");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.cards().len(),
                shell.app.generation_stopping(),
                shell.engine.is_none(),
                shell.app.welcome().notice.clone(),
            ),
            (Screen::WhatIUnderstood, 0, false, true, None),
            "a rejected in-flight request reopened key recovery or resumed a confirmed stop"
        );
    }

    #[test]
    fn failed_publish_after_stop_returns_to_review_without_resuming() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let language_pair = pair();
        let candidates = vec![candidate("alpha"), candidate("beta")];
        let ready = CardDraft::new("alpha", "first understanding", language_pair.clone())
            .with_artifacts(ready_artifacts());
        let incomplete = CardDraft::new("beta", "second understanding", language_pair.clone());
        let drafts = vec![ready, incomplete];
        let app = App::new(language_pair)
            .seeded_blob("alpha\nbeta")
            .confirmed_learning("en")
            .with_screen(Screen::YourCards)
            .understood(candidates.clone())
            .cards_started(drafts.clone());
        let mut record = SessionRecord::understood(
            String::from("stopped-publish"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha"), String::from("beta")],
            candidates
                .iter()
                .map(CandidateRecord::from_candidate)
                .collect(),
        );
        record.drafts = drafts
            .iter()
            .map(|draft| DraftRecord {
                term: draft.term().to_string(),
                understanding: draft.understanding().to_string(),
                costs: ArtifactCosts::default(),
                rewrite: None,
            })
            .collect();
        store.create(&record).expect("the session must persist");
        let session = TuiSession::resuming_in(&record, store).expect("the session must resume");
        let mut shell = shell_with(app, TestWorkflow::publish_failing());
        shell.session = Some(session);
        shell.output = home.path().to_path_buf();
        shell.engine = Some(SessionEngine::start(drafts));
        let claimed = shell.claim_session();
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        settle_shell(&mut shell, 200);
        shell.persist();
        let old: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/stopped-publish/session.json"))
                .expect("the cancelled record must read")
                .as_slice(),
        )
        .expect("the cancelled record must decode");
        let mut next = std::fs::read_dir(home.path().join("sessions"))
            .expect("the sessions directory must list")
            .flatten()
            .filter(|entry| entry.file_name() != "stopped-publish")
            .map(|entry| {
                serde_json::from_slice::<SessionRecord>(
                    std::fs::read(entry.path().join("session.json"))
                        .expect("the next record must read")
                        .as_slice(),
                )
                .expect("the next record must decode")
            })
            .collect::<Vec<_>>();
        let new = next.pop().expect("the preserved review must persist");
        assert_eq!(
            (
                shell.app.screen(),
                claimed,
                shell.app.cards().len(),
                shell.app.generation_stopping(),
                shell.engine.is_none(),
                shell.app.error(),
                old.phase,
                old.worker.is_none(),
                new.phase,
                new.drafts.len(),
                new.worker.is_none(),
                next.is_empty(),
            ),
            (
                Screen::WhatIUnderstood,
                true,
                0,
                false,
                true,
                Some("publish boom"),
                Phase::Cancelled,
                true,
                Phase::Understood,
                0,
                true,
                true,
            ),
            "a stopped publish failure left drafts able to auto-resume or reused its identity"
        );
    }

    #[test]
    fn stopped_publish_claim_conflict_keeps_live_ownership_and_cannot_resume() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let language_pair = pair();
        let candidates = vec![candidate("alpha"), candidate("beta")];
        let ready = CardDraft::new("alpha", "first understanding", language_pair.clone())
            .with_artifacts(ready_artifacts());
        let incomplete = CardDraft::new("beta", "second understanding", language_pair.clone());
        let drafts = vec![ready, incomplete];
        let app = App::new(language_pair)
            .seeded_blob("alpha\nbeta")
            .confirmed_learning("en")
            .with_screen(Screen::YourCards)
            .understood(candidates.clone())
            .cards_started(drafts.clone());
        let mut record = SessionRecord::understood(
            String::from("stop-publish-race"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha"), String::from("beta")],
            candidates
                .iter()
                .map(CandidateRecord::from_candidate)
                .collect(),
        );
        record.drafts = drafts
            .iter()
            .map(|draft| DraftRecord {
                term: draft.term().to_string(),
                understanding: draft.understanding().to_string(),
                costs: ArtifactCosts::default(),
                rewrite: None,
            })
            .collect();
        store.create(&record).expect("the session must persist");
        let session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut shell = shell_with(app, TestWorkflow::counting(calls.clone()));
        shell.session = Some(session);
        shell.output = home.path().to_path_buf();
        shell.engine = Some(SessionEngine::start(drafts));
        let claimed = shell.claim_session();
        store
            .update("stop-publish-race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing edit must persist");
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        shell.tick().expect("the stopped publish must try to claim");
        shell.persist();
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/stop-publish-race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let lock_available = store
            .hold("stop-publish-race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                shell.app.generation_stopping(),
                claimed,
                shell.publish_job.is_none(),
                shell.engine.is_none(),
                calls.load(Ordering::SeqCst),
                stored.phase,
                stored.senses,
                stored.drafts.len(),
                lock_available,
            ),
            (
                true,
                true,
                true,
                true,
                0,
                Phase::Generating,
                String::from("all"),
                2,
                false,
            ),
            "a stopped claim conflict released ownership, rewrote the record, or resumed work"
        );
    }

    #[test]
    fn confirmed_stop_omits_a_ready_card_with_an_uncommitted_rewrite() {
        let language_pair = pair();
        let clean = CardDraft::new("alpha", "first understanding", language_pair.clone())
            .with_artifacts(ready_artifacts());
        let staged = CardDraft::new("beta", "second understanding", language_pair.clone())
            .with_artifacts(ready_artifacts())
            .staging_rewrite(SentenceLabelSelection::empty(), "make beta formal");
        let incomplete = CardDraft::new("gamma", "third understanding", language_pair.clone());
        let drafts = vec![clean, staged, incomplete];
        let app = App::new(language_pair)
            .with_screen(Screen::YourCards)
            .cards_started(drafts.clone());
        let mut shell = shell_with(app, TestWorkflow::local());
        shell.engine = Some(SessionEngine::start(drafts));
        shell
            .handle(AppEvent::Cancel)
            .expect("first Escape must arm the stop");
        shell
            .handle(AppEvent::Cancel)
            .expect("second Escape must confirm the stop");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.done_artifacts().deck.as_str(),
                shell.app.done_artifacts().cards,
                shell.app.done_artifacts().failed,
            ),
            ("local-1-cards.apkg", 1, 2),
            "partial stop published a card whose rewrite was still staged"
        );
    }

    #[test]
    fn escape_closes_an_open_sentence_editor_before_arming_a_new_batch() {
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Generate)
            .expect("generation must start");
        settle_shell(&mut shell, 200);
        shell
            .handle(AppEvent::SentenceLabelFocus(LabelEditorRow::Register))
            .expect("sentence editor must open");
        let intercepted = shell
            .handle_new_batch_escape()
            .expect("Escape eligibility must be checked");
        let side = shell
            .handle(AppEvent::Cancel)
            .expect("Escape must close the editor");
        assert_eq!(
            (
                intercepted,
                side,
                shell.app.sentence_editor().is_none(),
                shell.app.screen(),
                shell.app.done_artifacts().deck.is_empty(),
            ),
            (false, Side::None, true, Screen::YourCards, false),
            "Escape armed or cleared a published batch while its sentence editor was open"
        );
    }

    #[test]
    fn double_escape_starts_over_after_every_card_gives_up_before_publication() {
        let mut shell = shell(failed_without_publication());
        shell
            .handle_new_batch_escape()
            .expect("first Escape must arm the reset");
        shell
            .handle_new_batch_escape()
            .expect("second Escape must reset the failed batch");
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.cards().len(),
                shell.app.done_artifacts().deck.as_str(),
            ),
            (Screen::YourWords, 0, ""),
            "a terminally failed unpublished batch still required an app restart"
        );
    }

    #[test]
    fn text_pass_failure_stays_in_the_tui_as_recoverable_error() {
        let mut shell = failing_shell(App::new(pair()).seeded_blob("wreck"));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start understanding");
        settle_shell(&mut shell, 200);
        let before = (
            shell.app.screen(),
            shell.app.blob().to_string(),
            shell.app.busy().is_none(),
            shell.app.error().map(String::from),
        );
        let side = shell
            .handle(AppEvent::KeyChar('x'))
            .expect("dismiss key must not crash");
        assert_eq!(
            (
                before,
                shell.app.screen(),
                shell.app.blob().to_string(),
                shell.app.busy().is_none(),
                shell.app.error().map(String::from),
                side,
            ),
            (
                (
                    Screen::YourWords,
                    String::from("wreck"),
                    true,
                    Some(String::from("INTERNAL: boom")),
                ),
                Screen::YourWords,
                String::from("wreck"),
                true,
                None,
                Side::None,
            ),
            "Gemini text errors must keep the TUI alive, preserve the input, and dismiss cleanly"
        );
    }

    #[test]
    fn bulk_correction_keeps_modal_during_request_and_closes_after_success() {
        let mut shell = shell(review().with_modal(ModalKind::ChangeSomething).typed('x'));
        shell
            .handle(AppEvent::Submit)
            .expect("bulk correction must start");
        let during = (shell.app.modal(), shell.app.busy().map(|busy| busy.kind()));
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                during,
                shell.app.modal(),
                shell.app.busy().is_none(),
                shell.app.candidates()[0].senses().len(),
            ),
            (
                (
                    Some(ModalKind::ChangeSomething),
                    Some(BusyKind::BulkCorrection)
                ),
                None,
                true,
                2,
            ),
            "bulk correction must leave the modal under the loader and close it only after success"
        );
    }

    #[test]
    fn ctrl_g_starts_every_staged_rewrite_and_keeps_unmodified_cards_ready() {
        let mut shell = shell(review().understood(vec![
            candidate("alpha"),
            candidate("beta"),
            candidate("gamma"),
        ]));
        shell
            .handle(AppEvent::Generate)
            .expect("initial generation must start");
        settle_shell(&mut shell, 300);
        let original = shell.app.cards().to_vec();
        let drafts = vec![
            original[0]
                .clone()
                .staging_rewrite(SentenceLabelSelection::empty(), "rewrite alpha"),
            original[1].clone(),
            original[2]
                .clone()
                .staging_rewrite(SentenceLabelSelection::empty(), "rewrite gamma"),
        ];
        shell.app = shell.app.clone().cards_replaced(drafts);
        let side = shell
            .handle(AppEvent::Generate)
            .expect("batch rewrite must start");
        let engine = shell
            .engine
            .as_ref()
            .expect("batch rewrite must install an engine");
        assert_eq!(
            (
                side,
                engine.drafts()[0]
                    .rewrite()
                    .map(crate::session::CardRewrite::started),
                engine.drafts()[0].meta().is_none(),
                engine.drafts()[1] == original[1],
                engine.drafts()[2]
                    .rewrite()
                    .map(crate::session::CardRewrite::started),
                engine.drafts()[2].meta().is_none(),
                engine.next_target(),
            ),
            (
                Side::RegenerateCards,
                Some(true),
                true,
                true,
                Some(true),
                true,
                Some((0, Artifact::Meta)),
            ),
            "Ctrl+G failed to activate every staged rewrite or mutated an unmodified card"
        );
    }

    #[test]
    fn ctrl_g_runs_a_staged_sentence_rewrite_inside_the_artifact_engine() {
        let draft = CardDraft::new("whilst", "local understanding", pair())
            .with_meta(
                TestWorkflow::local_meta("whilst", "local understanding"),
                None,
            )
            .staging_rewrite(SentenceLabelSelection::empty(), "x");
        let mut shell = shell(
            App::new(pair())
                .with_screen(Screen::YourCards)
                .cards_started(vec![draft]),
        );
        shell
            .handle(AppEvent::Generate)
            .expect("sentence rewrite batch must start");
        let during = (shell.app.busy().is_none(), shell.engine.is_some());
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                during,
                shell.app.cards()[0].understanding().contains("change: x"),
                shell.app.cards()[0].rewrite().is_none(),
                shell.app.cards()[0].artifacts().all_ready(),
                shell.app.cards()[0].artifacts().meta().cost(),
            ),
            (
                (true, true),
                true,
                true,
                true,
                Some(GenerationCost::from_nanos(123_000)),
            ),
            "staged sentence rewrite escaped the Ctrl+G artifact-engine batch"
        );
    }

    #[test]
    fn sentence_rewrite_waits_for_an_active_artifact_without_losing_the_request() {
        let draft = CardDraft::new("whilst", "local understanding", pair());
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                draft
                    .clone()
                    .staging_rewrite(SentenceLabelSelection::empty(), "make it formal"),
            ])
            .cards_running(Some((0, Artifact::Meta)));
        let mut shell = shell(app);
        shell.engine = Some(SessionEngine::start(vec![draft]));
        let settled = TestWorkflow::local_meta("whilst", "settled understanding");
        let sentence = settled.target_sentence().to_string();
        let (release, waiting) = channel();
        shell.artifact_job = Some(PendingArtifactJob {
            job: PendingJob::spawn(move || {
                waiting
                    .recv_timeout(Duration::from_secs(1))
                    .expect("old artifact job must be released");
                ArtifactOutcome::Meta(Box::new(ArtifactAttempt::new(
                    Ok((
                        CardRevision::new("whilst", "settled understanding", settled),
                        None,
                    )),
                    Some(GenerationCost::from_nanos(77_000)),
                )))
            }),
            card: 0,
            artifact: Artifact::Meta,
        });
        shell
            .handle(AppEvent::Generate)
            .expect("sentence rewrite must queue");
        release.send(()).expect("old artifact job must resume");
        for _ in 0..50 {
            shell.tick().expect("shell must advance the queued rewrite");
            if !shell.regeneration_pending
                && shell.app.cards_running_target() == Some((0, Artifact::Meta))
                && shell.app.cards()[0]
                    .rewrite()
                    .and_then(|rewrite| rewrite.previous())
                    .is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            (
                shell.app.cards()[0]
                    .rewrite()
                    .and_then(|rewrite| rewrite.previous())
                    .map(CardMeta::target_sentence),
                shell.app.cards()[0].artifacts().meta().cost(),
                shell.app.cards_running_target(),
            ),
            (
                Some(sentence.as_str()),
                Some(GenerationCost::from_nanos(77_000)),
                Some((0, Artifact::Meta)),
            ),
            "an active artifact outcome replaced or bypassed the queued sentence rewrite"
        );
    }

    #[test]
    fn staged_rewrite_survives_another_cards_artifact_outcome_without_ctrl_g() {
        let first = CardDraft::new("alpha", "first understanding", pair());
        let second = CardDraft::new("beta", "second understanding", pair()).with_meta(
            TestWorkflow::local_meta("beta", "second understanding"),
            None,
        );
        let sentence = second
            .meta()
            .map(CardMeta::target_sentence)
            .expect("second card must have current metadata")
            .to_string();
        let staged = second
            .clone()
            .staging_rewrite(SentenceLabelSelection::empty(), "make beta formal");
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(vec![first.clone(), staged])
            .cards_running(Some((0, Artifact::Meta)));
        let mut shell = shell(app);
        shell.engine = Some(SessionEngine::start(vec![first, second]));
        let settled = TestWorkflow::local_meta("alpha", "settled understanding");
        shell.artifact_job = Some(PendingArtifactJob {
            job: PendingJob::spawn(move || {
                ArtifactOutcome::Meta(Box::new(ArtifactAttempt::new(
                    Ok((
                        CardRevision::new("alpha", "settled understanding", settled),
                        None,
                    )),
                    Some(GenerationCost::from_nanos(81_000)),
                )))
            }),
            card: 0,
            artifact: Artifact::Meta,
        });
        for _ in 0..50 {
            shell.tick().expect("shell must settle the other card");
            if shell.app.cards()[0].meta().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            (
                shell.app.cards()[0].artifacts().meta().cost(),
                shell.app.cards()[1].meta().map(CardMeta::target_sentence),
                shell.app.cards()[1]
                    .rewrite()
                    .map(crate::session::CardRewrite::started),
                shell.publish_job.is_none(),
            ),
            (
                Some(GenerationCost::from_nanos(81_000)),
                Some(sentence.as_str()),
                Some(false),
                true,
            ),
            "another card settling erased staged tuning, current metadata, or deferred publication"
        );
    }

    #[test]
    fn staged_app_cannot_publish_an_all_ready_baseline_engine_before_ctrl_g() {
        let artifacts = CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded(),
            ArtifactSlot::fresh(Artifact::Scene).succeeded(),
            ArtifactSlot::fresh(Artifact::Picture).succeeded(),
            ArtifactSlot::fresh(Artifact::Sound).succeeded(),
        );
        let baseline = CardDraft::new("alpha", "first understanding", pair())
            .with_meta(
                TestWorkflow::local_meta("alpha", "first understanding"),
                None,
            )
            .with_artifacts(artifacts);
        let staged = baseline
            .clone()
            .staging_rewrite(SentenceLabelSelection::empty(), "make it formal");
        let calls = Arc::new(AtomicUsize::new(0));
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(vec![staged]);
        let mut shell = shell_with(app, TestWorkflow::counting(calls.clone()));
        shell.engine = Some(SessionEngine::start(vec![baseline]));
        for _ in 0..20 {
            shell.tick().expect("staged shell must remain responsive");
            if calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            (
                shell.app.cards_pending(),
                calls.load(Ordering::SeqCst),
                shell.publish_job.is_none(),
                shell.artifact_job.is_none(),
                shell.engine.is_some(),
                shell.app.done_artifacts().deck.is_empty(),
            ),
            (1, 0, true, true, true, true),
            "an all-ready baseline published while a newer staged edit still awaited Ctrl+G"
        );
    }

    #[test]
    fn failed_sentence_rewrite_keeps_every_retry_cost_in_the_ui_ledger() {
        let costs =
            ArtifactCosts::default().charged(Artifact::Meta, GenerationCost::from_nanos(45_000));
        let draft = CardDraft::new("whilst", "local understanding", pair())
            .with_costs(costs)
            .staging_rewrite(SentenceLabelSelection::empty(), "x");
        let mut shell = failing_shell(
            App::new(pair())
                .with_screen(Screen::YourCards)
                .cards_started(vec![draft]),
        );
        shell
            .handle(AppEvent::Generate)
            .expect("sentence rewrite must start");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.cards()[0].artifacts().meta().cost(),
                shell.app.cards()[0].artifacts().meta().failed_terminally(),
                shell.app.cards()[0].rewrite().is_some(),
            ),
            (Some(GenerationCost::from_nanos(537_000)), true, true),
            "failed sentence rewrite dropped retry spend or forgot its durable request"
        );
    }

    #[test]
    fn key_rejection_returns_to_welcome_key_step() {
        let mut shell = key_rejecting_shell(App::new(pair()).seeded_blob("wreck"));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start understanding");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.welcome().stage,
                shell.app.welcome().source,
                shell.app.welcome().key.to_string(),
                shell.app.welcome().notice.clone(),
                shell.app.blob().to_string(),
                shell.engine.is_none(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::EnterKey,
                KeySource::Empty,
                String::new(),
                Some(String::from(KEY_REJECTED_MESSAGE)),
                String::from("wreck"),
                true,
            ),
            "Gemini key rejection must clear the key field, preserve typed words, and reopen Welcome on the key step"
        );
    }

    #[test]
    fn artifact_key_rejection_pauses_once_without_changing_the_costed_plan() {
        let calls = Arc::new(AtomicUsize::new(0));
        let costs =
            ArtifactCosts::default().charged(Artifact::Scene, GenerationCost::from_nanos(71_000));
        let drafts = vec![
            CardDraft::new("alpha", "first understanding", pair()).with_costs(costs),
            CardDraft::new("beta", "second understanding", pair()),
        ];
        let original = drafts.clone();
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(drafts);
        let mut shell = key_rejecting_shell(app);
        shell.workflow.calls = Some(calls.clone());
        shell.start_engine();
        settle_shell(&mut shell, 200);
        for _ in 0..5 {
            shell.tick().expect("a paused Welcome shell must tick");
        }
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.modal(),
                calls.load(Ordering::SeqCst),
                shell.engine.is_none(),
                shell.artifact_job.is_none(),
                shell.app.cards_running_target(),
                shell.app.cards().to_vec(),
            ),
            (Screen::Welcome, None, 1, true, true, None, original,),
            "key recovery looped provider work, left a modal open, or changed stable card slots"
        );
    }

    #[test]
    fn successful_key_recovery_resumes_the_same_costed_card_plan() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let first =
            ArtifactCosts::default().charged(Artifact::Meta, GenerationCost::from_nanos(41_000));
        let second =
            ArtifactCosts::default().charged(Artifact::Picture, GenerationCost::from_nanos(83_000));
        let drafts = vec![
            CardDraft::new("alpha", "first understanding", pair()).with_costs(first),
            CardDraft::new("beta", "second understanding", pair()).with_costs(second),
        ];
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(drafts)
            .opening_welcome_at(
                WelcomeStage::EnterKey,
                KeySource::Pasted,
                "123456789012345678901234567890",
                false,
            );
        let mut record = SessionRecord::understood(
            String::from("key-recovery"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha"), String::from("beta")],
            Vec::new(),
        );
        record.drafts = vec![
            DraftRecord {
                term: String::from("alpha"),
                understanding: String::from("first understanding"),
                costs: first,
                meta_request: None,
                rewrite: None,
            },
            DraftRecord {
                term: String::from("beta"),
                understanding: String::from("second understanding"),
                costs: second,
                meta_request: None,
                rewrite: None,
            },
        ];
        store
            .create(&record)
            .expect("the committed plan must persist");
        let session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        store
            .cost_journal(&record)
            .charge(0, Artifact::Scene, GenerationCost::from_nanos(59_000))
            .expect("the rejected provider spend must be journaled");
        let mut shell = Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow: TestWorkflow::local(),
            keys: TestWorkflow::local(),
            store: PreferenceStore::at(home.path().join("preferences.json")),
            session: Some(session),
            output: home.path().to_path_buf(),
        };
        shell.finish_text(TextOutcome::KeyCheck(Ok(())));
        let app_plan = shell
            .app
            .cards()
            .iter()
            .map(|draft| {
                (
                    draft.term().to_string(),
                    draft.understanding().to_string(),
                    ArtifactCosts::from_artifacts(draft.artifacts()),
                )
            })
            .collect::<Vec<_>>();
        let engine_plan = shell.engine.as_ref().map(|engine| {
            engine
                .drafts()
                .iter()
                .map(|draft| {
                    (
                        draft.term().to_string(),
                        draft.understanding().to_string(),
                        ArtifactCosts::from_artifacts(draft.artifacts()),
                    )
                })
                .collect::<Vec<_>>()
        });
        let expected = vec![
            (
                String::from("alpha"),
                String::from("first understanding"),
                first.charged(Artifact::Scene, GenerationCost::from_nanos(59_000)),
            ),
            (
                String::from("beta"),
                String::from("second understanding"),
                second,
            ),
        ];
        assert_eq!(
            (shell.app.screen(), app_plan, engine_plan),
            (Screen::YourCards, expected.clone(), Some(expected)),
            "successful key recovery replaced or reordered the committed costed plan"
        );
    }

    #[test]
    fn successful_precommit_key_check_returns_to_words_without_an_engine() {
        let app = review().opening_welcome_at(
            WelcomeStage::EnterKey,
            KeySource::Pasted,
            "123456789012345678901234567890",
            false,
        );
        let mut shell = shell(app);
        shell.finish_text(TextOutcome::KeyCheck(Ok(())));
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.candidates().len(),
                shell.app.cards().is_empty(),
                shell.engine.is_none(),
                shell.app.welcome().notice.clone(),
            ),
            (Screen::YourWords, 1, true, true, None),
            "a precommit key check started generation or discarded the reviewed candidate"
        );
    }

    #[test]
    fn generation_uses_every_selected_understanding() {
        let candidate = WordCandidate::with_selected_senses(
            "bank",
            vec![
                Sense::tagged("Сущ. «банк», финансовое учреждение.", "фин."),
                Sense::plain("Сущ. «берег» реки или водоёма."),
                Sense::tagged("Гл. «наклонять(ся)» при повороте самолёта.", "авиац."),
            ],
            vec![0, 2],
            true,
        );
        let mut shell = shell(
            App::new(pair())
                .with_screen(Screen::WhatIUnderstood)
                .understood(vec![candidate]),
        );
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start");
        assert_eq!(
            (
                shell.app.cards().len(),
                shell.app.cards()[0].understanding(),
                shell.app.cards()[1].understanding()
            ),
            (
                2,
                "Сущ. «банк», финансовое учреждение.",
                "Гл. «наклонять(ся)» при повороте самолёта.",
            ),
            "Ctrl+G must create one card for every selected sense"
        );
    }

    #[test]
    fn a_stale_tui_cannot_start_provider_work_after_its_commit_conflicts() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let candidates = vec![candidate("alpha"), candidate("beta")];
        let record = SessionRecord::understood(
            String::from("race"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha"), String::from("beta")],
            candidates
                .iter()
                .map(CandidateRecord::from_candidate)
                .collect(),
        );
        store.create(&record).expect("the session must persist");
        let session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        store
            .update("race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing edit must persist");
        let calls = Arc::new(AtomicUsize::new(0));
        let app = App::new(pair())
            .with_screen(Screen::WhatIUnderstood)
            .understood(candidates);
        let mut shell = Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow: TestWorkflow::counting(calls.clone()),
            keys: TestWorkflow::local(),
            store: PreferenceStore::at(home.path().join("preferences.json")),
            session: Some(session),
            output: home.path().to_path_buf(),
        };
        let side = shell
            .handle(AppEvent::Generate)
            .expect("the generation request must stay in the TUI");
        for _ in 0..3 {
            shell.tick().expect("an idle conflicted shell must tick");
        }
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let journal = std::fs::read_dir(home.path().join("sessions/race"))
            .expect("the session directory must list")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("costs-"));
        let lock_released = store
            .hold("race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                side,
                calls.load(Ordering::SeqCst),
                shell.engine.is_none(),
                shell.artifact_job.is_none(),
                stored.senses,
                stored.drafts.is_empty(),
                journal,
                lock_released,
            ),
            (
                Side::StartGeneration,
                0,
                true,
                true,
                String::from("all"),
                true,
                false,
                true,
            ),
            "a stale TUI started provider work or poisoned journal slots after losing its save race"
        );
    }

    #[test]
    fn a_stale_tui_cannot_correct_a_card_before_its_commit_succeeds() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let artifacts = CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded(),
            ArtifactSlot::fresh(Artifact::Scene).succeeded(),
            ArtifactSlot::fresh(Artifact::Picture).succeeded(),
            ArtifactSlot::fresh(Artifact::Sound).succeeded(),
        );
        let draft = CardDraft::new("alpha", "first understanding", pair())
            .with_meta(
                TestWorkflow::local_meta("alpha", "first understanding"),
                None,
            )
            .with_artifacts(artifacts)
            .staging_rewrite(SentenceLabelSelection::empty(), "x");
        let original = draft.clone();
        let mut record = SessionRecord::understood(
            String::from("correction-race"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha")],
            Vec::new(),
        );
        record.phase = Phase::Published;
        record.drafts = vec![DraftRecord {
            term: String::from("alpha"),
            understanding: String::from("first understanding"),
            costs: ArtifactCosts::default(),
            meta_request: None,
            rewrite: None,
        }];
        record.result = Some(ResultRecord {
            deck: String::from("old.apkg"),
            report: String::from("old.pdf"),
            output: String::from("old-output"),
            cards: 1,
            failed: 0,
        });
        store.create(&record).expect("the session must persist");
        let session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        store
            .update("correction-race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing edit must persist");
        let calls = Arc::new(AtomicUsize::new(0));
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(vec![draft])
            .done_published("old.apkg", "old.pdf", "old-output");
        let mut shell = Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow: TestWorkflow::counting(calls.clone()),
            keys: TestWorkflow::local(),
            store: PreferenceStore::at(home.path().join("preferences.json")),
            session: Some(session),
            output: home.path().to_path_buf(),
        };
        let side = shell
            .handle(AppEvent::Generate)
            .expect("the correction request must stay in the TUI");
        for _ in 0..3 {
            shell.tick().expect("an idle conflicted shell must tick");
        }
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/correction-race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let journal = std::fs::read_dir(home.path().join("sessions/correction-race"))
            .expect("the session directory must list")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("costs-"));
        let lock_released = store
            .hold("correction-race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                side,
                calls.load(Ordering::SeqCst),
                (
                    shell.engine.is_none(),
                    shell.publish_job.is_none(),
                    shell.app.cards()[0] == original,
                    shell.app.cards()[0]
                        .staged_rewrite()
                        .map(crate::session::CardRewrite::started),
                ),
                (
                    shell.app.done_artifacts().deck.clone(),
                    shell.app.done_artifacts().report.clone(),
                    shell.app.error().map(String::from),
                ),
                (
                    stored.phase,
                    stored.senses,
                    stored.drafts[0].understanding.clone(),
                    stored
                        .result
                        .as_ref()
                        .map(|result| (result.deck.clone(), result.report.clone())),
                    journal,
                    lock_released,
                ),
            ),
            (
                Side::RegenerateCards,
                0,
                (true, true, true, Some(false)),
                (
                    String::from("old.apkg"),
                    String::from("old.pdf"),
                    Some(String::from(
                        "could not claim the session: session 'correction-race' changed outside this window; reopen it to continue",
                    )),
                ),
                (
                    Phase::Published,
                    String::from("all"),
                    String::from("first understanding"),
                    Some((String::from("old.apkg"), String::from("old.pdf"))),
                    false,
                    true,
                ),
            ),
            "a failed rewrite claim mutated staged presentation, publication, storage, or provider state"
        );
    }

    #[test]
    fn a_stale_tui_cannot_clear_published_paths_before_its_publish_claim_succeeds() {
        let home = tempfile::tempdir().expect("temp home");
        let store = SessionStore::new(home.path());
        let artifacts = CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded(),
            ArtifactSlot::fresh(Artifact::Scene).succeeded(),
            ArtifactSlot::fresh(Artifact::Picture).succeeded(),
            ArtifactSlot::fresh(Artifact::Sound).succeeded(),
        );
        let draft = CardDraft::new("alpha", "first understanding", pair())
            .with_meta(
                TestWorkflow::local_meta("alpha", "first understanding"),
                None,
            )
            .with_artifacts(artifacts);
        let original = draft.clone();
        let mut record = SessionRecord::understood(
            String::from("publish-race"),
            String::from("created-a"),
            String::from("ru"),
            String::from("en"),
            home.path().to_string_lossy().into_owned(),
            String::from("primary"),
            String::from("tui"),
            vec![String::from("alpha")],
            Vec::new(),
        );
        record.phase = Phase::Published;
        record.drafts = vec![DraftRecord {
            term: String::from("alpha"),
            understanding: String::from("first understanding"),
            costs: ArtifactCosts::default(),
            meta_request: None,
            rewrite: None,
        }];
        record.result = Some(ResultRecord {
            deck: String::from("old.apkg"),
            report: String::from("old.pdf"),
            output: String::from("old-output"),
            cards: 1,
            failed: 0,
        });
        store.create(&record).expect("the session must persist");
        let session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        store
            .update("publish-race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing edit must persist");
        let calls = Arc::new(AtomicUsize::new(0));
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .cards_started(vec![draft])
            .done_published("old.apkg", "old.pdf", "old-output");
        let mut shell = Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            regeneration_pending: false,
            started: None,
            quit_armed_at: None,
            new_batch_armed_at: None,
            destructive_escape_armed_at: None,
            workflow: TestWorkflow::counting(calls.clone()),
            keys: TestWorkflow::local(),
            store: PreferenceStore::at(home.path().join("preferences.json")),
            session: Some(session),
            output: home.path().to_path_buf(),
        };
        let side = shell
            .handle(AppEvent::Generate)
            .expect("the publish request must stay in the TUI");
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/publish-race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let journal = std::fs::read_dir(home.path().join("sessions/publish-race"))
            .expect("the session directory must list")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("costs-"));
        let lock_released = store
            .hold("publish-race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                side,
                calls.load(Ordering::SeqCst),
                (
                    shell.engine.is_none(),
                    shell.publish_job.is_none(),
                    shell.app.cards()[0] == original,
                ),
                (
                    shell.app.done_artifacts().deck.clone(),
                    shell.app.done_artifacts().report.clone(),
                    shell.app.error().map(String::from),
                ),
                (
                    stored.phase,
                    stored.senses,
                    stored
                        .result
                        .as_ref()
                        .map(|result| (result.deck.clone(), result.report.clone())),
                    journal,
                    lock_released,
                ),
            ),
            (
                Side::RegenerateCards,
                0,
                (true, true, true),
                (
                    String::from("old.apkg"),
                    String::from("old.pdf"),
                    Some(String::from(
                        "could not claim the session: session 'publish-race' changed outside this window; reopen it to continue",
                    )),
                ),
                (
                    Phase::Published,
                    String::from("all"),
                    Some((String::from("old.apkg"), String::from("old.pdf"))),
                    false,
                    true,
                ),
            ),
            "a failed publish claim cleared published paths or changed provider, storage, or lock state"
        );
    }

    #[test]
    fn submit_with_invalid_key_shows_inline_error_and_stays_on_welcome() {
        let mut shell = key_rejecting_shell(App::new(pair()).opening_welcome_at(
            WelcomeStage::EnterKey,
            KeySource::Pasted,
            "123456789012345678901234567890",
            false,
        ));
        shell
            .handle(AppEvent::Submit)
            .expect("submit must start the key check");
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                shell.app.screen(),
                shell.app.welcome().stage,
                shell.app.welcome().notice.clone(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::EnterKey,
                Some(String::from("key invalid")),
            ),
            "a rejected key must surface an inline notice and keep the user on the key step"
        );
    }

    #[test]
    fn load_env_key_action_reports_missing_or_loads_the_key() {
        let mut shell = shell(App::new(pair()).opening_welcome_at(
            WelcomeStage::EnterKey,
            KeySource::Empty,
            "",
            false,
        ));
        shell.apply_env_key(None);
        let missing = (
            shell.app.welcome().source,
            shell.app.welcome().key.to_string(),
            shell.app.welcome().notice.clone(),
        );
        shell.apply_env_key(Some(String::from("123456789012345678901234567890")));
        assert_eq!(
            (
                missing,
                shell.app.welcome().source,
                shell.app.welcome().key.to_string(),
                shell.app.welcome().notice.clone(),
            ),
            (
                (
                    KeySource::Empty,
                    String::new(),
                    Some(String::from("GEMINI_API_KEY is not set")),
                ),
                KeySource::Env,
                String::from("123456789012345678901234567890"),
                None,
            ),
            "load env must either show a missing-env notice or place GEMINI_API_KEY into the welcome key buffer"
        );
    }

    #[test]
    fn clearing_rejected_key_preserves_confirmed_language() {
        let home = tempfile::tempdir().expect("temp home");
        let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
        store
            .write(&Preferences::new("ru").with_api_key("123456789012345678901234567890"))
            .expect("seed preferences");
        clear_saved_key_in(&store).expect("clearing the saved key must succeed");
        let restored = store.read().expect("reload preferences");
        assert_eq!(
            (
                restored.my_language,
                restored.my_language_confirmed,
                restored.api_key,
            ),
            (String::from("ru"), true, None),
            "clearing a rejected key must not reset the confirmed support language"
        );
    }

    #[test]
    fn shell_generation_publishes_done_artifacts() {
        let mut shell = shell(review().understood(vec![candidate("whilst"), skipped("окно")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        settle_shell(&mut shell, 200);
        assert!(
            shell.app.screen() == Screen::YourCards
                && shell.app.done_artifacts().deck.ends_with(".apkg")
                && shell.app.done_artifacts().report.ends_with(".pdf")
                && !shell.app.done_artifacts().output.is_empty(),
            "generation must publish deck, report, and output path while staying on YourCards"
        );
    }

    #[test]
    fn publish_surfaces_the_building_deck_label_before_completing() {
        let mut shell = shell(review().understood(vec![candidate("whilst")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        let started = Instant::now();
        while shell.app.busy().is_none() && started.elapsed() < Duration::from_secs(5) {
            shell.tick().expect("tick must succeed");
            thread::sleep(Duration::from_millis(2));
        }
        let initial_kind = shell.app.busy().map(|busy| busy.kind());
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                initial_kind,
                shell.app.busy().is_none(),
                shell.app.done_artifacts().deck.is_empty(),
            ),
            (Some(BusyKind::PublishingDeck), true, false),
            "publish must put the building-deck loader up first, clear it once done, and populate done artifacts"
        );
    }

    #[test]
    fn ctrl_g_on_finished_cards_rebuilds_publish_outputs() {
        let mut shell = shell(review().understood(vec![candidate("whilst")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        settle_shell(&mut shell, 200);
        let side = shell
            .handle(AppEvent::Generate)
            .expect("Ctrl+G regenerate must start");
        let during = (
            side,
            shell.app.busy().map(|busy| busy.kind()),
            shell.app.done_artifacts().deck.is_empty(),
        );
        settle_shell(&mut shell, 200);
        assert_eq!(
            (
                during,
                shell.app.done_artifacts().deck.ends_with(".apkg"),
                shell.app.done_artifacts().report.ends_with(".pdf"),
            ),
            (
                (Side::RegenerateCards, Some(BusyKind::PublishingDeck), true),
                true,
                true,
            ),
            "Ctrl+G on finished cards must clear stale outputs and rebuild APKG/PDF"
        );
    }

    #[test]
    fn busy_kind_swapped_preserves_elapsed_time() {
        let app = App::new(pair())
            .busy_started(BusyKind::PublishingDeck)
            .busy_elapsed(Duration::from_millis(345))
            .busy_kind_swapped(BusyKind::PublishingReport);
        assert_eq!(
            app.busy().map(|busy| (busy.kind(), busy.elapsed())),
            Some((BusyKind::PublishingReport, Duration::from_millis(345))),
            "swapping kind mid-job must not reset the elapsed counter"
        );
    }

    #[test]
    fn stale_background_jobs_cannot_keep_fast_redraws() {
        let (_sender, receiver) = channel::<TextOutcome>();
        let mut fresh = shell(App::new(pair()));
        fresh.text = Some(PendingJob {
            receiver,
            handle: thread::spawn(|| {}),
            started: Instant::now(),
        });
        let (_sender, receiver) = channel::<TextOutcome>();
        let mut stale = shell(App::new(pair()));
        stale.text = Some(PendingJob {
            receiver,
            handle: thread::spawn(|| {}),
            started: Instant::now() - FAST_JOB_WINDOW - Duration::from_millis(1),
        });
        assert_eq!(
            (fresh.poll_timeout(), stale.poll_timeout()),
            (FAST_JOB_POLL, BACKGROUND_POLL),
            "background work must leave the fast cadence after the cache-hit window"
        );
    }

    #[test]
    fn elapsed_ticks_redraw_only_on_animation_frames() {
        let mut early = shell(App::new(pair()));
        early.started = Some(Instant::now() - Duration::from_millis(ANIMATION_FRAME_MILLIS / 2));
        let early_dirty = early.tick().expect("early tick must succeed");
        let mut framed = shell(App::new(pair()));
        framed.started = Some(Instant::now() - Duration::from_millis(ANIMATION_FRAME_MILLIS));
        let framed_dirty = framed.tick().expect("framed tick must succeed");
        assert_eq!(
            (early_dirty, framed_dirty, framed.app.elapsed()),
            (false, true, Duration::from_millis(ANIMATION_FRAME_MILLIS)),
            "elapsed updates must repaint only on animation-frame boundaries"
        );
    }

    #[test]
    fn idle_ticks_cannot_request_redraws() {
        let mut shell = shell(App::new(pair()));
        assert!(
            !shell.tick().expect("idle tick must succeed"),
            "idle ticks must not keep repainting the terminal"
        );
    }

    #[test]
    fn a_failed_publish_clears_the_engine_instead_of_relooping() {
        let mut shell = shell_with(
            review().understood(vec![candidate("whilst")]),
            TestWorkflow::publish_failing(),
        );
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        let mut settled = false;
        for _ in 0..300 {
            shell.tick().expect("tick must succeed");
            if shell.engine.is_none() && shell.publish_job.is_none() && shell.artifact_job.is_none()
            {
                settled = true;
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            settled && shell.engine.is_none(),
            "a failed publish must clear the engine so it stops instead of auto-relooping publish"
        );
    }

    #[test]
    fn shell_generation_skips_rejected_candidates() {
        let mut shell = shell(review().understood(vec![candidate("whilst"), skipped("окно")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        assert_eq!(
            shell
                .app
                .cards()
                .iter()
                .map(|draft| draft.term())
                .collect::<Vec<_>>(),
            vec!["whilst"],
            "generation must not create drafts for rejected candidates"
        );
    }
}
