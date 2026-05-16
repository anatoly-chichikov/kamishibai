//! Keyboard-driven integration flow: `Your words -> What I understood`.
//!
//! Feeds real `crossterm::event::KeyEvent` values through the locked-in input
//! mapper and transition function, then renders each intermediate state to a
//! `TestBackend` buffer and checks its contents. All LLM calls are mocked —
//! the session-engine events are fabricated inline.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use kamishibai::session::LanguagePair;
use kamishibai::tui::{App, AppEvent, Screen, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn release(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Release)
}

fn render_contains(app: &App, needle: &str) -> bool {
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
    flat.contains(needle)
}

#[test]
fn plain_enter_on_your_words_inserts_newline_without_advancing() {
    let mut state = App::new(LanguagePair::new("en", "ru"));
    for symbol in "whilst".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit = to_app(press(KeyCode::Enter)).expect("Enter must map");
    assert_eq!(
        submit,
        AppEvent::KeyEnter,
        "plain Enter must produce the physical Enter event"
    );
    let (after, _) = transit(state, submit);
    assert_eq!(
        (after.screen(), after.blob().to_string()),
        (Screen::YourWords, String::from("whilst\n")),
        "plain Enter on Your words must insert a newline and stay in the textarea"
    );
}

#[test]
fn shift_enter_on_your_words_advances_to_what_i_understood_with_language_pair_visible() {
    let mut state = App::new(LanguagePair::new("en", "ru"));
    for symbol in "whilst".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit = to_app(modified(KeyCode::Enter, KeyModifiers::SHIFT)).expect("map");
    assert_eq!(
        submit,
        AppEvent::Submit,
        "Shift+Enter must produce the semantic submit event"
    );
    let (after, _) = transit(state, submit);
    let next = after.confirmed_target("en");
    assert_eq!(
        (
            next.screen(),
            render_contains(&next, "what i understood"),
            render_contains(&next, "RU → EN"),
        ),
        (Screen::WhatIUnderstood, true, true),
        "after typing and pressing Shift+Enter the shell must render What I understood with a visible language pair"
    );
}

#[test]
fn ctrl_d_on_your_words_does_not_submit_or_type() {
    let mut state = App::new(LanguagePair::new("en", "ru"));
    for symbol in "whilst".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit = to_app(modified(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(
        (submit, state.screen(), state.blob().to_string()),
        (None, Screen::YourWords, String::from("whilst")),
        "Ctrl+D must not submit and must not type a stray d"
    );
}

#[test]
fn release_events_from_keyboard_enhancement_do_not_type_twice() {
    let press = to_app(KeyEvent::new_with_kind(
        KeyCode::Char('w'),
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ));
    let release = to_app(release(KeyCode::Char('w'), KeyModifiers::NONE));
    assert_eq!(
        (press, release),
        (Some(AppEvent::KeyChar('w')), None),
        "release events from enhanced keyboard mode must not duplicate typed characters"
    );
}

#[test]
fn left_arrow_maps_to_the_text_cursor_event() {
    let event = to_app(press(KeyCode::Left));
    assert_eq!(
        event,
        Some(AppEvent::CursorLeft),
        "left arrow must reach text editors as a cursor move instead of list navigation"
    );
}

#[test]
fn release_submit_from_keyboard_enhancement_does_not_submit_twice() {
    let press = to_app(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::SHIFT,
        KeyEventKind::Press,
    ));
    let release = to_app(release(KeyCode::Enter, KeyModifiers::SHIFT));
    assert_eq!(
        (press, release),
        (Some(AppEvent::Submit), None),
        "release events from enhanced keyboard mode must not duplicate submit"
    );
}
