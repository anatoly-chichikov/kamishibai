//! Keyboard-driven integration flow: `Your words -> What I understood`.
//!
//! Feeds real `crossterm::event::KeyEvent` values through the locked-in input
//! mapper and transition function, then renders each intermediate state to a
//! `TestBackend` buffer and checks its contents. All LLM calls are mocked —
//! the session-engine events are fabricated inline.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::session::LanguagePair;
use kamishibai::tui::{App, AppEvent, Screen, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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
fn enter_on_your_words_advances_to_what_i_understood_with_language_pair_visible() {
    let start = App::new(LanguagePair::new("en", "ru"));
    let event = to_app(press(KeyCode::Enter)).expect("Enter must map to Submit");
    assert_eq!(event, AppEvent::Submit, "Enter must produce a Submit event");
    let (next, _) = transit(start, event);
    assert!(
        next.screen() == Screen::WhatIUnderstood
            && render_contains(&next, "What I understood")
            && render_contains(&next, "EN → RU"),
        "after pressing Enter the shell must render What I understood with a visible language pair"
    );
}
