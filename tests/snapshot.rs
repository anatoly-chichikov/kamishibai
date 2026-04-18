//! Deterministic render snapshots for every locked-in screen.
//!
//! Uses `ratatui::backend::TestBackend` + `insta` snapshot review. No real
//! terminal, no Gemini calls, no background threads.

use kamishibai::session::LanguagePair;
use kamishibai::tui::{App, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn render(app: &App) -> String {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let mut buffer = String::new();
    for row in 0..terminal.backend().buffer().area.height {
        for column in 0..terminal.backend().buffer().area.width {
            let cell = &terminal.backend().buffer()[(column, row)];
            buffer.push_str(cell.symbol());
        }
        buffer.push('\n');
    }
    buffer
}

#[test]
fn your_words_snapshot_locks_initial_layout() {
    let app = App::new(LanguagePair::new("en", "ru"));
    insta::assert_snapshot!("your_words", render(&app));
}

#[test]
fn what_i_understood_snapshot_locks_second_screen() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::WhatIUnderstood);
    insta::assert_snapshot!("what_i_understood", render(&app));
}

#[test]
fn your_cards_snapshot_locks_work_screen() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::YourCards);
    insta::assert_snapshot!("your_cards", render(&app));
}

#[test]
fn done_snapshot_locks_final_screen() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::Done);
    insta::assert_snapshot!("done", render(&app));
}
