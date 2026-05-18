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
use kamishibai::tui::{App, AppEvent, BusyKind, Screen, Side, draw, to_app, transit};
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

fn long_blob(count: usize) -> String {
    (1..=count)
        .map(|index| format!("word-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n")
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
fn long_pasted_your_words_scrolls_to_the_cursor_line() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob(long_blob(60))
        .body_scroll_to_selection(7, 72);
    let flat = flatten(&app);
    assert!(
        flat.contains("word-60") && !flat.contains("word-01"),
        "long pasted word lists must render the cursor end of the scrollable editor: {flat}"
    );
}

#[test]
fn repeated_enter_on_your_words_moves_the_editor_scroll() {
    let mut app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("word-01")
        .body_scroll_to_selection(4, 72);
    for _ in 0..12 {
        app = transit(app, AppEvent::KeyEnter)
            .0
            .body_scroll_to_selection(4, 72);
    }
    assert!(
        app.body_scroll() > 0,
        "typing past the visible editor height must advance the body scroll"
    );
}

#[test]
fn your_words_renders_placeholder_tagline_and_language_pair() {
    let app = App::new(LanguagePair::new("en", "ru")).confirmed_target("en");
    let flat = flatten(&app);
    assert!(
        flat.contains("words you want to learn")
            && flat.contains("step 1/3")
            && flat.contains("→ EN"),
        "your words screen must render the PDF labels and a language chip on the right: {flat}"
    );
}

#[test]
fn your_words_footer_shows_language_shortcut() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let flat = flatten(&app);
    assert!(
        flat.contains("[Ctrl+L] language"),
        "your words footer must reveal the language picker shortcut: {flat}"
    );
}

#[test]
fn your_words_footer_keeps_paste_shortcut() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let flat = flatten(&app);
    assert!(
        flat.contains("[Cmd+V] paste"),
        "your words footer must keep the paste shortcut: {flat}"
    );
}

#[test]
fn busy_loader_covers_the_current_screen_with_request_status() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .busy_started(BusyKind::Understanding)
        .busy_elapsed(Duration::from_millis(320));
    let flat = flatten(&app);
    assert!(
        flat.contains("working")
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
        flat.contains("can't reach gemini")
            && flat.contains("INTERNAL: boom")
            && flat.contains("press any key to dismiss"),
        "recoverable Gemini errors must render as an in-app overlay"
    );
}

#[test]
fn ctrl_l_on_your_words_opens_the_language_picker() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (next, side) = transit(
        app,
        to_app(modified(KeyCode::Char('l'), KeyModifiers::CONTROL)).expect("map"),
    );
    assert_eq!(
        (next.modal(), side),
        (Some(kamishibai::tui::ModalKind::PickMyLanguage), Side::None,),
        "Ctrl+L on Your words must open the language picker modal without persisting yet"
    );
}

#[test]
fn cmd_l_on_your_words_opens_the_language_picker() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (next, side) = transit(
        app,
        to_app(modified(KeyCode::Char('l'), KeyModifiers::SUPER)).expect("map"),
    );
    assert_eq!(
        (next.modal(), side),
        (Some(kamishibai::tui::ModalKind::PickMyLanguage), Side::None,),
        "Cmd+L on Your words must open the language picker modal in kitty-protocol terminals"
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
fn left_arrow_on_your_words_inserts_before_the_last_character() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("harbor");
    let (after_left, _) = transit(app, to_app(press(KeyCode::Left)).expect("Left must map"));
    let (after_type, side) = transit(
        after_left,
        to_app(press(KeyCode::Char('u'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("harbour"), Side::None),
        "left arrow on Your words must move the text cursor before the next insertion"
    );
}

#[test]
fn left_arrow_on_your_words_moves_over_utf8_characters() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("окно");
    let (after_left, _) = transit(app, to_app(press(KeyCode::Left)).expect("Left must map"));
    let (after_type, side) = transit(
        after_left,
        to_app(press(KeyCode::Char('!'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("окн!о"), Side::None),
        "left arrow on Your words must move by UTF-8 character boundaries instead of bytes"
    );
}

#[test]
fn up_arrow_on_your_words_inserts_on_the_previous_line() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("moon\nship");
    let (after_up, _) = transit(app, to_app(press(KeyCode::Up)).expect("Up must map"));
    let (after_type, side) = transit(
        after_up,
        to_app(press(KeyCode::Char('!'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("moon!\nship"), Side::None),
        "up arrow on Your words must move the text cursor onto the previous line"
    );
}

#[test]
fn arrows_on_empty_your_words_materialize_the_requested_position() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (after_down, _) = transit(app, to_app(press(KeyCode::Down)).expect("Down must map"));
    let (after_right, _) = transit(
        after_down,
        to_app(press(KeyCode::Right)).expect("Right must map"),
    );
    let (after_type, side) = transit(
        after_right,
        to_app(press(KeyCode::Char('x'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("\n x"), Side::None),
        "arrows on empty Your words must let the first typed character land at the chosen row and column"
    );
}

#[test]
fn typing_and_pressing_ctrl_g_advances_to_what_i_understood_and_locks_target_language() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let mut state = app;
    for symbol in "окно".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit =
        to_app(modified(KeyCode::Char('g'), KeyModifiers::CONTROL)).expect("Ctrl+G must map");
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
        "Ctrl+G on non-empty blob must move to What I understood, request understanding, and confirm the detected target language"
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
