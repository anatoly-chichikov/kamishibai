//! TUI entrypoint for the word-first kamishibai flow.
//!
//! The old JSON-first CLI is gone. The binary boots straight into the locked
//! TUI state machine. Heavy generation (scene / picture / sound) will be
//! wired up by CTX-177; this module only owns the terminal shell, the
//! preference store, and the side-effect bridge.

use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyModifiers, poll, read};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::{Preferences, default_store};
use crate::runtime::locations::SystemContext;
use crate::session::{
    CandidateKind, CardDraft, CardPayload, LanguagePair, ScriptDetection, TargetDetection,
    WordCandidate, catalog_for_detection,
};
use crate::tui::{App, Screen, Side, draw, to_app, transit};

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
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let outcome = loop_forever(&mut terminal, app);
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    outcome
}

fn loop_forever<B>(terminal: &mut Terminal<B>, mut app: App) -> Result<()>
where
    B: ratatui::backend::Backend,
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    loop {
        terminal.draw(|frame| draw(frame, &app))?;
        if !poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = read()? else {
            continue;
        };
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        let Some(event) = to_app(key) else { continue };
        let (next, side) = transit(app.clone(), event);
        app = apply_side(next, side)?;
        if app.screen() == Screen::Done && key.code == KeyCode::Char('q') {
            return Ok(());
        }
    }
}

fn apply_side(app: App, side: Side) -> Result<App> {
    match side {
        Side::RunUnderstanding => {
            let guess = ScriptDetection.detect(app.blob(), &catalog_for_detection())?;
            let candidates = first_pass(app.blob());
            Ok(app.confirmed_target(guess.code()).understood(candidates))
        }
        Side::StartGeneration => {
            let drafts = drafts_from(&app);
            Ok(app.cards_started(drafts))
        }
        Side::PersistMyLanguage(code) => {
            if let Ok(store) = default_store(&SystemContext) {
                let _ = store.write(&Preferences::new(code));
            }
            Ok(app)
        }
        Side::ExitApp | Side::None => Ok(app),
        _ => Ok(app),
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
                    candidate.preview(),
                    candidate.note(),
                    candidate.term(),
                ),
            )
        })
        .collect()
}

/// Deterministic placeholder for the cheap understanding pass until CTX-177
/// wires a real Gemini implementation. Splits the raw blob on newlines and
/// commas, trims empty entries, and wraps each row as a generic word candidate.
fn first_pass(blob: &str) -> Vec<WordCandidate> {
    blob.split(['\n', ','])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| WordCandidate::new(entry, CandidateKind::Word, String::new(), String::new()))
        .collect()
}

fn load_preferences() -> Result<Preferences> {
    let store = default_store(&SystemContext)?;
    store.read()
}
