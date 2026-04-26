//! Integration flow for `app start -> Your words -> What I understood`.
//!
//! Exercises the real input mapper, transition function, renderer, and the
//! language pair the shell persists. No real Gemini: the understanding pass
//! is mocked with `ScriptDetection`, which is the deterministic fallback the
//! production code uses when the LLM is unavailable.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::languages::catalog;
use kamishibai::session::{LanguagePair, ScriptDetection, TargetDetection};
use kamishibai::tui::{App, BusyKind, Screen, Side, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::tempdir;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn flatten(app: &App) -> String {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let buffer = terminal.backend().buffer();
    let mut flat = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            flat.push_str(buffer[(column, row)].symbol());
        }
        flat.push('\n');
    }
    flat
}

fn apply_side(app: App, side: Side) -> App {
    match side {
        Side::RunUnderstanding => {
            let guess = ScriptDetection
                .detect(app.blob(), &catalog())
                .expect("detection must succeed");
            app.confirmed_target(guess.code())
        }
        _ => app,
    }
}

#[test]
fn your_words_renders_placeholder_tagline_and_language_pair() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let flat = flatten(&app);
    assert!(
        flat.contains("Your words")
            && flat.contains("paste anything — I figure out the rest")
            && flat.contains("type or paste one word/phrase per line:")
            && flat.contains("one per line")
            && flat.contains("Ctrl+L")
            && flat.contains("Shift+Enter")
            && !flat.contains("минимум трения")
            && flat.contains("kamishibai ·")
            && flat.contains("→ RU"),
        "Your words screen must render the PDF labels and a language pair badge on top, without any design-tool commentary"
    );
}

#[test]
fn busy_loader_covers_the_current_screen_with_request_status() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .busy_started(BusyKind::Understanding)
        .busy_elapsed(Duration::from_millis(320));
    let flat = flatten(&app);
    assert!(
        flat.contains("Working")
            && flat.contains("understanding your words")
            && flat.contains("the request is still running"),
        "busy loader must cover the current screen with a visible request status"
    );
}

#[test]
fn busy_loader_owns_keyboard_input_until_request_finishes() {
    let app = App::new(LanguagePair::new("en", "ru")).busy_started(BusyKind::Understanding);
    let event = to_app(press(KeyCode::Char('x'))).expect("char must map");
    let (next, side) = transit(app, event);
    assert_eq!(
        (
            next.blob().to_string(),
            next.busy().map(|busy| busy.kind()),
            side,
        ),
        (String::new(), Some(BusyKind::Understanding), Side::None),
        "busy loader must suppress ordinary keyboard input until the request finishes"
    );
}

#[test]
fn recoverable_error_overlay_keeps_the_message_visible() {
    let app = App::new(LanguagePair::new("en", "ru")).error_shown("INTERNAL: boom");
    let flat = flatten(&app);
    assert!(
        flat.contains("Не получилось")
            && flat.contains("запрос к Gemini завершился ошибкой")
            && flat.contains("INTERNAL: boom")
            && flat.contains("нажми любую клавишу"),
        "recoverable Gemini errors must render as an in-app overlay"
    );
}

#[test]
fn ctrl_l_on_your_words_toggles_my_language() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (next, side) = transit(
        app,
        to_app(modified(KeyCode::Char('l'), KeyModifiers::CONTROL)).expect("map"),
    );
    assert_eq!(
        (next.pair().support().to_string(), side),
        (
            String::from("es"),
            Side::PersistMyLanguage(String::from("es"))
        ),
        "Ctrl+L on Your words must rotate `my language`"
    );
}

#[test]
fn enter_on_empty_blob_inserts_newline_and_stays_on_your_words() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let event = to_app(press(KeyCode::Enter)).expect("Enter must map");
    let (next, side) = transit(app, event);
    assert_eq!(
        (next.screen(), next.blob().to_string(), side),
        (Screen::YourWords, String::from("\n"), Side::None),
        "plain Enter on an empty blob must insert a newline without side effects"
    );
}

#[test]
fn typing_and_pressing_shift_enter_advances_to_what_i_understood_and_locks_target_language() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let mut state = app;
    for symbol in "окно".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit = to_app(modified(KeyCode::Enter, KeyModifiers::SHIFT)).expect("Enter must map");
    let (after_submit, side) = transit(state, submit);
    let resolved = apply_side(after_submit, side.clone());
    assert_eq!(
        (
            resolved.screen(),
            side,
            resolved.target_pending(),
            resolved.pair().target().to_string(),
        ),
        (
            Screen::WhatIUnderstood,
            Side::RunUnderstanding,
            false,
            String::from("ru"),
        ),
        "Shift+Enter on non-empty blob must move to What I understood, request understanding, and confirm the detected target language"
    );
}

#[test]
fn preference_store_feeds_my_language_into_the_initial_pair() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("es"))
        .expect("must persist my language");
    let persisted = store.read().expect("must reload my language").my_language;
    let app = App::new(LanguagePair::new("en", persisted.as_str()));
    assert_eq!(
        app.pair().support(),
        "es",
        "persisted my language must feed into the initial pair at app start"
    );
}
