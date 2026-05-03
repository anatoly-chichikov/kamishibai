//! Integration flow for `What I understood -> Change something -> updated What I understood`.

use anyhow::Result;
use kamishibai::session::{BulkCorrection, LanguagePair, WordCandidate};
use kamishibai::tui::{App, AppEvent, ModalKind, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct FakeBulk;

impl BulkCorrection for FakeBulk {
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        _comment: &str,
        _pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        candidates
            .iter()
            .map(|candidate| {
                Ok(WordCandidate::new(
                    candidate.term(),
                    "updated by bulk pass — verb sense selected",
                    candidate.ok(),
                ))
            })
            .collect()
    }
}

fn flat(app: &App) -> String {
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut flat = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            flat.push_str(buffer[(column, row)].symbol());
        }
        flat.push('\n');
    }
    flat
}

fn seeded() -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(vec![
            WordCandidate::new("whilst", "neutral conjunction; while", true),
            WordCandidate::new("wreck", "noun: remains of a destroyed ship", true),
        ])
}

#[test]
fn modal_renders_prompt_dashes_textarea_and_send_cancel_footer() {
    let app = seeded()
        .with_modal(ModalKind::ChangeSomething)
        .typed('#')
        .typed('2')
        .typed(' ')
        .typed('-')
        .typed(' ')
        .typed('v')
        .typed('e')
        .typed('r')
        .typed('b');
    let rendered = flat(&app);
    assert!(
        rendered.contains("change")
            && rendered.contains("tell me what to change")
            && rendered.contains("#2 - verb")
            && rendered.contains("[Esc] cancel")
            && rendered.contains("[Enter] send"),
        "Change something modal must render its prompt, the typed buffer, and send/cancel footer"
    );
}

#[test]
fn submit_on_modal_runs_bulk_correction_and_closes_modal() {
    let app = seeded()
        .with_modal(ModalKind::ChangeSomething)
        .typed('#')
        .typed('2')
        .typed(' ')
        .typed('v')
        .typed('e')
        .typed('r')
        .typed('b');
    let (after, side) = transit(app, AppEvent::Submit);
    let expected = Side::RunBulkCorrection(String::from("#2 verb"));
    assert_eq!(
        (after.modal(), side),
        (None, expected),
        "Enter inside Change something must emit RunBulkCorrection with the typed comment and close the modal"
    );
}

#[test]
fn empty_modal_submit_keeps_modal_open_and_emits_no_side_effect() {
    let app = seeded().with_modal(ModalKind::ChangeSomething);
    let (after, side) = transit(app, AppEvent::Submit);
    assert_eq!(
        (after.modal(), side),
        (Some(ModalKind::ChangeSomething), Side::None),
        "submitting an empty comment must keep the modal open and suppress the bulk pass"
    );
}

#[test]
fn escape_on_modal_dismisses_without_touching_candidates() {
    let app = seeded().with_modal(ModalKind::ChangeSomething).typed('x');
    let (after, side) = transit(app, AppEvent::Cancel);
    assert_eq!(
        (after.modal(), side, after.candidates().len(),),
        (None, Side::None, 2),
        "Esc must close the modal without running the bulk pass or touching the list"
    );
}

#[test]
fn bulk_pass_result_flows_back_into_what_i_understood() {
    let app = seeded();
    let updated = FakeBulk
        .correct_bulk(app.candidates(), "#2 verb", app.pair())
        .expect("mock bulk pass");
    let reviewed = app.understood(updated);
    let rendered = flat(&reviewed);
    assert!(
        rendered.contains("updated by bulk pass"),
        "after returning from Change something the review screen must show the patched understanding"
    );
}
