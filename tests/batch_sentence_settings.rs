//! Integration coverage for generation guidance on `What I understood`.

use std::collections::BTreeSet;

use kamishibai::session::{
    LanguagePair, Sense, SentenceBatchSettings, SentenceLevel, SentenceTypeMix, WordCandidate,
};
use kamishibai::tui::{
    App, AppEvent, BatchSettingsRow, ModalKind, MousePointer, ReviewFocus, Screen, Side, draw,
    mouse_pointer_at, review_event_at, scroll_body_width, scroll_viewport, transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

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

fn style_of(app: &App, needle: &str, width: u16, height: u16) -> (Color, Color, Modifier) {
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
            let column = u16::try_from(rendered[..start].chars().count())
                .expect("rendered column must fit the terminal");
            let cell = &buffer[(column, row)];
            return (cell.fg, cell.bg, cell.modifier);
        }
    }
    panic!("the rendered screen never showed '{needle}'");
}

fn line_of(app: &App, needle: &str, width: u16, height: u16) -> String {
    flat_at(app, width, height)
        .lines()
        .find(|line| line.contains(needle))
        .map(str::trim)
        .map(String::from)
        .unwrap_or_else(|| panic!("the rendered screen never showed '{needle}'"))
}

fn choices(app: &App, area: Rect, row: BatchSettingsRow) -> BTreeSet<usize> {
    let mut choices = BTreeSet::new();
    for screen_row in 0..area.height {
        for column in 0..area.width {
            if let Some(AppEvent::SentenceSettingsChoose(found, index)) =
                review_event_at(app, area, column, screen_row)
                && found == row
            {
                choices.insert(index);
            }
        }
    }
    choices
}

#[test]
fn a_wide_review_sheds_the_conventional_keys_before_the_exit() {
    let rendered = flat_at(&review(2), 120, 24);
    assert!(
        rendered.contains("[Ctrl+C] quit")
            && !rendered.contains("[↑↓] nav")
            && !rendered.contains("[Ctrl+L]"),
        "at a normal width the review must shed the keys nobody needs told before anything else: {rendered}"
    );
}

#[test]
fn review_places_quiet_generation_guidance_one_blank_row_above_words() {
    let rendered = flat_at(&review(2), 120, 24);
    let lines = rendered.lines().collect::<Vec<_>>();
    let summary = lines
        .iter()
        .position(|line| line.contains("generation guidance   best fit"))
        .expect("the review must expose the compact generation guidance");
    let words = lines
        .iter()
        .position(|line| line.contains("01  term-01"))
        .expect("the review must expose the first word");
    assert!(
        words == summary + 2
            && lines[summary + 1].trim().is_empty()
            && rendered.contains("[↑] guidance")
            && !rendered.contains("[S] sentences"),
        "generation guidance must read as a quiet list row above the words: {rendered}"
    );
}

#[test]
fn expanded_guidance_hides_summary_chips_and_uses_card_editor_questions() {
    let area = terminal(120, 24);
    let app = review(2).sentence_settings_opened();
    let configured = review(2)
        .with_sentence_settings(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Questions,
        ))
        .sentence_settings_opened();
    let rendered = flat_at(&app, area.width, area.height);
    let active = style_of(&app, "what's the desired level?", area.width, area.height);
    let inactive = style_of(&app, "what kinds of phrases?", area.width, area.height);
    assert!(
        line_of(&app, "generation guidance", area.width, area.height) == "generation guidance"
            && line_of(&configured, "generation guidance", area.width, area.height,)
                == "generation guidance"
            && rendered.contains("what's the desired level?")
            && rendered.contains("what kinds of phrases?")
            && rendered.matches("best fit").count() == 2
            && active.0 == Color::Rgb(0xe6, 0xe3, 0xda)
            && active.2.contains(Modifier::BOLD)
            && inactive.0 == Color::Rgb(0x5a, 0x59, 0x53)
            && !inactive.2.contains(Modifier::BOLD),
        "expanded guidance must replace summary chips with two card-editor-style best-fit questions: {rendered}"
    );
}

#[test]
fn summary_collapses_inferred_axes_and_keeps_explicit_axes_in_fixed_order() {
    let area = terminal(120, 24);
    let defaults = review(2);
    let level = review(2).with_sentence_settings(SentenceBatchSettings::new(
        Some(SentenceLevel::B1),
        SentenceTypeMix::BestFit,
    ));
    let format = review(2)
        .with_sentence_settings(SentenceBatchSettings::new(None, SentenceTypeMix::Questions));
    let both = review(2).with_sentence_settings(SentenceBatchSettings::new(
        Some(SentenceLevel::B1),
        SentenceTypeMix::Questions,
    ));
    assert_eq!(
        [
            line_of(&defaults, "generation guidance", area.width, area.height),
            line_of(&level, "generation guidance", area.width, area.height),
            line_of(&format, "generation guidance", area.width, area.height),
            line_of(&both, "generation guidance", area.width, area.height),
        ],
        [
            String::from("generation guidance   best fit"),
            String::from("generation guidance   b1"),
            String::from("generation guidance   questions"),
            String::from("generation guidance   b1   questions"),
        ],
        "collapsed guidance must show one inferred default or only explicit axes in level-then-format order"
    );
}

#[test]
fn summary_reuses_muted_default_and_brightens_only_explicit_guidance() {
    let area = terminal(120, 24);
    let defaults = review(2);
    let explicit = review(2).with_sentence_settings(SentenceBatchSettings::new(
        Some(SentenceLevel::B1),
        SentenceTypeMix::Questions,
    ));
    assert_eq!(
        (
            style_of(&defaults, "best fit", area.width, area.height),
            style_of(&explicit, "b1", area.width, area.height),
            style_of(&explicit, "questions", area.width, area.height),
        ),
        (
            (
                Color::Rgb(0x0e, 0x0e, 0x10),
                Color::Rgb(0x8b, 0x8a, 0x83),
                Modifier::empty(),
            ),
            (
                Color::Rgb(0x0e, 0x0e, 0x10),
                Color::Rgb(0xe6, 0xe3, 0xda),
                Modifier::empty(),
            ),
            (
                Color::Rgb(0x0e, 0x0e, 0x10),
                Color::Rgb(0xe6, 0xe3, 0xda),
                Modifier::empty(),
            ),
        ),
        "guidance choices must share the generated-card tag hierarchy without competing with the title"
    );
}

#[test]
fn upward_list_navigation_opens_settings_and_downward_navigation_returns_to_words() {
    let opened = transit(review(2), AppEvent::NavPrev).0;
    let level = transit(opened, AppEvent::NavPrev).0;
    let types = transit(level, AppEvent::NavNext).0;
    let closed = transit(types, AppEvent::NavNext).0;
    assert_eq!(
        (
            closed.sentence_settings_editor(),
            closed.selected(),
            closed.candidates().len(),
        ),
        (None, 0, 2),
        "vertical navigation must leave the settings block through the first reviewed word"
    );
}

#[test]
fn upward_navigation_reaches_the_first_word_before_opening_settings() {
    let second = review(2).review_focus_next();
    let first = transit(second, AppEvent::NavPrev).0;
    let opened = transit(first.clone(), AppEvent::NavPrev).0;
    let opened_with_k = transit(first.clone(), AppEvent::KeyChar('k')).0;
    assert_eq!(
        (
            first.selected(),
            first.sentence_settings_editor(),
            opened.sentence_settings_editor(),
            opened_with_k.sentence_settings_editor(),
        ),
        (
            0,
            None,
            Some(BatchSettingsRow::Types),
            Some(BatchSettingsRow::Types),
        ),
        "settings must extend the candidate list instead of stealing an ordinary upward move"
    );
}

#[test]
fn walking_up_from_inside_an_open_sense_list_reaches_its_head_before_guidance() {
    let app = App::new(LanguagePair::new("en", "fr"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("fr")
        .understood(vec![WordCandidate::with_senses(
            "bank",
            vec![
                Sense::plain("a financial institution"),
                Sense::plain("a river edge"),
            ],
            0,
            true,
        )]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let inside = transit(opened, AppEvent::NavNext).0;
    let head = transit(inside, AppEvent::NavPrev).0;
    let guidance = transit(head.clone(), AppEvent::NavPrev).0;
    assert_eq!(
        (
            head.review_focus(),
            head.sentence_settings_editor(),
            head.sense_list_open(0),
            guidance.sentence_settings_editor(),
            guidance.sense_list_open(0),
        ),
        (
            ReviewFocus::Head(0),
            None,
            true,
            Some(BatchSettingsRow::Types),
            true,
        ),
        "the walk skipped the open list head or guidance collapsed the list it walked out of"
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
            SentenceBatchSettings::new(Some(SentenceLevel::A1), SentenceTypeMix::Statements),
            None,
        ),
        "escape must close only the editor and preserve both chosen settings"
    );
}

#[test]
fn re_understanding_and_screen_changes_keep_only_the_durable_choices() {
    let settings = SentenceBatchSettings::new(Some(SentenceLevel::C1), SentenceTypeMix::Mixed);
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
fn enter_closes_the_open_editor_while_printable_keys_stay_owned() {
    let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Questions);
    let app = transit(
        review(2).with_sentence_settings(settings),
        AppEvent::SentenceSettingsOpen,
    )
    .0;
    let enter = transit(app.clone(), AppEvent::KeyEnter).0;
    let drop = transit(app.clone(), AppEvent::KeyChar('D')).0;
    let move_key = transit(app.clone(), AppEvent::KeyChar('J')).0;
    let space = transit(app.clone(), AppEvent::KeyChar(' ')).0;
    assert_eq!(
        (
            enter.sentence_settings_editor(),
            enter.sentence_settings(),
            enter.candidates().len(),
            enter.any_sense_list_open(),
            drop.candidates().len(),
            move_key.selected(),
            space.any_sense_list_open(),
        ),
        (None, settings, 2, false, 2, 0, false),
        "Enter did not collapse generation guidance or another owned key leaked into review controls"
    );
}

#[test]
fn ctrl_g_commits_generation_while_the_editor_is_open() {
    let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Questions);
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
    let summary = cell_of(&closed, "generation guidance   ", area.width, area.height);
    let separator = (summary.0, summary.1 + 1);
    let open = closed.clone().sentence_settings_opened();
    assert_eq!(
        (
            review_event_at(&closed, area, summary.0, summary.1),
            mouse_pointer_at(&closed, area, summary.0, summary.1),
            review_event_at(&closed, area, separator.0, separator.1),
            mouse_pointer_at(&closed, area, separator.0, separator.1),
            mouse_pointer_at(&closed, area, 0, 0),
            choices(&open, area, BatchSettingsRow::Level),
            choices(&open, area, BatchSettingsRow::Types),
        ),
        (
            Some(AppEvent::SentenceSettingsOpen),
            MousePointer::Hand,
            None,
            MousePointer::Arrow,
            MousePointer::Arrow,
            BTreeSet::from([0, 1, 2, 3, 4, 5, 6]),
            BTreeSet::from([0, 1, 2, 3, 4]),
        ),
        "renderer, click dispatch, and pointer policy must agree on the whole settings block"
    );
}

#[test]
fn narrow_guidance_wraps_each_full_carousel_without_losing_hit_regions() {
    let area = terminal(48, 28);
    let app = review(1).sentence_settings_opened();
    let rendered = flat_at(&app, area.width, area.height);
    let lines = rendered.lines().collect::<Vec<_>>();
    let level = lines
        .iter()
        .position(|line| line.contains("what's the desired level?"))
        .expect("narrow guidance must keep the level label");
    let format = lines
        .iter()
        .position(|line| line.contains("what kinds of phrases?"))
        .expect("narrow guidance must keep the format label");
    assert!(
        lines
            .get(level + 1)
            .is_some_and(|line| line.contains("best fit"))
            && lines
                .get(format + 1)
                .is_some_and(|line| line.contains("best fit"))
            && choices(&app, area, BatchSettingsRow::Level)
                == BTreeSet::from([0, 1, 2, 3, 4, 5, 6])
            && choices(&app, area, BatchSettingsRow::Types) == BTreeSet::from([0, 1, 2, 3, 4])
            && rendered.contains("01  term-01"),
        "narrow guidance clipped a label, choice, candidate, or clickable carousel region: {rendered}"
    );
}

#[test]
fn opening_settings_scrolls_a_long_review_to_the_top_carousels() {
    let area = terminal(140, 13);
    let viewport = scroll_viewport(&review(25), area);
    let width = scroll_body_width(area);
    let app = review(25)
        .with_sentence_settings(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Questions,
        ))
        .sentence_settings_opened()
        .sentence_settings_focused(BatchSettingsRow::Types)
        .body_scroll_to_selection(viewport, width);
    let rendered = flat_at(&app, area.width, area.height);
    let questions = cell_of_on_line(
        &app,
        "questions",
        "what kinds of phrases?",
        area.width,
        area.height,
    );
    assert!(
        app.body_scroll() == 0
            && rendered.contains("what kinds of phrases?")
            && rendered.contains("term-01")
            && review_event_at(&app, area, questions.0, questions.1)
                == Some(AppEvent::SentenceSettingsChoose(BatchSettingsRow::Types, 2))
            && mouse_pointer_at(&app, area, questions.0, questions.1) == MousePointer::Hand,
        "the focused batch carousel must anchor above the review in a short viewport: {rendered}"
    );
}

#[test]
fn modal_overlay_suppresses_underlying_sentence_settings_hits() {
    let area = terminal(120, 24);
    let app = review(1);
    let summary = cell_of(&app, "generation guidance   ", area.width, area.height);
    let covered = app.with_modal(ModalKind::PickLanguages);
    assert_eq!(
        review_event_at(&covered, area, summary.0, summary.1),
        None,
        "an overlay must own clicks instead of leaking them to the review body"
    );
}
