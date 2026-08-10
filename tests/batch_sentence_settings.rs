//! Integration coverage for sentence settings on `What I understood`.

use std::collections::BTreeSet;

use kamishibai::session::{
    LanguagePair, SentenceBatchSettings, SentenceLevel, SentenceTypeMix, WordCandidate,
};
use kamishibai::tui::{
    App, AppEvent, BatchSettingsRow, ModalKind, MousePointer, Screen, Side, draw, mouse_pointer_at,
    scroll_body_width, scroll_viewport, sentence_settings_event_at, transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn review(candidates: usize) -> App {
    App::new(LanguagePair::new("en", "fr"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("fr")
        .understood(
            (1..=candidates)
                .map(|index| {
                    WordCandidate::new(
                        format!("term-{index:02}"),
                        format!("understanding for term-{index:02}"),
                        true,
                    )
                })
                .collect(),
        )
}

fn terminal(width: u16, height: u16) -> Rect {
    Rect::new(0, 0, width, height)
}

fn flat_at(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("backend must be available");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let buffer = terminal.backend().buffer();
    let mut rendered = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        rendered.push('\n');
    }
    rendered
}

fn cell_of(app: &App, needle: &str, width: u16, height: u16) -> (u16, u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("backend must be available");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = rendered[..start].chars().count();
            return (
                u16::try_from(column).expect("rendered column must fit the terminal"),
                row,
            );
        }
    }
    panic!("the rendered screen never showed '{needle}'");
}

fn cell_of_on_line(
    app: &App,
    needle: &str,
    companion: &str,
    width: u16,
    height: u16,
) -> (u16, u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("backend must be available");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if rendered.contains(companion)
            && let Some(start) = rendered.find(needle)
        {
            let column = rendered[..start].chars().count();
            return (
                u16::try_from(column).expect("rendered column must fit the terminal"),
                row,
            );
        }
    }
    panic!("the rendered screen never showed '{needle}' beside '{companion}'");
}

fn choices(app: &App, area: Rect, row: BatchSettingsRow) -> BTreeSet<usize> {
    let mut choices = BTreeSet::new();
    for screen_row in 0..area.height {
        for column in 0..area.width {
            if let Some(AppEvent::SentenceSettingsChoose(found, index)) =
                sentence_settings_event_at(app, area, column, screen_row)
                && found == row
            {
                choices.insert(index);
            }
        }
    }
    choices
}

#[test]
fn review_shows_the_exact_default_sentence_summary_and_open_hint() {
    let rendered = flat_at(&review(2), 120, 24);
    assert!(
        rendered.contains("sentences: level — · types natural")
            && rendered.contains("[S] sentences")
            && rendered.contains("[Esc] back"),
        "the closed review must expose the batch sentence summary, settings shortcut, and back action: {rendered}"
    );
}

#[test]
fn keyboard_changes_both_rows_and_escape_closes_without_losing_choices() {
    let app = transit(review(2), AppEvent::KeyChar('S')).0;
    let app = transit(app, AppEvent::CursorRight).0;
    let app = transit(app, AppEvent::NavNext).0;
    let app = transit(app, AppEvent::CursorRight).0;
    let app = transit(app, AppEvent::Cancel).0;
    assert_eq!(
        (app.sentence_settings(), app.sentence_settings_editor()),
        (
            SentenceBatchSettings::new(Some(SentenceLevel::A1), SentenceTypeMix::Varied),
            None,
        ),
        "escape must close only the editor and preserve both chosen settings"
    );
}

#[test]
fn re_understanding_and_screen_changes_keep_only_the_durable_choices() {
    let settings = SentenceBatchSettings::new(Some(SentenceLevel::C1), SentenceTypeMix::Varied);
    let app = review(2)
        .with_sentence_settings(settings)
        .sentence_settings_opened()
        .understood(vec![WordCandidate::new(
            "revised",
            "a revised understanding",
            true,
        )])
        .with_screen(Screen::YourWords)
        .with_screen(Screen::WhatIUnderstood);
    assert_eq!(
        (app.sentence_settings(), app.sentence_settings_editor()),
        (settings, None),
        "re-review and screen movement must preserve choices while closing the ephemeral row focus"
    );
}

#[test]
fn open_editor_owns_enter_drop_navigation_and_printable_keys() {
    let app = transit(review(2), AppEvent::SentenceSettingsOpen).0;
    let enter = transit(app.clone(), AppEvent::KeyEnter).0;
    let drop = transit(app.clone(), AppEvent::KeyChar('D')).0;
    let move_key = transit(app.clone(), AppEvent::KeyChar('J')).0;
    let space = transit(app.clone(), AppEvent::KeyChar(' ')).0;
    assert_eq!(
        (
            enter.candidates().len(),
            enter.expanded_sense(),
            drop.candidates().len(),
            move_key.selected(),
            space.expanded_sense(),
        ),
        (2, None, 2, 0, None),
        "an open settings editor must prevent review controls from receiving its keys"
    );
}

#[test]
fn ctrl_g_commits_generation_while_the_editor_is_open() {
    let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Varied);
    let app = review(2)
        .with_sentence_settings(settings)
        .sentence_settings_opened();
    let (next, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        (
            next.screen(),
            side,
            next.sentence_settings(),
            next.sentence_settings_editor(),
        ),
        (Screen::YourCards, Side::StartGeneration, settings, None),
        "Ctrl+G must keep the allocated settings while closing the ephemeral editor"
    );
}

#[test]
fn summary_and_every_carousel_choice_share_mouse_hit_geometry() {
    let area = terminal(120, 24);
    let closed = review(1);
    let summary = cell_of(&closed, "sentences:", area.width, area.height);
    let open = closed.clone().sentence_settings_opened();
    assert_eq!(
        (
            sentence_settings_event_at(&closed, area, summary.0, summary.1),
            mouse_pointer_at(&closed, area, summary.0, summary.1),
            mouse_pointer_at(&closed, area, 0, 0),
            choices(&open, area, BatchSettingsRow::Level),
            choices(&open, area, BatchSettingsRow::Types),
        ),
        (
            Some(AppEvent::SentenceSettingsOpen),
            MousePointer::Hand,
            MousePointer::Arrow,
            BTreeSet::from([0, 1, 2, 3, 4, 5, 6]),
            BTreeSet::from([0, 1]),
        ),
        "renderer, click dispatch, and pointer policy must agree on the whole settings block"
    );
}

#[test]
fn opening_settings_scrolls_a_long_review_to_the_focused_carousel() {
    let area = terminal(140, 13);
    let viewport = scroll_viewport(&review(25), area);
    let width = scroll_body_width(area);
    let app = review(25)
        .with_sentence_settings(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Varied,
        ))
        .sentence_settings_opened()
        .sentence_settings_focused(BatchSettingsRow::Types)
        .body_scroll_to_selection(viewport, width);
    let rendered = flat_at(&app, area.width, area.height);
    let varied = cell_of_on_line(
        &app,
        "varied",
        "how to mix the types?",
        area.width,
        area.height,
    );
    assert!(
        app.body_scroll() > 0
            && rendered.contains("how to mix the types?")
            && !rendered.contains("term-01")
            && sentence_settings_event_at(&app, area, varied.0, varied.1)
                == Some(AppEvent::SentenceSettingsChoose(BatchSettingsRow::Types, 1))
            && mouse_pointer_at(&app, area, varied.0, varied.1) == MousePointer::Hand,
        "the focused batch carousel must scroll into a short viewport: {rendered}"
    );
}

#[test]
fn modal_overlay_suppresses_underlying_sentence_settings_hits() {
    let area = terminal(120, 24);
    let app = review(1);
    let summary = cell_of(&app, "sentences:", area.width, area.height);
    let covered = app.with_modal(ModalKind::PickMyLanguage);
    assert_eq!(
        sentence_settings_event_at(&covered, area, summary.0, summary.1),
        None,
        "an overlay must own clicks instead of leaking them to the review body"
    );
}
