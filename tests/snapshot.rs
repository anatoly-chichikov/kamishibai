//! Deterministic render snapshots for every locked-in screen.
//!
//! Uses `ratatui::backend::TestBackend` + `insta` snapshot review. No real
//! terminal, no Gemini calls, no background threads.

use kamishibai::session::{LanguagePair, Sense, WordCandidate};
use kamishibai::tui::{App, AppEvent, KeySource, Screen, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn render(app: &App) -> String {
    render_sized(app, 80, 12)
}

fn render_sized(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
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
fn header_title_is_inverted_in_the_render_buffer() {
    let backend = TestBackend::new(96, 16);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::YourCards);
    terminal
        .draw(|frame| draw(frame, &app))
        .expect("draw must succeed");
    let cell = &terminal.backend().buffer()[(5, 1)];
    assert_eq!(
        (cell.fg, cell.bg),
        (Color::Rgb(0x0e, 0x0e, 0x10), Color::Rgb(0xe6, 0xe3, 0xda)),
        "header title must render black ink on a cream block"
    );
}

#[test]
fn welcome_key_step_without_env_locks_submit_only_layout() {
    let app = App::new(LanguagePair::new("fr", "en"))
        .opening_welcome(KeySource::Empty, String::new(), false)
        .welcome_advance();
    insta::assert_snapshot!("welcome_key_step_no_env", render_sized(&app, 96, 16));
}

#[test]
fn welcome_key_step_with_env_locks_load_from_env_chip() {
    let app = App::new(LanguagePair::new("fr", "en"))
        .opening_welcome(KeySource::Empty, String::new(), true)
        .welcome_advance()
        .welcome_focus_next();
    insta::assert_snapshot!("welcome_key_step_with_env", render_sized(&app, 96, 16));
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

#[test]
fn pick_my_language_modal_snapshot_locks_picker_layout() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::WhatIUnderstood);
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker);
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    terminal
        .draw(|frame| draw(frame, &opened))
        .expect("draw must succeed");
    let mut buffer = String::new();
    for row in 0..terminal.backend().buffer().area.height {
        for column in 0..terminal.backend().buffer().area.width {
            let cell = &terminal.backend().buffer()[(column, row)];
            buffer.push_str(cell.symbol());
        }
        buffer.push('\n');
    }
    insta::assert_snapshot!("pick_my_language_modal", buffer);
}

#[test]
fn header_hint_sits_left_of_language_chip_on_every_screen() {
    let pair = LanguagePair::new("en", "ru");
    let cases = [
        (
            Screen::YourWords,
            "each word becomes a small learning scene",
        ),
        (
            Screen::WhatIUnderstood,
            "quick check before i build the cards",
        ),
        (Screen::YourCards, "drawing each card one by one"),
        (Screen::Done, "all done"),
    ];
    for (screen, hint) in cases {
        let app = App::new(pair.clone()).with_screen(screen);
        let buffer = render(&app);
        let header_row = buffer.lines().nth(1).expect("header row must render");
        let hint_pos = header_row
            .find(hint)
            .unwrap_or_else(|| panic!("hint missing on {screen:?} header row"));
        let chip_pos = header_row
            .find("RU")
            .unwrap_or_else(|| panic!("language chip missing on {screen:?} header row"));
        assert!(
            hint_pos < chip_pos,
            "contextual hint must sit left of the language chip on {screen:?} (hint at {hint_pos}, chip at {chip_pos})"
        );
    }
}

#[test]
fn what_i_understood_multi_meaning_snapshot_locks_the_block() {
    let candidate = WordCandidate::with_selected_senses(
        "bank",
        vec![
            Sense::tagged("Сущ. «банк», финансовое учреждение.", "фин."),
            Sense::plain("Сущ. «берег» реки или водоёма."),
            Sense::tagged("Гл. «наклонять(ся)» при повороте самолёта.", "авиац."),
        ],
        vec![0, 1],
        true,
    );
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![candidate]);
    insta::assert_snapshot!("what_i_understood_multi_meaning", render(&app));
}
