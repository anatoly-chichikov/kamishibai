//! Deterministic render snapshots for every locked-in screen.
//!
//! Uses `ratatui::backend::TestBackend` + `insta` snapshot review. No real
//! terminal, no Gemini calls, no background threads.

use kamishibai::session::{
    LanguagePair, Sense, SentenceBatchSettings, SentenceLevel, SentenceTypeMix, WordCandidate,
};
use kamishibai::tui::{App, AppEvent, BusyKind, KeySource, ModalKind, Screen, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

const AI_DISCLAIMER: &str = "ai may be wrong, please verify results";

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

fn has_fixed_ai_disclaimer(app: &App, width: u16, height: u16) -> bool {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let expected_row = height - 3;
    let disclaimer_width =
        u16::try_from(AI_DISCLAIMER.chars().count()).expect("disclaimer must fit in u16");
    let gutter = if width >= disclaimer_width + 8 { 4 } else { 0 };
    let expected_column = width - disclaimer_width - gutter;
    let buffer = terminal.backend().buffer();
    let visible = AI_DISCLAIMER.chars().enumerate().all(|(index, character)| {
        let offset = u16::try_from(index).expect("disclaimer offset must fit in u16");
        buffer[(expected_column + offset, expected_row)].symbol() == character.to_string()
    });
    let muted = (0..disclaimer_width).all(|offset| {
        let cell = &buffer[(expected_column + offset, expected_row)];
        cell.fg == Color::Rgb(0x5a, 0x59, 0x53) && cell.bg == Color::Rgb(0x0e, 0x0e, 0x10)
    });
    visible && muted
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
fn ai_disclaimer_stays_right_aligned_above_the_divider_on_every_screen() {
    let width = 96;
    let height = 16;
    let pair = LanguagePair::new("en", "ru");
    let apps = [
        App::new(pair.clone()).opening_welcome(KeySource::Empty, String::new(), false),
        App::new(pair.clone()).with_screen(Screen::YourWords),
        App::new(pair.clone()).with_screen(Screen::WhatIUnderstood),
        App::new(pair.clone()).with_screen(Screen::YourCards),
        App::new(pair).with_screen(Screen::Done),
    ];
    let mut failures = Vec::new();
    for app in apps {
        if !has_fixed_ai_disclaimer(&app, width, height) {
            failures.push(app.screen());
        }
    }
    assert!(
        failures.is_empty(),
        "AI disclaimer must stay right-aligned in muted ink above the divider on every screen, failed on {failures:?}"
    );
}

#[test]
fn ai_disclaimer_remains_visible_while_overlays_are_open() {
    let width = 96;
    let height = 16;
    let pair = LanguagePair::new("en", "ru");
    let apps = [
        App::new(pair.clone())
            .with_screen(Screen::WhatIUnderstood)
            .with_modal(ModalKind::ChangeSomething),
        App::new(pair.clone()).busy_started(BusyKind::Understanding),
        App::new(pair).error_shown("INTERNAL: boom"),
    ];
    let visible = apps
        .iter()
        .all(|app| has_fixed_ai_disclaimer(app, width, height));
    assert!(
        visible,
        "AI disclaimer must remain visible while modal, busy, and error overlays are open"
    );
}

#[test]
fn ai_disclaimer_uses_the_full_terminal_width_when_the_copy_fits() {
    let app = App::new(LanguagePair::new("en", "ru"));
    assert!(
        has_fixed_ai_disclaimer(&app, 38, 8),
        "AI disclaimer must not be clipped by the content gutter when the terminal can fit it"
    );
}

#[test]
fn ai_disclaimer_leaves_the_divider_uninterrupted_below_it() {
    let width = 96;
    let height = 16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    let app = App::new(LanguagePair::new("en", "ru"));
    terminal
        .draw(|frame| draw(frame, &app))
        .expect("draw must succeed");
    let buffer = terminal.backend().buffer();
    let row = height - 2;
    let ruled = buffer[(0, row)].modifier.contains(Modifier::CROSSED_OUT)
        && buffer[(94, row)].modifier.contains(Modifier::CROSSED_OUT);
    assert!(
        ruled,
        "AI disclaimer must leave the dashed divider uninterrupted on the row below it"
    );
}

#[test]
fn modal_input_cursor_stays_inside_an_extremely_short_terminal() {
    let backend = TestBackend::new(10, 2);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .with_modal(ModalKind::ChangeSomething);
    terminal
        .draw(|frame| draw(frame, &app))
        .expect("draw must succeed");
    let cursor = terminal
        .get_cursor_position()
        .expect("cursor position must remain readable");
    assert!(
        cursor.x < 10 && cursor.y < 2,
        "modal input cursor must not escape an extremely short terminal, got {cursor:?}"
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
fn welcome_language_row_keeps_the_dutch_chip_visible_at_standard_width() {
    let app = App::new(LanguagePair::new("fr", "en")).opening_welcome(
        KeySource::Empty,
        String::new(),
        false,
    );
    assert!(
        render_sized(&app, 80, 16).contains(" NL "),
        "welcome language row clipped the Dutch chip at standard width"
    );
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
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::Done)
        .done_published(
            "~/Documents/Kamishibai/RU_cards.apkg",
            "~/Documents/Kamishibai/RU_cards.pdf",
            "~/Documents/Kamishibai",
        );
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
fn pick_my_language_modal_keeps_the_dutch_chip_visible() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::WhatIUnderstood);
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker);
    assert!(
        render_sized(&opened, 80, 16).contains(" NL "),
        "language picker clipped the Dutch chip"
    );
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

#[test]
fn batch_sentence_settings_snapshot_locks_the_inline_editor() {
    let app = App::new(LanguagePair::new("en", "fr"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("fr")
        .understood(vec![WordCandidate::new(
            "chouette",
            "an owl or something excellent",
            true,
        )])
        .with_sentence_settings(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Varied,
        ))
        .sentence_settings_opened();
    insta::assert_snapshot!("batch_sentence_settings", render_sized(&app, 96, 16));
}
