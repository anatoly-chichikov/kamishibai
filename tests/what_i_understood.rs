//! Integration flow for `Your words -> What I understood -> drop item -> make cards`.
//!
//! Uses the real input mapper, renderer, and transition function. The LLM
//! understanding pass is replaced with an inline `Understanding` fake.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::session::TargetGuess;
use kamishibai::session::{
    CandidateKind, LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate,
};
use kamishibai::tui::{App, Screen, Side, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn flat(app: &App) -> String {
    let backend = TestBackend::new(100, 16);
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

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
        Ok(Understood::new(
            TargetGuess::new("en", true),
            vec![
                WordCandidate::new(
                    "whilst",
                    CandidateKind::Other(String::from("formal conjunction")),
                    "«пока, в то время как» · BrE",
                    String::new(),
                ),
                WordCandidate::new(
                    "at the end",
                    CandidateKind::Phrase,
                    "«в конце» — о времени/месте",
                    String::new(),
                ),
                WordCandidate::new(
                    "in the end",
                    CandidateKind::Idiom,
                    "«в итоге» — о результате",
                    String::new(),
                ),
                WordCandidate::new(
                    "wreck",
                    CandidateKind::Other(String::from("noun / verb")),
                    "обломки · разрушать",
                    String::new(),
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
fn what_i_understood_renders_four_candidates_with_prompts_and_language_pair() {
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
        rendered.contains("What I understood")
            && rendered.contains("a quick look before making the cards")
            && rendered.contains("whilst")
            && rendered.contains("at the end")
            && rendered.contains("in the end")
            && rendered.contains("wreck")
            && rendered.contains("looks right?")
            && rendered.contains("[Enter] make cards")
            && rendered.contains("[R] change something")
            && rendered.contains("kamishibai ·")
            && rendered.contains("EN → RU"),
        "What I understood must render headline, tagline, every candidate, both prompts, and language pair"
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
                String::from("whilst"),
                String::from("in the end"),
                String::from("wreck"),
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
