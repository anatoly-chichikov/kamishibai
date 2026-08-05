//! Skeleton flow smoke test for the locked-in TUI state machine.
//!
//! Drives the pure transition function with fabricated events to cover the
//! path `YourWords -> WhatIUnderstood -> YourCards -> Done`. No UI rendering,
//! no network, no Gemini.

use kamishibai::session::{CardDraft, CardMeta, LanguagePair, WordCandidate};
use kamishibai::tui::{App, AppEvent, ModalKind, Screen, Side, transit};

fn fake_candidates() -> Vec<WordCandidate> {
    vec![WordCandidate::new(
        "a",
        "letter A; included by default",
        true,
    )]
}

#[test]
fn skeleton_flow_publishes_done_inline_on_your_cards() {
    let start = App::new(LanguagePair::new("en", "ru")).seeded_blob("a");
    let (after_words, understanding) = transit(start, AppEvent::Generate);
    let reviewing = after_words
        .clone()
        .with_screen(Screen::WhatIUnderstood)
        .understood(fake_candidates());
    let (after_understood, generation) = transit(reviewing, AppEvent::Generate);
    let (after_batch, publish) = transit(after_understood.clone(), AppEvent::BatchReady);
    assert_eq!(
        (
            after_words.screen(),
            understanding,
            after_understood.screen(),
            generation,
            after_batch.screen(),
            publish,
        ),
        (
            Screen::YourWords,
            Side::RunUnderstanding,
            Screen::YourCards,
            Side::StartGeneration,
            Screen::YourCards,
            Side::StartPublish,
        ),
        "skeleton flow must hand off to publish on YourCards instead of leaving the screen"
    );
}

#[test]
fn language_pair_travels_untouched_through_the_full_flow() {
    let start = App::new(LanguagePair::new("en", "ru")).seeded_blob("x");
    let (a, _) = transit(start, AppEvent::Generate);
    let reviewing = a
        .with_screen(Screen::WhatIUnderstood)
        .understood(fake_candidates());
    let (b, _) = transit(reviewing, AppEvent::Generate);
    let (c, _) = transit(b, AppEvent::BatchDone { failed: 0 });
    assert_eq!(
        c.pair().label(),
        "RU → EN",
        "the language pair must survive every transition without mutation"
    );
}

#[test]
fn add_more_modal_returns_bulk_correction_and_stays_open_while_running() {
    let start = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .understood(fake_candidates());
    let expanded = transit(start, AppEvent::KeyEnter).0;
    let add_more = transit(expanded, AppEvent::NavNext).0;
    let (opened, _) = transit(add_more, AppEvent::KeyChar(' '));
    let (running, side) = transit(
        opened.clone(),
        AppEvent::SendCorrection(String::from("#4 — глагол")),
    );
    assert_eq!(
        (opened.modal(), running.modal(), side),
        (
            Some(ModalKind::ChangeSomething),
            Some(ModalKind::ChangeSomething),
            Side::RunBulkCorrection(String::from("#4 — глагол")),
        ),
        "add more modal must open, request bulk correction, and stay visible while the request runs"
    );
}

#[test]
fn sentence_label_editor_queues_a_durable_card_rewrite() {
    let pair = LanguagePair::new("en", "ru");
    let start = App::new(pair.clone())
        .with_screen(Screen::YourCards)
        .cards_started(vec![CardDraft::new("a", "letter A", pair).with_meta(
            CardMeta::new(
                "/a/",
                "/a sentence/",
                "letter A",
                5,
                "source a",
                "a",
                "hint",
                "context",
                "Example with a.",
            ),
            None,
        )])
        .sentence_editor_opened_for_note();
    let typed = "другое значение"
        .chars()
        .fold(start.clone(), |app, symbol| {
            transit(app, AppEvent::KeyChar(symbol)).0
        });
    let (running, side) = transit(typed.clone(), AppEvent::Generate);
    assert_eq!(
        (
            start.sentence_editor().is_some(),
            typed.cards()[0]
                .rewrite()
                .map(kamishibai::session::CardRewrite::note),
            running.sentence_editor(),
            running.cards()[0]
                .rewrite()
                .map(kamishibai::session::CardRewrite::note),
            side,
        ),
        (
            true,
            Some("другое значение"),
            None,
            Some("другое значение"),
            Side::RegenerateCards,
        ),
        "the inline sentence editor failed to stage live input for bulk regeneration"
    );
}

#[test]
fn quit_from_done_requests_app_exit() {
    let start = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::Done);
    let (_, side) = transit(start, AppEvent::Quit);
    assert_eq!(
        side,
        Side::ExitApp,
        "Q on Done must ask the shell to exit the application"
    );
}
