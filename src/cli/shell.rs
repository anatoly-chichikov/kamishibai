//! Interactive CLI shell that coordinates app state and background jobs.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use super::card_workflow::{
    ArtifactOutcome, CardWorkflow, DeckPublishMessage, DeckPublishProgress, TextOutcome,
};
use super::live_generator::{LiveCardGenerator, default_output};
use crate::config::default_store;
use crate::gemini::GeminiClient;
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{Artifact, CardDraft, RawInputBatch, SessionEngine};
use crate::tui::{App, AppEvent, BusyKind, Screen, Side, transit};

const ANIMATION_FRAME_MILLIS: u64 = 250;
const IDLE_POLL: Duration = Duration::from_millis(ANIMATION_FRAME_MILLIS);
const BACKGROUND_POLL: Duration = Duration::from_millis(ANIMATION_FRAME_MILLIS);
const FAST_JOB_POLL: Duration = Duration::from_millis(25);
const FAST_JOB_WINDOW: Duration = Duration::from_millis(50);
const QUIT_WINDOW: Duration = Duration::from_millis(1000);

struct PendingTextJob {
    receiver: Receiver<TextOutcome>,
    handle: JoinHandle<()>,
    started: Instant,
}

struct PendingArtifactJob {
    receiver: Receiver<ArtifactOutcome>,
    handle: JoinHandle<()>,
    card: usize,
    artifact: Artifact,
    started: Instant,
}

struct PendingPublishJob {
    receiver: Receiver<DeckPublishMessage>,
    handle: JoinHandle<()>,
    started: Instant,
}

/// Stateful coordinator for TUI app transitions and background card workflow execution.
pub(super) struct Shell<P> {
    app: App,
    engine: Option<SessionEngine>,
    text: Option<PendingTextJob>,
    artifact_job: Option<PendingArtifactJob>,
    publish_job: Option<PendingPublishJob>,
    started: Option<Instant>,
    quit_armed_at: Option<Instant>,
    generator: P,
}

impl Shell<LiveCardGenerator> {
    /// Build a live card shell for an interactive empty session.
    pub(super) fn new(app: App) -> Result<Self> {
        let saved_key = default_store(&SystemContext)
            .ok()
            .and_then(|store| store.read().ok())
            .and_then(|prefs| prefs.api_key);
        let client = GeminiClient::from_env_or_saved(saved_key.as_deref())?;
        let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
        let output = default_output()?;
        crate::report::warm_fonts_async();
        Ok(Self {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            started: None,
            quit_armed_at: None,
            generator: LiveCardGenerator::new(client, cache, output),
        })
    }

    /// Build a live card shell that starts with generation already running.
    pub(super) fn startup(app: App, drafts: Vec<CardDraft>) -> Result<Self> {
        let mut shell = Self::new(app)?;
        shell.engine = Some(SessionEngine::start(drafts));
        shell.started = Some(Instant::now());
        Ok(shell)
    }
}

impl<P> Shell<P>
where
    P: CardWorkflow,
{
    /// Borrow the app model for rendering and pointer geometry.
    pub(super) fn app(&self) -> &App {
        &self.app
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
        self.text
            .as_ref()
            .map(|job| job.started.elapsed() <= FAST_JOB_WINDOW)
            .unwrap_or(false)
            || self
                .artifact_job
                .as_ref()
                .map(|job| job.started.elapsed() <= FAST_JOB_WINDOW)
                .unwrap_or(false)
            || self
                .publish_job
                .as_ref()
                .map(|job| job.started.elapsed() <= FAST_JOB_WINDOW)
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
            && now.duration_since(armed) <= QUIT_WINDOW
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
            && armed.elapsed() > QUIT_WINDOW
        {
            return self.disarm_quit();
        }
        false
    }

    /// Apply one app event and start any resulting side effect.
    pub(super) fn handle(&mut self, event: AppEvent) -> Result<Side> {
        if self.text.is_some() {
            return Ok(Side::None);
        }
        let (next, side) = transit(self.app.clone(), event);
        self.app = next;
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
        changed |= self.poll_publish()?;
        if self.publish_job.is_some() {
            return Ok(changed);
        }
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
        if let Some(event) = engine.batch_state() {
            match event {
                crate::session::EngineEvent::BatchReady => {
                    let side = self.handle(AppEvent::BatchReady)?;
                    if side == Side::ExitApp {
                        return Ok(true);
                    }
                    return Ok(true);
                }
                crate::session::EngineEvent::BatchDone { failed_cards } => {
                    let side = self.handle(AppEvent::BatchDone {
                        failed: failed_cards,
                    })?;
                    if side == Side::ExitApp {
                        return Ok(true);
                    }
                    return Ok(true);
                }
                _ => {}
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
        let pair = draft.pair().clone();
        let term = draft.term().to_string();
        let understanding = draft.understanding().to_string();
        let generator = self.generator.clone();
        let (sender, receiver) = channel();
        let handle = thread::spawn(move || {
            let outcome = match artifact {
                Artifact::Body => ArtifactOutcome::Body(
                    generator
                        .generate_card_body(&term, &understanding, &pair)
                        .map(|body| {
                            let file = generator
                                .store_card_text(&term, &understanding, &pair, &body)
                                .ok();
                            (body, file)
                        }),
                ),
                Artifact::Scene => ArtifactOutcome::Media(generator.generate_scene(&draft)),
                Artifact::Picture => ArtifactOutcome::Media(generator.generate_picture(&draft)),
                Artifact::Sound => ArtifactOutcome::Media(generator.generate_sound(&draft)),
            };
            let _ = sender.send(outcome);
        });
        self.artifact_job = Some(PendingArtifactJob {
            receiver,
            handle,
            card,
            artifact,
            started: Instant::now(),
        });
        self.app = self.app.clone().cards_running(Some((card, artifact)));
        Ok(())
    }

    fn poll_artifact(&mut self) -> Result<bool> {
        let Some(job) = self.artifact_job.as_ref() else {
            return Ok(false);
        };
        match job.receiver.try_recv() {
            Ok(outcome) => {
                let job = self
                    .artifact_job
                    .take()
                    .expect("invariant: artifact job must exist");
                let _ = join_thread(job.handle);
                self.apply_artifact_outcome(job.card, job.artifact, outcome);
                Ok(true)
            }
            Err(TryRecvError::Empty) => Ok(false),
            Err(TryRecvError::Disconnected) => {
                let job = self
                    .artifact_job
                    .take()
                    .expect("invariant: artifact job must exist");
                let _ = join_thread(job.handle);
                let synthetic = anyhow!("background artifact task disconnected");
                let outcome = match job.artifact {
                    Artifact::Body => ArtifactOutcome::Body(Err(synthetic)),
                    _ => ArtifactOutcome::Media(Err(synthetic)),
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
        let Some(engine) = self.engine.as_mut() else {
            self.app = self.app.clone().cards_running(None);
            return;
        };
        let _event = match outcome {
            ArtifactOutcome::Body(result) => engine.applied_body(card, result),
            ArtifactOutcome::Media(result) => engine.applied_media(card, artifact, result),
        };
        let drafts = engine.drafts().to_vec();
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
                        .confirmed_target(understood.guess().code())
                        .understood(understood.candidates().to_vec());
                }
                Err(error) => {
                    self.app = self
                        .app
                        .clone()
                        .with_screen(Screen::YourWords)
                        .error_shown(error.to_string());
                }
            },
            TextOutcome::BulkCorrection(result) => match result {
                Ok(updated) => {
                    let Some(refined) = updated.into_iter().next() else {
                        return;
                    };
                    let mut candidates = self.app.candidates().to_vec();
                    let selected = self.app.selected();
                    if selected < candidates.len() {
                        candidates[selected] = refined;
                        self.app = self.app.clone().understood(candidates);
                    }
                }
                Err(error) => {
                    self.app = self.app.clone().error_shown(error.to_string());
                }
            },
            TextOutcome::CardCorrection(result) => match result {
                Ok(payload) => {
                    let (revision, file) = *payload;
                    let (term, understanding, body) = revision.into_parts();
                    let Some(current) = self.app.cards().get(self.app.card_selected()).cloned()
                    else {
                        return;
                    };
                    let updated = current.recomposed(term, understanding, body, file);
                    self.app = self.app.clone().card_replaced(updated);
                    self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
                    self.started = Some(Instant::now());
                }
                Err(error) => {
                    self.app = self.app.clone().error_shown(error.to_string());
                }
            },
        }
    }

    fn apply(&mut self, side: Side) -> Result<()> {
        match side {
            Side::RunUnderstanding => {
                let raw = RawInputBatch::new(self.app.blob());
                let support = self.app.pair().support().to_string();
                let generator = self.generator.clone();
                self.start_text(BusyKind::Understanding, move || {
                    TextOutcome::Understanding(generator.understand(&raw, support.as_str()))
                })?;
            }
            Side::StartGeneration => {
                let drafts = drafts_from(&self.app);
                self.app = self.app.clone().cards_started(drafts);
                self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
                self.started = Some(Instant::now());
            }
            Side::RegenerateFailed => {
                self.app = self.app.clone().cards_reset_failures();
                self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
                self.started = Some(Instant::now());
            }
            Side::RegenerateCurrent => {
                self.regenerate_current()?;
            }
            Side::RunBulkCorrection(comment) => {
                let Some(focused) = self.app.candidates().get(self.app.selected()).cloned() else {
                    return Ok(());
                };
                let pair = self.app.pair().clone();
                let generator = self.generator.clone();
                self.start_text(BusyKind::BulkCorrection, move || {
                    TextOutcome::BulkCorrection(generator.correct_bulk(
                        std::slice::from_ref(&focused),
                        comment.as_str(),
                        &pair,
                    ))
                })?;
            }
            Side::RunCardCorrection(comment) => {
                if let Some(draft) = self.app.cards().get(self.app.card_selected()) {
                    let draft = draft.clone();
                    let pair = self.app.pair().clone();
                    let generator = self.generator.clone();
                    self.start_text(BusyKind::CardCorrection, move || {
                        TextOutcome::CardCorrection(
                            generator.correct_card(&draft, comment.as_str(), &pair).map(
                                |revision| {
                                    let file = generator
                                        .store_card_text(
                                            revision.term(),
                                            revision.understanding(),
                                            &pair,
                                            revision.body(),
                                        )
                                        .ok();
                                    Box::new((revision, file))
                                },
                            ),
                        )
                    })?;
                }
            }
            Side::StartPublish => {
                self.start_publish()?;
            }
            Side::PersistMyLanguage(code) => {
                if let Ok(store) = default_store(&SystemContext) {
                    let prefs = store.read().unwrap_or_default().adopt(code);
                    let _ = store.write(&prefs);
                }
            }
            Side::PersistWelcome { language, api_key } => {
                if let Ok(store) = default_store(&SystemContext) {
                    let mut prefs = store.read().unwrap_or_default().adopt(language);
                    if let Some(key) = api_key {
                        prefs = prefs.with_api_key(key);
                    }
                    let _ = store.write(&prefs);
                }
            }
            Side::OpenKeyHelp => {}
            Side::ExitApp | Side::None => {}
        }
        Ok(())
    }

    fn start_text<F>(&mut self, kind: BusyKind, run: F) -> Result<()>
    where
        F: FnOnce() -> TextOutcome + Send + 'static,
    {
        if self.text.is_some() {
            bail!("background text pass already running");
        }
        let (sender, receiver) = channel();
        let handle = thread::spawn(move || {
            let _ = sender.send(run());
        });
        self.text = Some(PendingTextJob {
            receiver,
            handle,
            started: Instant::now(),
        });
        self.app = self.app.clone().busy_started(kind);
        Ok(())
    }

    fn regenerate_current(&mut self) -> Result<()> {
        if self.artifact_job.is_some() || self.publish_job.is_some() || self.app.cards().is_empty()
        {
            return Ok(());
        }
        if self.app.cards_failed() > 0 {
            self.app = self.app.clone().cards_reset_failures();
            self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
            self.started = Some(Instant::now());
            return Ok(());
        }
        self.app = self.app.clone().publication_cleared();
        if self
            .app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready())
        {
            self.start_publish()?;
        } else {
            self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
            self.started = Some(Instant::now());
        }
        Ok(())
    }

    fn start_publish(&mut self) -> Result<()> {
        if self.publish_job.is_some() {
            bail!("background publish job already running");
        }
        let drafts = self.app.cards().to_vec();
        let generator = self.generator.clone();
        let (sender, receiver) = channel();
        let progress = DeckPublishProgress::new(sender.clone());
        let handle = thread::spawn(move || {
            let outcome = generator.publish_deck(&drafts, &progress);
            let _ = sender.send(DeckPublishMessage::Done(outcome));
        });
        self.publish_job = Some(PendingPublishJob {
            receiver,
            handle,
            started: Instant::now(),
        });
        self.app = self.app.clone().busy_started(BusyKind::PublishingDeck);
        Ok(())
    }

    fn poll_publish(&mut self) -> Result<bool> {
        let Some(started) = self.publish_job.as_ref().map(|job| job.started) else {
            return Ok(false);
        };
        let mut changed = self.refresh_busy_elapsed(started);
        let Some(job) = self.publish_job.as_ref() else {
            return Ok(changed);
        };
        loop {
            let message = match job.receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return Ok(changed),
                Err(TryRecvError::Disconnected) => {
                    let job = self
                        .publish_job
                        .take()
                        .expect("invariant: publish job must exist");
                    let message = join_thread(job.handle)
                        .map(|()| String::from("background publish job disconnected"))
                        .unwrap_or_else(|error| error.to_string());
                    self.app = self.app.clone().busy_finished().error_shown(message);
                    return Ok(true);
                }
            };
            match message {
                DeckPublishMessage::Phase(kind) => {
                    self.app = self.app.clone().busy_kind_swapped(kind);
                    changed = true;
                }
                DeckPublishMessage::Done(result) => {
                    let job = self
                        .publish_job
                        .take()
                        .expect("invariant: publish job must exist");
                    if let Err(error) = join_thread(job.handle) {
                        self.app = self
                            .app
                            .clone()
                            .busy_finished()
                            .error_shown(error.to_string());
                        return Ok(true);
                    }
                    self.app = self.app.clone().busy_finished();
                    match result {
                        Ok((deck, report, output)) => {
                            self.app = self.app.clone().done_published(deck, report, output);
                            self.engine = None;
                            self.started = None;
                        }
                        Err(error) => {
                            self.app = self.app.clone().error_shown(error.to_string());
                        }
                    }
                    return Ok(true);
                }
            }
        }
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
    app.candidates()
        .iter()
        .filter(|candidate| candidate.ok())
        .map(|candidate| {
            CardDraft::new(
                candidate.term(),
                candidate.understanding(),
                app.pair().clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::Result;
    use tempfile::tempdir;

    use super::super::card_workflow::{
        CardGeneration, DeckPublishProgress, DeckPublishing, TextOutcome,
    };
    use super::*;
    use crate::session::{
        ArtifactFile, BulkCorrection, CardBody, CardBodyGeneration, CardCorrection, CardRevision,
        LanguagePair, RawInputBatch, ScriptDetection, TargetDetection, Understanding, Understood,
        WordCandidate, catalog_for_detection,
    };

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct LocalCardGenerator;

    impl LocalCardGenerator {
        fn local_body(term: &str, understanding: &str) -> CardBody {
            CardBody::new(
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
    }

    impl Understanding for LocalCardGenerator {
        fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
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

    impl BulkCorrection for LocalCardGenerator {
        fn correct_bulk(
            &self,
            candidates: &[WordCandidate],
            comment: &str,
            _pair: &LanguagePair,
        ) -> Result<Vec<WordCandidate>> {
            Ok(candidates
                .iter()
                .map(|candidate| {
                    WordCandidate::new(
                        candidate.term(),
                        format!("{} · {}", candidate.understanding(), comment),
                        candidate.ok(),
                    )
                })
                .collect())
        }
    }

    impl CardBodyGeneration for LocalCardGenerator {
        fn generate_card_body(
            &self,
            term: &str,
            understanding: &str,
            _pair: &LanguagePair,
        ) -> Result<CardBody> {
            Ok(Self::local_body(term, understanding))
        }
    }

    impl CardCorrection for LocalCardGenerator {
        fn correct_card(
            &self,
            draft: &CardDraft,
            comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            let understanding = format!("{} · change: {comment}", draft.understanding());
            let body = Self::local_body(draft.term(), understanding.as_str());
            Ok(CardRevision::new(draft.term(), understanding, body))
        }
    }

    impl CardGeneration for LocalCardGenerator {
        fn generate_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Scene)
        }

        fn generate_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Picture)
        }

        fn generate_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Sound)
        }

        fn store_card_text(
            &self,
            term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
            _body: &CardBody,
        ) -> Result<ArtifactFile> {
            let name = format!("{}-body.local.json", slug(term));
            let path = std::env::temp_dir().join(&name);
            Ok(ArtifactFile::new(name, path, "1 B", false))
        }
    }

    impl DeckPublishing for LocalCardGenerator {
        fn publish_deck(
            &self,
            drafts: &[CardDraft],
            progress: &DeckPublishProgress,
        ) -> Result<(String, String, String)> {
            progress.report_phase(BusyKind::PublishingReport);
            Ok((
                format!("local-{}-cards.apkg", drafts.len()),
                format!("local-{}-cards.pdf", drafts.len()),
                String::from("/tmp/local-out"),
            ))
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

    fn shell(app: App) -> Shell<LocalCardGenerator> {
        Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            started: None,
            quit_armed_at: None,
            generator: LocalCardGenerator,
        }
    }

    fn failing_shell(app: App) -> Shell<FailingCardGenerator> {
        Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            publish_job: None,
            started: None,
            quit_armed_at: None,
            generator: FailingCardGenerator,
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
            .confirmed_target("en")
            .understood(vec![candidate("whilst")])
    }

    fn settle_text<P>(shell: &mut Shell<P>)
    where
        P: CardWorkflow,
    {
        for _ in 0..200 {
            shell.tick().expect("text pass tick must succeed");
            if shell.app.busy().is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("text pass did not settle before the deadline");
    }

    fn settle_engine<P>(shell: &mut Shell<P>, max_ticks: usize)
    where
        P: CardWorkflow,
    {
        for _ in 0..max_ticks {
            shell.tick().expect("engine tick must succeed");
            if shell.engine.is_none() && shell.artifact_job.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn settle_shell<P>(shell: &mut Shell<P>, max_ticks: usize)
    where
        P: CardWorkflow,
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

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct FailingCardGenerator;

    impl Understanding for FailingCardGenerator {
        fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl BulkCorrection for FailingCardGenerator {
        fn correct_bulk(
            &self,
            _candidates: &[WordCandidate],
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<Vec<WordCandidate>> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl CardBodyGeneration for FailingCardGenerator {
        fn generate_card_body(
            &self,
            _term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
        ) -> Result<CardBody> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl CardCorrection for FailingCardGenerator {
        fn correct_card(
            &self,
            _draft: &CardDraft,
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl CardGeneration for FailingCardGenerator {
        fn generate_scene(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }

        fn generate_picture(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }

        fn generate_sound(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }

        fn store_card_text(
            &self,
            _term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
            _body: &CardBody,
        ) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl DeckPublishing for FailingCardGenerator {
        fn publish_deck(
            &self,
            _drafts: &[CardDraft],
            _progress: &DeckPublishProgress,
        ) -> Result<(String, String, String)> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    #[test]
    fn first_pass_keeps_commas_inside_lines() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(App::new(pair()).seeded_blob("whilst, in the end\nwreck"));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must run understanding");
        settle_text(&mut shell);
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
    fn text_pass_failure_stays_in_the_tui_as_recoverable_error() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = failing_shell(App::new(pair()).seeded_blob("wreck"));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start understanding");
        settle_text(&mut shell);
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
    fn shell_generation_publishes_done_artifacts() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(review().understood(vec![candidate("whilst"), skipped("окно")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        settle_engine(&mut shell, 200);
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
        let _output = tempdir().expect("temp output must exist");
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
        settle_engine(&mut shell, 200);
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
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(review().understood(vec![candidate("whilst")]));
        shell
            .handle(AppEvent::Generate)
            .expect("generate must start generation");
        settle_engine(&mut shell, 200);
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
                (
                    Side::RegenerateCurrent,
                    Some(BusyKind::PublishingDeck),
                    true
                ),
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
        fresh.text = Some(PendingTextJob {
            receiver,
            handle: thread::spawn(|| {}),
            started: Instant::now(),
        });
        let (_sender, receiver) = channel::<TextOutcome>();
        let mut stale = shell(App::new(pair()));
        stale.text = Some(PendingTextJob {
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
    fn shell_generation_skips_rejected_candidates() {
        let _output = tempdir().expect("temp output must exist");
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
