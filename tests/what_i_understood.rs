//! Integration flow for `Your words -> What I understood -> drop item -> make cards`.
//!
//! Uses the real input mapper, renderer, and transition function. The LLM
//! understanding pass is replaced with an inline `Understanding` fake.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::session::TargetGuess;
use kamishibai::session::{LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate};
use kamishibai::tui::{App, AppEvent, ModalKind, Screen, Side, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn flat(app: &App) -> String {
    let backend = TestBackend::new(140, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
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

fn modifiers(app: &App, needle: &str) -> Vec<Modifier> {
    let backend = TestBackend::new(140, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = rendered[..start].chars().count() as u16;
            return (0..needle.chars().count())
                .map(|offset| buffer[(column + offset as u16, row)].modifier)
                .collect();
        }
    }
    Vec::new()
}

fn many_candidates(count: usize) -> Vec<WordCandidate> {
    (1..=count)
        .map(|index| {
            WordCandidate::new(
                format!("term-{index:02}"),
                format!("understanding for term-{index:02}"),
                true,
            )
        })
        .collect()
}

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
        Ok(Understood::new(
            TargetGuess::new("en", true),
            vec![
                WordCandidate::new(
                    "sincerely",
                    "Наречие «искренне» — формальная закрывающая фраза в письмах.",
                    true,
                ),
                WordCandidate::new(
                    "expel",
                    "Глагол «исключить» в смысле учебного заведения, не «выпустить газ».",
                    true,
                ),
                WordCandidate::new("at the end", "Фраза о времени или месте — «в конце».", true),
                WordCandidate::new(
                    "celebratory",
                    "Прилагательное «праздничный»; в исходнике опечатка, исправлено.",
                    true,
                ),
                WordCandidate::new(
                    "debuted",
                    "Прошедшая форма глагола «дебютировать», окончание -ed.",
                    true,
                ),
            ],
        ))
    }
}

fn run_understanding(app: App) -> App {
    let result = FakeUnderstanding
        .understand(&RawInputBatch::new(app.blob()), app.pair().support())
        .expect("fake understanding must succeed");
    app.confirmed_target(result.guess().code())
        .understood(result.candidates().to_vec())
}

#[test]
fn long_what_i_understood_list_scrolls_to_the_selected_candidate() {
    let mut app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(many_candidates(35));
    for _ in 0..29 {
        app = transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(6, 132);
    }
    let rendered = flat(&app);
    assert!(
        app.body_scroll() > 0 && rendered.contains("term-30") && !rendered.contains("term-01"),
        "long review lists must keep the selected candidate inside the visible scroll window: {rendered}"
    );
}

#[test]
fn what_i_understood_renders_understanding_rows_with_localized_prompts_and_card_count() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(
            FakeUnderstanding
                .understand(&RawInputBatch::new("whilst"), "ru")
                .expect("fake must succeed")
                .candidates()
                .to_vec(),
        );
    let rendered = flat(&app);
    assert!(
        rendered.contains("RU → EN")
            && rendered.contains("step 2/3")
            && rendered.contains("what i understood")
            && rendered.contains("quick check before i build the cards")
            && rendered.contains("sincerely")
            && rendered.contains("искренне")
            && rendered.contains("expel")
            && rendered.contains("at the end")
            && rendered.contains("[↑↓]")
            && rendered.contains("[Enter] refine")
            && rendered.contains("[Ctrl+G]")
            && rendered.contains("generate"),
        "sense check must render the new mono header, gloss list, and key hints: {rendered}"
    );
}

#[test]
fn what_i_understood_styles_selected_row_distinctly_from_others() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(
            FakeUnderstanding
                .understand(&RawInputBatch::new("sincerely"), "ru")
                .expect("fake must succeed")
                .candidates()
                .to_vec(),
        );
    assert!(
        modifiers(&app, "sincerely")
            .iter()
            .any(|modifier| modifier.contains(Modifier::BOLD)),
        "the selected term on the gloss list must render in bold"
    );
}

#[test]
fn excluded_candidate_renders_with_strikethrough_and_dim_gloss() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(vec![WordCandidate::new(
            "сообщение",
            "Слово на русском, не на target-языке — карточка не создаётся.",
            false,
        )]);
    let rendered = flat(&app);
    let term_modifiers = modifiers(&app, "сообщение");
    assert!(
        rendered.contains("не на target-языке")
            && term_modifiers
                .iter()
                .any(|modifier| modifier.contains(Modifier::CROSSED_OUT)),
        "excluded items must show their reason and render the term with a strikethrough: {rendered}"
    );
}

#[test]
fn drop_selected_removes_candidate_and_make_cards_advances_to_your_cards() {
    let start = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("whilst\nat the end\nin the end\nwreck");
    let (after_submit, side) = transit(start, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        side,
        Side::RunUnderstanding,
        "Generate on blob must request the understanding pass"
    );
    let reviewing = run_understanding(after_submit);
    let after_nav = transit(reviewing, to_app(press(KeyCode::Down)).expect("map")).0;
    let after_drop = transit(after_nav, to_app(press(KeyCode::Char('d'))).expect("map")).0;
    let (after_refine, refine_side) = transit(
        after_drop.clone(),
        to_app(press(KeyCode::Enter)).expect("map"),
    );
    let (after_make, make_side) = transit(
        after_drop,
        to_app(modified(KeyCode::Char('g'), KeyModifiers::CONTROL)).expect("map"),
    );
    let remaining: Vec<String> = after_make
        .candidates()
        .iter()
        .map(|candidate| String::from(candidate.term()))
        .collect();
    assert_eq!(
        (
            after_refine.modal(),
            refine_side,
            after_make.screen(),
            make_side,
            remaining,
        ),
        (
            Some(ModalKind::ChangeSomething),
            Side::None,
            Screen::YourCards,
            Side::StartGeneration,
            vec![
                String::from("sincerely"),
                String::from("at the end"),
                String::from("celebratory"),
                String::from("debuted"),
            ],
        ),
        "flow must drop the highlighted row, then Enter must refine and Ctrl+G must advance to Your Cards with StartGeneration"
    );
}

#[test]
fn empty_candidate_list_keeps_user_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        (next.screen(), side),
        (Screen::WhatIUnderstood, Side::None),
        "submitting with no candidates must keep the user on What I understood"
    );
}

#[test]
fn skipped_candidate_list_keeps_user_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(vec![WordCandidate::new(
            "окно",
            "Слово на русском, не на EN-target — карточка не создаётся.",
            false,
        )]);
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        (next.screen(), side),
        (Screen::WhatIUnderstood, Side::None),
        "only skipped candidates must not advance into card generation"
    );
}

#[test]
fn override_target_language_sticks_and_flips_target_pending_off() {
    let app = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::WhatIUnderstood);
    let next = transit(
        app,
        kamishibai::tui::AppEvent::OverrideTarget(String::from("es")),
    )
    .0;
    assert_eq!(
        (next.pair().target().to_string(), next.target_pending()),
        (String::from("es"), false),
        "OverrideTarget must change the target code and flip the pending flag off"
    );
}

#[test]
fn uppercase_t_on_what_i_understood_cycles_target_language() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let next = transit(app, to_app(press(KeyCode::Char('T'))).expect("map")).0;
    assert_eq!(
        (next.pair().target().to_string(), next.target_pending()),
        (String::from("zh"), false),
        "uppercase T on What I understood must cycle the target override"
    );
}
