//! TUI entrypoint for the word-first kamishibai flow.
//!
//! The old JSON-first CLI is gone. The binary boots straight into the locked
//! TUI state machine. This module owns the terminal shell, preference store,
//! first-pass understanding, and a deterministic artifact side-effect bridge.

use std::fs;
use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{
    Event, KeyCode, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags, poll, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::{Preferences, default_store};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactFile, ArtifactProducer, BulkCorrection, CandidateKind, CardCorrection,
    CardDraft, CardPayload, EngineEvent, LanguagePair, RawInputBatch, ScriptDetection,
    SessionEngine, TargetDetection, Understanding, Understood, WordCandidate,
    catalog_for_detection,
};
use crate::tui::{App, AppEvent, Side, draw, to_app, transit};

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
    execute!(out, EnterAlternateScreen)?;
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
        terminal.draw(|frame| draw(frame, shell.app()))?;
        if !poll(Duration::from_millis(200))? {
            shell.tick()?;
            continue;
        }
        let Event::Key(key) = read()? else {
            continue;
        };
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        let Some(event) = to_app(key) else { continue };
        let side = shell.handle(event)?;
        if side == Side::ExitApp {
            return Ok(());
        }
    }
}

struct Shell {
    app: App,
    engine: Option<SessionEngine>,
    producer: LocalArtifactProducer,
    started: Option<Instant>,
}

impl Shell {
    fn new(app: App) -> Result<Self> {
        Ok(Self {
            app,
            engine: None,
            producer: LocalArtifactProducer::new(default_output()?),
            started: None,
        })
    }

    fn app(&self) -> &App {
        &self.app
    }

    fn handle(&mut self, event: AppEvent) -> Result<Side> {
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
        let Some(event) = self.advance() else {
            return Ok(());
        };
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
            EngineEvent::ArtifactReady { .. }
            | EngineEvent::RetryStarted { .. }
            | EngineEvent::RetryExhausted { .. } => {}
        }
        Ok(())
    }

    fn advance(&mut self) -> Option<EngineEvent> {
        let engine = self.engine.as_mut()?;
        let event = engine.advance(&mut self.producer)?;
        self.app = self.app.clone().cards_replaced(engine.drafts().to_vec());
        Some(event)
    }

    fn apply(&mut self, side: Side) -> Result<()> {
        match side {
            Side::RunUnderstanding => {
                let understood = LocalUnderstanding.understand(
                    &RawInputBatch::new(self.app.blob()),
                    self.app.pair().support(),
                )?;
                self.app = self
                    .app
                    .clone()
                    .confirmed_target(understood.guess().code())
                    .understood(understood.candidates().to_vec());
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
                let updated = LocalCorrection.correct_bulk(
                    self.app.candidates(),
                    &comment,
                    self.app.pair(),
                )?;
                self.app = self.app.clone().understood(updated);
            }
            Side::RunCardCorrection(comment) => {
                if let Some(draft) = self.app.cards().get(self.app.card_selected()) {
                    let updated = LocalCorrection.correct_card(draft, &comment, self.app.pair())?;
                    self.app = self.app.clone().card_replaced(updated);
                    self.engine = Some(SessionEngine::start(self.app.cards().to_vec()));
                    self.started = Some(Instant::now());
                }
            }
            Side::PublishDone => {
                let (deck, report, output) = self.producer.publish(self.app.cards())?;
                self.app = self.app.clone().done_published(deck, report, output);
                self.engine = None;
                self.started = None;
            }
            Side::PersistMyLanguage(code) => {
                if let Ok(store) = default_store(&SystemContext) {
                    let _ = store.write(&Preferences::new(code));
                }
            }
            Side::ExitApp | Side::None => {}
        }
        Ok(())
    }
}

fn drafts_from(app: &App) -> Vec<CardDraft> {
    app.candidates()
        .iter()
        .map(|candidate| {
            CardDraft::new(
                candidate.term(),
                app.pair().clone(),
                CardPayload::new(
                    candidate.term(),
                    nonempty(candidate.preview(), candidate.term()),
                    nonempty(candidate.note(), "line-delimited input"),
                    candidate.term(),
                ),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LocalUnderstanding;

impl Understanding for LocalUnderstanding {
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
                    kind_for(entry),
                    format!("{entry} · review for {my}"),
                    String::from("one item per line"),
                )
            })
            .collect();
        Ok(Understood::new(guess, candidates))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LocalCorrection;

impl BulkCorrection for LocalCorrection {
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
                    candidate.kind().clone(),
                    format!(
                        "{} · {}",
                        nonempty(candidate.preview(), candidate.term()),
                        comment
                    ),
                    nonempty(candidate.note(), comment),
                )
            })
            .collect())
    }
}

impl CardCorrection for LocalCorrection {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        _pair: &LanguagePair,
    ) -> Result<CardDraft> {
        Ok(draft.clone().recomposed(CardPayload::new(
            draft.payload().front(),
            format!("{}\nchange: {comment}", draft.payload().back()),
            nonempty(draft.payload().hint(), comment),
            draft.payload().highlight(),
        )))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalArtifactProducer {
    output: PathBuf,
    stamp: String,
}

impl LocalArtifactProducer {
    fn new(output: PathBuf) -> Self {
        Self {
            output,
            stamp: stamp(),
        }
    }

    fn publish(&self, drafts: &[CardDraft]) -> Result<(String, String, String)> {
        fs::create_dir_all(&self.output)?;
        let deck = self.output.join(format!("kamishibai-{}.apkg", self.stamp));
        let report = self.output.join(format!("kamishibai-{}.pdf", self.stamp));
        fs::write(&deck, deck_text(drafts))?;
        fs::write(&report, report_text(drafts))?;
        Ok((
            file_name(&deck),
            file_name(&report),
            self.output.to_string_lossy().into_owned(),
        ))
    }
}

impl ArtifactProducer for LocalArtifactProducer {
    fn produce(&mut self, draft: &CardDraft, artifact: Artifact) -> Result<ArtifactFile> {
        fs::create_dir_all(&self.output)?;
        let filename = format!(
            "{}-{}-{}.{}",
            self.stamp,
            slug(draft.term()),
            artifact.label(),
            extension(artifact)
        );
        let path = self.output.join(&filename);
        fs::write(
            &path,
            format!(
                "kamishibai {} artifact for {}\n{}\n",
                artifact.label(),
                draft.term(),
                draft.payload().front()
            ),
        )?;
        let size = fs::metadata(&path)?.len();
        Ok(ArtifactFile::new(filename, format!("{size} B"), false))
    }
}

fn default_output() -> Result<PathBuf> {
    Locations::new(LocationArgs::default(), SystemContext).output()
}

fn kind_for(entry: &str) -> CandidateKind {
    if entry.split_whitespace().count() > 1 {
        return CandidateKind::Phrase;
    }
    CandidateKind::Word
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        return String::from(fallback);
    }
    String::from(value)
}

fn extension(artifact: Artifact) -> &'static str {
    match artifact {
        Artifact::Scene => "scene.txt",
        Artifact::Picture => "picture.txt",
        Artifact::Sound => "sound.txt",
    }
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

fn stamp() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => String::from("0"),
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn deck_text(drafts: &[CardDraft]) -> String {
    let mut text = String::from("kamishibai deck\n");
    for draft in drafts {
        text.push_str(draft.term());
        text.push('\n');
    }
    text
}

fn report_text(drafts: &[CardDraft]) -> String {
    let mut text = String::from("kamishibai report\n");
    for draft in drafts {
        text.push_str(draft.payload().front());
        text.push('\n');
        text.push_str(draft.payload().back());
        text.push('\n');
    }
    text
}

fn load_preferences() -> Result<Preferences> {
    let store = default_store(&SystemContext)?;
    store.read()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::tui::Screen;

    fn shell(app: App, output: PathBuf) -> Shell {
        Shell {
            app,
            engine: None,
            producer: LocalArtifactProducer::new(output),
            started: None,
        }
    }

    fn pair() -> LanguagePair {
        LanguagePair::new("en", "ru")
    }

    fn candidate(term: &str) -> WordCandidate {
        WordCandidate::new(term, CandidateKind::Word, format!("{term} preview"), "note")
    }

    fn review() -> App {
        App::new(pair())
            .with_screen(Screen::WhatIUnderstood)
            .confirmed_target("en")
            .understood(vec![candidate("whilst")])
    }

    #[test]
    fn first_pass_keeps_commas_inside_lines() {
        let output = tempdir().expect("temp output must exist");
        let mut shell = shell(
            App::new(pair()).seeded_blob("whilst, in the end\nwreck"),
            output.path().to_path_buf(),
        );
        shell
            .handle(AppEvent::Submit)
            .expect("submit must run understanding");
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
    fn shell_generation_publishes_done_artifacts() {
        let output = tempdir().expect("temp output must exist");
        let mut shell = shell(review(), output.path().to_path_buf());
        shell
            .handle(AppEvent::KeyEnter)
            .expect("enter must start generation");
        for _ in 0..4 {
            shell.tick().expect("generation tick must succeed");
        }
        assert!(
            shell.app.screen() == Screen::Done
                && shell.app.done_artifacts().deck.ends_with(".apkg")
                && shell.app.done_artifacts().report.ends_with(".pdf")
                && !shell.app.done_artifacts().output.is_empty(),
            "generation must publish deck, report, and output path before Done"
        );
    }

    #[test]
    fn shell_drop_artifact_restarts_the_engine_from_current_cards() {
        let output = tempdir().expect("temp output must exist");
        let mut shell = shell(review(), output.path().to_path_buf());
        shell
            .handle(AppEvent::Submit)
            .expect("submit must start generation");
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
