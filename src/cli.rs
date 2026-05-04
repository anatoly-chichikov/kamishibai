//! TUI entrypoint for the word-first kamishibai flow.
//!
//! The TUI shell owns the terminal, preferences, and the background workers
//! for both text passes (understand / bulk / per-card correction / card body)
//! and media passes (scene / picture / audio). Every Gemini call runs in a
//! background thread so the TUI never blocks on network I/O. Once a batch
//! finishes, `PublishDone` saves a real APKG deck and a real PDF report.

use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags, poll, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::config::{Preferences, default_store};
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::artifact_cache::Cache;
use crate::generation::manga::{
    BorderDetector, Illustration, MangaRenderer, Progress as SceneProgress, TextDetector,
};
use crate::generation::speech::Audio;
use crate::generation::{SceneComposer, render_audio_prompt};
use crate::languages::{LanguageCatalog, ReportLabels, catalog, naming};
use crate::report::{Report, ReportFonts, Thumbnail, VocabularyLayout};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactFile, BulkCorrection, CardBody, CardBodyGeneration, CardCorrection,
    CardDraft, CardRevision, EngineEvent, LanguagePair, RawInputBatch, SessionEngine,
    Understanding, Understood, WordCandidate, to_entry,
};
use crate::tui::{
    App, AppEvent, BusyKind, ModalKind, Screen, Side, draw, language_chip_at, link_at,
    picker_geometry, to_app, transit,
};
use crate::vocabulary::VocabularyEntry;

#[cfg(test)]
use crate::session::{ScriptDetection, TargetDetection, catalog_for_detection};

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

/// Execute the TUI and translate failures into a process exit code.
pub fn run() -> u8 {
    match start() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn start() -> Result<()> {
    let preferences = load_preferences().unwrap_or_default();
    let pair = LanguagePair::new(String::from("en"), preferences.my_language.clone());
    let app = App::new(pair);
    enable_raw_mode()?;
    let mut out = stdout();
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    if enhanced {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
    }
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let outcome = loop_forever(&mut terminal, app);
    if enhanced {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags).ok();
    }
    execute!(terminal.backend_mut(), DisableMouseCapture).ok();
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    outcome
}

fn loop_forever<B>(terminal: &mut Terminal<B>, app: App) -> Result<()>
where
    B: ratatui::backend::Backend,
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    let mut shell = Shell::new(app)?;
    loop {
        shell.refresh_quit_pending();
        terminal.draw(|frame| draw(frame, shell.app()))?;
        if !poll(Duration::from_millis(100))? {
            shell.tick()?;
            continue;
        }
        let event = read()?;
        match event {
            Event::Key(key) => {
                let Some(event) = to_app(key) else { continue };
                if matches!(event, AppEvent::Quit) {
                    if shell.arm_quit() {
                        return Ok(());
                    }
                    continue;
                }
                shell.disarm_quit();
                let side = shell.handle(event)?;
                if side == Side::ExitApp {
                    return Ok(());
                }
                shell.tick()?;
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    shell.scroll(-1);
                }
                MouseEventKind::ScrollDown => {
                    shell.scroll(1);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    let area = terminal.size()?;
                    let rect = ratatui::layout::Rect {
                        x: 0,
                        y: 0,
                        width: area.width,
                        height: area.height,
                    };
                    if shell.app().modal() == Some(ModalKind::PickMyLanguage) {
                        if let Some(index) = picker_geometry::chip_at(rect, mouse.column, mouse.row)
                        {
                            let codes = catalog().codes();
                            if let Some(code) = codes.get(index) {
                                let event = AppEvent::SetMyLanguage(String::from(*code));
                                let side = shell.handle(event)?;
                                if side == Side::ExitApp {
                                    return Ok(());
                                }
                                shell.tick()?;
                            }
                        }
                    } else if language_chip_at(shell.app(), rect, mouse.column, mouse.row) {
                        let side = shell.handle(AppEvent::OpenLanguagePicker)?;
                        if side == Side::ExitApp {
                            return Ok(());
                        }
                        shell.tick()?;
                    } else if let Some(target) = link_at(shell.app(), rect, mouse.column, mouse.row)
                    {
                        let _ = open_path(target.as_str());
                    }
                }
                _ => {}
            },
            _ => {
                shell.tick()?;
            }
        }
    }
}

/// All text-oriented Gemini passes the shell delegates to.
trait TextPasses:
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

/// All media-oriented Gemini passes plus deck and report finalization.
trait MediaPasses: Clone + Send + 'static {
    fn produce_scene(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn produce_picture(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    fn produce_sound(&self, draft: &CardDraft) -> Result<ArtifactFile>;
    /// Persist a generated card body to the local cache so the user can open
    /// the rich JSON from the step list (clickable just like the media files).
    fn persist_body(
        &self,
        term: &str,
        pair: &LanguagePair,
        body: &CardBody,
    ) -> Result<ArtifactFile>;
    fn publish(&self, drafts: &[CardDraft]) -> Result<(String, String, String)>;
}

trait Lifecycle: TextPasses + MediaPasses {}
impl<T> Lifecycle for T where T: TextPasses + MediaPasses {}

enum TextOutcome {
    Understanding(Result<Understood>),
    BulkCorrection(Result<Vec<WordCandidate>>),
    CardCorrection(Result<Box<CardRevision>>),
}

struct PendingTextJob {
    receiver: Receiver<TextOutcome>,
    handle: JoinHandle<()>,
    started: Instant,
}

enum ArtifactOutcome {
    Body(Result<(CardBody, Option<ArtifactFile>)>),
    Media(Result<ArtifactFile>),
}

struct PendingArtifactJob {
    receiver: Receiver<ArtifactOutcome>,
    handle: JoinHandle<()>,
    card: usize,
    artifact: Artifact,
    #[allow(dead_code)]
    started: Instant,
}

const QUIT_WINDOW: Duration = Duration::from_millis(1000);

struct Shell<P> {
    app: App,
    engine: Option<SessionEngine>,
    text: Option<PendingTextJob>,
    artifact_job: Option<PendingArtifactJob>,
    started: Option<Instant>,
    quit_armed_at: Option<Instant>,
    passes: P,
}

impl Shell<ProductionPasses> {
    fn new(app: App) -> Result<Self> {
        let client = GeminiClient::from_env()?;
        let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
        let output = default_output()?;
        Ok(Self {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            started: None,
            quit_armed_at: None,
            passes: ProductionPasses::new(client, cache, output),
        })
    }
}

impl<P> Shell<P>
where
    P: Lifecycle,
{
    fn app(&self) -> &App {
        &self.app
    }

    fn scroll(&mut self, delta: i32) {
        self.app = self.app.clone().body_scrolled(delta);
    }

    fn arm_quit(&mut self) -> bool {
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

    fn disarm_quit(&mut self) {
        self.quit_armed_at = None;
        if self.app.quit_pending() {
            self.app = self.app.clone().with_quit_pending(false);
        }
    }

    fn refresh_quit_pending(&mut self) {
        if let Some(armed) = self.quit_armed_at
            && armed.elapsed() > QUIT_WINDOW
        {
            self.disarm_quit();
        }
    }

    fn handle(&mut self, event: AppEvent) -> Result<Side> {
        if self.text.is_some() {
            return Ok(Side::None);
        }
        let sync_engine = matches!(event, AppEvent::KeyChar('d') | AppEvent::KeyChar('D'));
        let (next, side) = transit(self.app.clone(), event);
        self.app = next;
        self.apply(side.clone())?;
        if sync_engine && self.engine.is_some() {
            self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
        }
        Ok(side)
    }

    fn tick(&mut self) -> Result<()> {
        if let Some(started) = self.started {
            self.app = self.app.clone().with_elapsed(started.elapsed());
        }
        self.poll_text()?;
        if self.text.is_some() {
            return Ok(());
        }
        self.poll_artifact()?;
        if self.artifact_job.is_some() {
            return Ok(());
        }
        self.advance_engine()
    }

    fn advance_engine(&mut self) -> Result<()> {
        let Some(engine) = self.engine.as_ref() else {
            return Ok(());
        };
        if let Some((card, kind)) = engine.next_target() {
            self.spawn_artifact(card, kind)?;
            return Ok(());
        }
        if let Some(event) = engine.batch_state() {
            match event {
                EngineEvent::BatchReady => {
                    let side = self.handle(AppEvent::BatchReady)?;
                    if side == Side::ExitApp {
                        return Ok(());
                    }
                }
                EngineEvent::BatchDone { failed_cards } => {
                    let side = self.handle(AppEvent::BatchDone {
                        failed: failed_cards,
                    })?;
                    if side == Side::ExitApp {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Ok(())
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
        let passes = self.passes.clone();
        let (sender, receiver) = channel();
        let handle = thread::spawn(move || {
            let outcome = match artifact {
                Artifact::Body => ArtifactOutcome::Body(
                    passes
                        .generate_card_body(&term, &understanding, &pair)
                        .map(|body| {
                            let file = passes.persist_body(&term, &pair, &body).ok();
                            (body, file)
                        }),
                ),
                Artifact::Scene => ArtifactOutcome::Media(passes.produce_scene(&draft)),
                Artifact::Picture => ArtifactOutcome::Media(passes.produce_picture(&draft)),
                Artifact::Sound => ArtifactOutcome::Media(passes.produce_sound(&draft)),
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

    fn poll_artifact(&mut self) -> Result<()> {
        let Some(job) = self.artifact_job.as_ref() else {
            return Ok(());
        };
        match job.receiver.try_recv() {
            Ok(outcome) => {
                let job = self
                    .artifact_job
                    .take()
                    .expect("invariant: artifact job must exist");
                let _ = join_thread(job.handle);
                self.apply_artifact_outcome(job.card, job.artifact, outcome);
            }
            Err(TryRecvError::Empty) => {}
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
            }
        }
        Ok(())
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

    fn poll_text(&mut self) -> Result<()> {
        let Some(job) = self.text.as_ref() else {
            return Ok(());
        };
        self.app = self.app.clone().busy_elapsed(job.started.elapsed());
        match job.receiver.try_recv() {
            Ok(outcome) => {
                let job = self.text.take().expect("invariant: text job must exist");
                self.app = self.app.clone().busy_finished();
                if let Err(error) = join_thread(job.handle) {
                    self.app = self.app.clone().error_shown(error.to_string());
                    return Ok(());
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
            }
        }
        Ok(())
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
                    self.app = self.app.clone().understood(updated);
                }
                Err(error) => {
                    self.app = self.app.clone().error_shown(error.to_string());
                }
            },
            TextOutcome::CardCorrection(result) => match result {
                Ok(revision) => {
                    let (term, understanding, body) = revision.into_parts();
                    let Some(current) = self.app.cards().get(self.app.card_selected()).cloned()
                    else {
                        return;
                    };
                    let updated = current.recomposed(term, understanding, body);
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
                let passes = self.passes.clone();
                self.start_text(BusyKind::Understanding, move || {
                    TextOutcome::Understanding(passes.understand(&raw, support.as_str()))
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
            Side::RunBulkCorrection(comment) => {
                let candidates = self.app.candidates().to_vec();
                let pair = self.app.pair().clone();
                let passes = self.passes.clone();
                self.start_text(BusyKind::BulkCorrection, move || {
                    TextOutcome::BulkCorrection(passes.correct_bulk(
                        candidates.as_slice(),
                        comment.as_str(),
                        &pair,
                    ))
                })?;
            }
            Side::RunCardCorrection(comment) => {
                if let Some(draft) = self.app.cards().get(self.app.card_selected()) {
                    let draft = draft.clone();
                    let pair = self.app.pair().clone();
                    let passes = self.passes.clone();
                    self.start_text(BusyKind::CardCorrection, move || {
                        TextOutcome::CardCorrection(
                            passes
                                .correct_card(&draft, comment.as_str(), &pair)
                                .map(Box::new),
                        )
                    })?;
                }
            }
            Side::PublishDone => {
                let drafts = self.app.cards().to_vec();
                let (deck, report, output) = self.passes.publish(&drafts)?;
                self.app = self.app.clone().done_published(deck, report, output);
                self.engine = None;
                self.started = None;
            }
            Side::PersistMyLanguage(code) => {
                if let Ok(store) = default_store(&SystemContext) {
                    let _ = store.write(&Preferences::new(code));
                }
            }
            Side::PersistApiKey(_) => {}
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
}

fn join_thread(handle: JoinHandle<()>) -> Result<()> {
    if handle.join().is_err() {
        bail!("background task panicked");
    }
    Ok(())
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

/// Production media + lifecycle passes backed by Gemini and the on-disk cache.
#[derive(Clone)]
pub struct ProductionPasses {
    client: GeminiClient<HttpTransport>,
    cache: PathBuf,
    output: PathBuf,
    catalog: LanguageCatalog,
}

impl ProductionPasses {
    /// Build production passes from a live Gemini client and on-disk locations.
    pub fn new(client: GeminiClient<HttpTransport>, cache: PathBuf, output: PathBuf) -> Self {
        Self {
            client,
            cache,
            output,
            catalog: catalog(),
        }
    }

    fn audio_for(&self, target_lang: &str) -> Result<Audio<GeminiClient<HttpTransport>>> {
        let item = self.catalog.item(target_lang)?;
        Ok(Audio::new(
            Cache::new(item.audio_cache.as_str(), self.cache.clone()),
            render_audio_prompt(item.prompt.as_str()),
            self.client.clone(),
        ))
    }

    fn illustration_for(
        &self,
        target_lang: &str,
    ) -> Result<Illustration<SceneComposer<GeminiClient<HttpTransport>>, MangaRenderer<TextDetector>>>
    {
        let item = self.catalog.item(target_lang)?;
        let client = self.client.clone();
        Ok(Illustration::new(
            Cache::new(item.image_cache.as_str(), self.cache.clone()),
            SceneComposer::new(client.clone(), item.prompt.as_str()),
            MangaRenderer::new(
                client,
                3,
                TextDetector::cached(60, item.ocr.as_str(), self.cache.clone()),
                BorderDetector::new(6, 240, 10),
            ),
        ))
    }
}

impl Understanding for ProductionPasses {
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        self.client.understand(raw, my)
    }
}

impl BulkCorrection for ProductionPasses {
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        self.client.correct_bulk(candidates, comment, pair)
    }
}

impl CardBodyGeneration for ProductionPasses {
    fn generate_card_body(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardBody> {
        self.client.generate_card_body(term, understanding, pair)
    }
}

impl CardCorrection for ProductionPasses {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.client.correct_card(draft, comment, pair)
    }
}

impl MediaPasses for ProductionPasses {
    fn produce_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before scene"))?;
        let illustration = self.illustration_for(draft.pair().target())?;
        let mut progress = NoopProgress;
        let (filename, cached) = illustration.scene_only(
            body.target_sentence(),
            draft.pair().target(),
            &mut progress,
        )?;
        let path = illustration.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn produce_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before picture"))?;
        let illustration = self.illustration_for(draft.pair().target())?;
        let mut progress = NoopProgress;
        let (filename, cached) = illustration.picture_only(
            body.target_sentence(),
            draft.pair().target(),
            &mut progress,
        )?;
        let path = illustration.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn produce_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before sound"))?;
        let audio = self.audio_for(draft.pair().target())?;
        let (filename, cached) = audio.generate(body.target_sentence())?;
        let path = audio.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn persist_body(
        &self,
        term: &str,
        pair: &LanguagePair,
        body: &CardBody,
    ) -> Result<ArtifactFile> {
        let item = self.catalog.item(pair.target())?;
        let cache = Cache::new(format!("body-{}", item.code), self.cache.clone());
        let digest_full = format!("{:x}", md5::compute(format!("{}\0{}", pair.target(), term)));
        let filename = format!("{}.json", &digest_full[..12]);
        let path = cache.filepath(filename.as_str())?;
        let cached = cache.exists(filename.as_str());
        if !cached {
            let payload = serde_json::json!({
                "term": term,
                "target_lang": pair.target(),
                "source_lang": pair.support(),
                "pronunciation": body.pronunciation(),
                "transcription": body.transcription(),
                "meaning": body.meaning(),
                "importance": body.importance(),
                "source_sentence": body.source_sentence(),
                "source_highlight": body.source_highlight(),
                "source_hint": body.source_hint(),
                "source_context": body.source_context(),
                "target_sentence": body.target_sentence(),
            });
            let staged = cache.stage(".json")?;
            let result = fs::write(&staged, serde_json::to_string_pretty(&payload)?)
                .map_err(anyhow::Error::from)
                .and_then(|()| cache.commit(&staged, filename.as_str()));
            if result.is_err() {
                let _ = fs::remove_file(&staged);
            }
            result?;
        }
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn publish(&self, drafts: &[CardDraft]) -> Result<(String, String, String)> {
        fs::create_dir_all(&self.output)?;
        let entries: Vec<VocabularyEntry> = drafts
            .iter()
            .filter(|draft| draft.artifacts().all_ready())
            .map(to_entry)
            .collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            bail!("no completed cards to publish");
        }
        let decknaming = naming(None, entries.as_slice());
        let model = CardModel::new().model();
        let mut container = VocabularyDeck::new(
            StableId::new(decknaming.name.as_str()).value(),
            decknaming.name.as_str(),
            VocabularyNote::new(model),
            Vec::<PathBuf>::new(),
        );
        let mut report = Report::new(
            VocabularyLayout::new(ReportLabels::default()),
            ReportFonts::default(),
        );
        for draft in drafts.iter().filter(|draft| draft.artifacts().all_ready()) {
            let entry = to_entry(draft)?;
            let audio_file = draft
                .artifacts()
                .sound()
                .file()
                .ok_or_else(|| anyhow!("sound artifact missing for {}", draft.term()))?;
            let picture_file = draft
                .artifacts()
                .picture()
                .file()
                .ok_or_else(|| anyhow!("picture artifact missing for {}", draft.term()))?;
            let audio_path = self
                .audio_for(draft.pair().target())?
                .filepath(audio_file.name())?;
            let picture_path = self
                .illustration_for(draft.pair().target())?
                .filepath(picture_file.name())?;
            container.attach(audio_path);
            container.attach(picture_path.clone());
            container.add(
                &entry,
                format!("[sound:{}]", audio_file.name()).as_str(),
                format!("<img src='{}' style='{IMAGE_STYLE}'>", picture_file.name()).as_str(),
            );
            report.append(&entry, Some(picture_path));
        }
        let stamp = release_stamp()?;
        let apkg = self
            .output
            .join(format!("{}_{}.apkg", decknaming.prefix, stamp));
        container.save(&apkg)?;
        let pdf = self
            .output
            .join(format!("{}_{}.pdf", decknaming.prefix, stamp));
        report.save(&pdf, &Thumbnail::new(150))?;
        Ok((
            apkg.to_string_lossy().into_owned(),
            pdf.to_string_lossy().into_owned(),
            self.output.to_string_lossy().into_owned(),
        ))
    }
}

struct NoopProgress;

impl SceneProgress for NoopProgress {
    fn step(&mut self, _name: &str) {}
    fn done(&mut self, _name: &str, _label: &str, _path: Option<&Path>) {}
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn release_stamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc()
        .format(parse_time("[year]-[month]-[day]_[hour][minute][second]")?.as_slice())?)
}

fn default_output() -> Result<PathBuf> {
    Locations::new(LocationArgs::default(), SystemContext).output()
}

/// Open one filesystem path with the host system's default handler. macOS
/// uses `open`, Linux uses `xdg-open`, Windows uses `cmd /c start`.
fn open_path(path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", path])
            .spawn()?;
    }
    Ok(())
}

fn load_preferences() -> Result<Preferences> {
    let store = default_store(&SystemContext)?;
    store.read()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::tui::Screen;

    /// Test-only passes: produces deterministic fakes without touching the network.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(super) struct LocalPasses;

    impl LocalPasses {
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

    impl Understanding for LocalPasses {
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

    impl BulkCorrection for LocalPasses {
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

    impl CardBodyGeneration for LocalPasses {
        fn generate_card_body(
            &self,
            term: &str,
            understanding: &str,
            _pair: &LanguagePair,
        ) -> Result<CardBody> {
            Ok(Self::local_body(term, understanding))
        }
    }

    impl CardCorrection for LocalPasses {
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

    impl MediaPasses for LocalPasses {
        fn produce_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Scene)
        }

        fn produce_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Picture)
        }

        fn produce_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            local_artifact(draft, Artifact::Sound)
        }

        fn persist_body(
            &self,
            term: &str,
            _pair: &LanguagePair,
            _body: &CardBody,
        ) -> Result<ArtifactFile> {
            let name = format!("{}-body.local.json", slug(term));
            let path = std::env::temp_dir().join(&name);
            Ok(ArtifactFile::new(name, path, "1 B", false))
        }

        fn publish(&self, drafts: &[CardDraft]) -> Result<(String, String, String)> {
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

    fn shell(app: App) -> Shell<LocalPasses> {
        Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            started: None,
            quit_armed_at: None,
            passes: LocalPasses,
        }
    }

    fn failing_shell(app: App) -> Shell<FailingPasses> {
        Shell {
            app,
            engine: None,
            text: None,
            artifact_job: None,
            started: None,
            quit_armed_at: None,
            passes: FailingPasses,
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
        P: Lifecycle,
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
        P: Lifecycle,
    {
        for _ in 0..max_ticks {
            shell.tick().expect("engine tick must succeed");
            if shell.engine.is_none() && shell.artifact_job.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct FailingPasses;

    impl Understanding for FailingPasses {
        fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl BulkCorrection for FailingPasses {
        fn correct_bulk(
            &self,
            _candidates: &[WordCandidate],
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<Vec<WordCandidate>> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl CardBodyGeneration for FailingPasses {
        fn generate_card_body(
            &self,
            _term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
        ) -> Result<CardBody> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl CardCorrection for FailingPasses {
        fn correct_card(
            &self,
            _draft: &CardDraft,
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    impl MediaPasses for FailingPasses {
        fn produce_scene(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
        fn produce_picture(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
        fn produce_sound(&self, _draft: &CardDraft) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
        fn persist_body(
            &self,
            _term: &str,
            _pair: &LanguagePair,
            _body: &CardBody,
        ) -> Result<ArtifactFile> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
        fn publish(&self, _drafts: &[CardDraft]) -> Result<(String, String, String)> {
            Err(anyhow::anyhow!("INTERNAL: boom"))
        }
    }

    #[test]
    fn first_pass_keeps_commas_inside_lines() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(App::new(pair()).seeded_blob("whilst, in the end\nwreck"));
        shell
            .handle(AppEvent::Submit)
            .expect("submit must run understanding");
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
            .handle(AppEvent::Submit)
            .expect("submit must start understanding");
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
            .handle(AppEvent::KeyEnter)
            .expect("enter must start generation");
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
    fn shell_generation_skips_rejected_candidates() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(review().understood(vec![candidate("whilst"), skipped("окно")]));
        shell
            .handle(AppEvent::Submit)
            .expect("submit must start generation");
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

    #[test]
    fn shell_drop_artifact_restarts_the_engine_from_current_cards() {
        let _output = tempdir().expect("temp output must exist");
        let mut shell = shell(review());
        shell
            .handle(AppEvent::Submit)
            .expect("submit must start generation");
        for _ in 0..30 {
            shell.tick().expect("tick");
            let body_ready = shell
                .app
                .cards()
                .first()
                .map(|draft| draft.artifacts().body().ready())
                .unwrap_or(false);
            if body_ready {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        shell
            .handle(AppEvent::KeyChar('d'))
            .expect("drop must sync generation queue");
        let discarded = shell
            .engine
            .as_ref()
            .expect("engine must still exist")
            .drafts()[0]
            .artifacts()
            .scene()
            .discarded();
        assert!(
            discarded,
            "drop artifact must restart the engine with the discarded slot"
        );
    }
}
