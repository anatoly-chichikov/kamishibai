//! Skeleton flow smoke test for the locked-in TUI state machine.
//!
//! Drives the pure transition function with fabricated events to cover the
//! path `YourWords -> WhatIUnderstood -> YourCards -> Done`. No UI rendering,
//! no network, no Gemini.

use kamishibai::session::{CandidateKind, LanguagePair, WordCandidate};
use kamishibai::tui::{App, AppEvent, ModalKind, Screen, Side, transit};

fn fake_candidates() -> Vec<WordCandidate> {
    vec![WordCandidate::new("a", CandidateKind::Word, "sample", "")]
}

#[test]
fn skeleton_flow_reaches_done_through_every_fullscreen_screen() {
    let start = App::new(LanguagePair::new("en", "ru")).seeded_blob("a");
    let (after_words, understanding) = transit(start, AppEvent::Submit);
    let reviewing = after_words.clone().understood(fake_candidates());
    let (after_understood, generation) = transit(reviewing, AppEvent::Submit);
    let (after_cards, _) = transit(after_understood.clone(), AppEvent::BatchReady);
    assert_eq!(
        (
            after_words.screen(),
            understanding,
            after_understood.screen(),
            generation,
            after_cards.screen(),
        ),
        (
            Screen::WhatIUnderstood,
            Side::RunUnderstanding,
            Screen::YourCards,
            Side::StartGeneration,
            Screen::Done,
        ),
        "skeleton flow must traverse YourWords -> WhatIUnderstood -> YourCards -> Done"
    );
}

#[test]
fn language_pair_travels_untouched_through_the_full_flow() {
    let start = App::new(LanguagePair::new("en", "ru")).seeded_blob("x");
    let (a, _) = transit(start, AppEvent::Submit);
    let reviewing = a.understood(fake_candidates());
    let (b, _) = transit(reviewing, AppEvent::Submit);
    let (c, _) = transit(b, AppEvent::BatchDone { failed: 0 });
    assert_eq!(
        c.pair().label(),
        "EN → RU",
        "the language pair must survive every transition without mutation"
    );
}

#[test]
fn change_something_modal_returns_bulk_correction_and_closes_modal() {
    let start = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::WhatIUnderstood);
    let (opened, _) = transit(start, AppEvent::RequestChange);
    let (closed, side) = transit(
        opened.clone(),
        AppEvent::SendCorrection(String::from("#4 — глагол")),
    );
    assert_eq!(
        (opened.modal(), closed.modal(), side),
        (
            Some(ModalKind::ChangeSomething),
            None,
            Side::RunBulkCorrection(String::from("#4 — глагол")),
        ),
        "Change something modal must open, request bulk correction, and close cleanly"
    );
}

#[test]
fn change_this_card_modal_returns_card_correction_and_closes_modal() {
    let start = App::new(LanguagePair::new("en", "ru")).with_screen(Screen::YourCards);
    let (opened, _) = transit(start, AppEvent::RequestChange);
    let (closed, side) = transit(
        opened.clone(),
        AppEvent::SendCorrection(String::from("другое значение")),
    );
    assert_eq!(
        (opened.modal(), closed.modal(), side),
        (
            Some(ModalKind::ChangeThisCard),
            None,
            Side::RunCardCorrection(String::from("другое значение")),
        ),
        "Change this card modal must open, request per-card correction, and close cleanly"
    );
}

#[test]
fn new_batch_from_done_resets_to_your_words_without_losing_language() {
    let start = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::Done)
        .with_failed(2);
    let (next, _) = transit(start, AppEvent::NewBatch);
    assert_eq!(
        (next.screen(), next.failed(), next.pair().label()),
        (Screen::YourWords, 0, String::from("EN → RU")),
        "new batch must restart at YourWords, clear failures, and keep the language pair"
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
