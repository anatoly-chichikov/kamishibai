//! Integration flow for `Your words -> What I understood -> drop item -> make cards`.
//!
//! Uses the real input mapper, renderer, and transition function. The LLM
//! understanding pass is replaced with an inline `Understanding` fake.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::session::TargetGuess;
use kamishibai::session::{
    CandidateKind, CandidateMeta, LanguagePair, MetaSegment, RawInputBatch, Understanding,
    Understood, WordCandidate,
};
use kamishibai::tui::{App, Screen, Side, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
        Ok(Understood::new(
            TargetGuess::new("en", true),
            vec![
                candidate(
                    "sincerely",
                    "искренне",
                    CandidateMeta::new(
                        MetaSegment::dim("наречие"),
                        MetaSegment::dim("с искренними чувствами"),
                        Some(MetaSegment::bright("стиль: формальный")),
                    ),
                ),
                candidate(
                    "expel",
                    "исключать",
                    CandidateMeta::new(
                        MetaSegment::dim("глагол"),
                        MetaSegment::bright("из учебного заведения или организации"),
                        None,
                    ),
                ),
                WordCandidate::with_meta(
                    "at the end",
                    CandidateKind::Phrase,
                    "в конце",
                    String::new(),
                    CandidateMeta::new(
                        MetaSegment::dim("фраза"),
                        MetaSegment::dim("о времени или месте"),
                        None,
                    ),
                ),
                WordCandidate::with_meta(
                    "celebratory",
                    CandidateKind::Word,
                    "праздничный",
                    String::new(),
                    CandidateMeta::typo(
                        MetaSegment::dim("прилагательное"),
                        "исправлена опечатка: было \"celeblatory\"",
                    ),
                ),
                candidate(
                    "debuted",
                    "дебютировал",
                    CandidateMeta::new(
                        MetaSegment::bright("прошедшее время от слова \"debut\""),
                        MetaSegment::bright("о первом публичном появлении"),
                        None,
                    ),
                ),
            ],
        ))
    }
}

fn candidate(term: &str, preview: &str, meta: CandidateMeta) -> WordCandidate {
    WordCandidate::with_meta(term, CandidateKind::Word, preview, "", meta)
}

fn run_understanding(app: App) -> App {
    let result = FakeUnderstanding
        .understand(&RawInputBatch::new(app.blob()), app.pair().support())
        .expect("fake understanding must succeed");
    app.confirmed_target(result.guess().code())
        .understood(result.candidates().to_vec())
}

#[test]
fn what_i_understood_renders_sense_check_rows_with_localized_prompts_and_card_count() {
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
        rendered.contains("kamishibai · EN → RU")
            && rendered.contains("шаг 1 из 2 · проверка смысла")
            && rendered.contains("Я правильно понял эти слова?")
            && rendered.contains("поправь до того, как я сгенерирую карточки")
            && rendered.contains("sincerely")
            && rendered.contains("искренне")
            && rendered.contains("наречие · с искренними чувствами · стиль: формальный")
            && rendered.contains("expel")
            && rendered.contains("глагол · из учебного заведения или организации")
            && rendered.contains("at the end")
            && rendered.contains("фраза · о времени или месте")
            && rendered.contains("исправлена опечатка: было \"celeblatory\"")
            && rendered
                .contains("прошедшее время от слова \"debut\" · о первом публичном появлении")
            && rendered.contains("[↑↓] навигация")
            && rendered.contains("[T] сменить target · [Enter] сгенерировать 5 карточек"),
        "sense check must render localized header, dense one-line metadata, and exact generation count"
    );
}

#[test]
fn what_i_understood_styles_each_meta_segment_by_its_own_tone() {
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
        modifiers(&app, "наречие")
            .iter()
            .all(|modifier| !modifier.contains(Modifier::BOLD))
            && modifiers(&app, "стиль: формальный")
                .iter()
                .all(|modifier| modifier.contains(Modifier::BOLD)),
        "sense check metadata must style dim and bright labels independently"
    );
}

#[test]
fn drop_selected_removes_candidate_and_make_cards_advances_to_your_cards() {
    let start = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("whilst\nat the end\nin the end\nwreck");
    let (after_submit, side) = transit(start, kamishibai::tui::AppEvent::Submit);
    assert_eq!(
        side,
        Side::RunUnderstanding,
        "Submit on blob must request the understanding pass"
    );
    let reviewing = run_understanding(after_submit);
    let after_nav = transit(reviewing, to_app(press(KeyCode::Down)).expect("map")).0;
    let after_drop = transit(after_nav, to_app(press(KeyCode::Char('d'))).expect("map")).0;
    let (after_make, make_side) = transit(after_drop, to_app(press(KeyCode::Enter)).expect("map"));
    let remaining: Vec<String> = after_make
        .candidates()
        .iter()
        .map(|candidate| String::from(candidate.term()))
        .collect();
    assert_eq!(
        (after_make.screen(), make_side, remaining),
        (
            Screen::YourCards,
            Side::StartGeneration,
            vec![
                String::from("sincerely"),
                String::from("at the end"),
                String::from("celebratory"),
                String::from("debuted"),
            ],
        ),
        "flow must drop the highlighted row, then Enter must advance to Your Cards with StartGeneration"
    );
}

#[test]
fn empty_candidate_list_keeps_user_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Submit);
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
        .understood(vec![WordCandidate::with_meta(
            "окно",
            CandidateKind::Skipped,
            "not generated",
            "ru · outside the EN batch",
            CandidateMeta::new(
                MetaSegment::dim("пропущено"),
                MetaSegment::dim("другой язык исходного списка"),
                None,
            ),
        )]);
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Submit);
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
        (String::from("es"), false),
        "uppercase T on What I understood must cycle the target override"
    );
}
