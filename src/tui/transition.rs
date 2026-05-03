use super::app::App;
use super::event::AppEvent;
use super::screen::{ModalKind, Screen, WelcomeStage};

/// A side effect requested by a transition. The shell interprets it outside the pure function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Side {
    None,
    RunUnderstanding,
    RunBulkCorrection(String),
    RunCardCorrection(String),
    StartGeneration,
    RegenerateFailed,
    PersistMyLanguage(String),
    PersistApiKey(String),
    OpenKeyHelp,
    PublishDone,
    ExitApp,
}

/// Pure transition function: given the current app and one event, produce the
/// next app plus an optional side effect. No IO, no Gemini calls.
pub fn transit(app: App, event: AppEvent) -> (App, Side) {
    if app.error().is_some() && event != AppEvent::Redraw {
        return (app.error_cleared(), Side::None);
    }
    if app.busy().is_some() && event != AppEvent::Redraw {
        return (app, Side::None);
    }
    let event = promote(&app, event);
    match (app.screen(), app.modal(), event) {
        (Screen::Welcome, _, e) => welcome(app, e),
        (Screen::YourWords, None, AppEvent::Submit) => {
            if app
                .blob()
                .chars()
                .any(|character| !character.is_whitespace())
            {
                (
                    app.with_screen(Screen::WhatIUnderstood),
                    Side::RunUnderstanding,
                )
            } else {
                (app, Side::None)
            }
        }
        (Screen::YourWords, None, AppEvent::KeyEnter) => (app.typed('\n'), Side::None),
        (Screen::YourWords, None, AppEvent::KeyChar(symbol)) => (app.typed(symbol), Side::None),
        (Screen::YourWords, None, AppEvent::KeyBackspace) => (app.rubbed(), Side::None),
        (Screen::YourWords, None, AppEvent::ToggleMyLanguage) => {
            let next = app.toggle_support();
            let code = next.pair().support().to_string();
            (next, Side::PersistMyLanguage(code))
        }
        (Screen::WhatIUnderstood, None, AppEvent::Submit)
        | (Screen::WhatIUnderstood, None, AppEvent::KeyEnter) => {
            if !app.candidates().iter().any(|candidate| candidate.ok()) {
                (app, Side::None)
            } else {
                (app.with_screen(Screen::YourCards), Side::StartGeneration)
            }
        }
        (Screen::WhatIUnderstood, None, AppEvent::RequestChange) => {
            (app.with_modal(ModalKind::ChangeSomething), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('d'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('D')) => {
            (app.dropped_selected(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('k'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('K')) => {
            (app.selected_previous(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('j'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('J')) => {
            (app.selected_next(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavPrev) => (app.selected_previous(), Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::NavNext) => (app.selected_next(), Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::OverrideTarget(code)) => {
            (app.override_target(code), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::ToggleMyLanguage) => {
            let next = app.toggle_support();
            let code = next.pair().support().to_string();
            (next, Side::PersistMyLanguage(code))
        }
        (
            Screen::WhatIUnderstood,
            Some(ModalKind::ChangeSomething),
            AppEvent::SendCorrection(text),
        ) => (app.close_modal(), Side::RunBulkCorrection(text)),
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::Submit)
        | (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::KeyEnter) => {
            let text = app.modal_buffer().to_string();
            if text.chars().any(|c| !c.is_whitespace()) {
                (app.close_modal(), Side::RunBulkCorrection(text))
            } else {
                (app, Side::None)
            }
        }
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::Cancel) => {
            (app.close_modal(), Side::None)
        }
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::KeyChar(symbol)) => {
            (app.typed(symbol), Side::None)
        }
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::KeyBackspace) => {
            (app.rubbed(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::RequestChange) => {
            (app.with_modal(ModalKind::ChangeThisCard), Side::None)
        }
        (Screen::YourCards, None, AppEvent::NavPrev) => (app.card_selected_previous(), Side::None),
        (Screen::YourCards, None, AppEvent::NavNext) => (app.card_selected_next(), Side::None),
        (Screen::YourCards, None, AppEvent::Submit)
        | (Screen::YourCards, None, AppEvent::KeyEnter) => (app.card_toggle_expanded(), Side::None),
        (Screen::YourCards, None, AppEvent::KeyChar('d'))
        | (Screen::YourCards, None, AppEvent::KeyChar('D')) => {
            (app.card_dropped_artifact(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::KeyChar('r')) if app.cards_failed() > 0 => {
            (app, Side::RegenerateFailed)
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::SendCorrection(text)) => {
            (app.close_modal(), Side::RunCardCorrection(text))
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::Submit)
        | (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::KeyEnter) => {
            let text = app.modal_buffer().to_string();
            if text.chars().any(|c| !c.is_whitespace()) {
                (app.close_modal(), Side::RunCardCorrection(text))
            } else {
                (app, Side::None)
            }
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::Cancel) => {
            (app.close_modal(), Side::None)
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::KeyChar(symbol)) => {
            (app.typed(symbol), Side::None)
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::KeyBackspace) => {
            (app.rubbed(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::BatchReady) => (app, Side::PublishDone),
        (Screen::YourCards, None, AppEvent::BatchDone { failed: _ }) => (app, Side::PublishDone),
        (Screen::YourCards, None, AppEvent::NewBatch) => (app.fresh_batch(), Side::None),
        (Screen::Done, None, AppEvent::NewBatch) => (app.fresh_batch(), Side::None),
        (Screen::Done, None, AppEvent::Quit) => (app, Side::ExitApp),
        (_, _, AppEvent::Redraw) => (app, Side::None),
        (_, _, _) => (app, Side::None),
    }
}

fn welcome(app: App, event: AppEvent) -> (App, Side) {
    let stage = app.welcome().stage;
    match (stage, event) {
        (WelcomeStage::PickLanguage, AppEvent::WelcomeNextLanguage) => {
            let next = next_support(app.pair().support(), 1);
            let next_app = app.welcome_pick_language(next.clone());
            (next_app, Side::PersistMyLanguage(next))
        }
        (WelcomeStage::PickLanguage, AppEvent::WelcomePrevLanguage) => {
            let next = next_support(app.pair().support(), -1);
            let next_app = app.welcome_pick_language(next.clone());
            (next_app, Side::PersistMyLanguage(next))
        }
        (WelcomeStage::PickLanguage, AppEvent::Submit)
        | (WelcomeStage::PickLanguage, AppEvent::KeyEnter) => (app.welcome_advance(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::Cancel) => (app.welcome_step_back(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::WelcomePasteKey(text)) => {
            let trimmed = text.trim().to_string();
            let next = app.welcome_paste_key(trimmed.clone());
            (next, Side::PersistApiKey(trimmed))
        }
        (WelcomeStage::EnterKey, AppEvent::KeyChar(symbol)) => {
            let mut key = app.welcome().key.clone();
            key.push(symbol);
            (app.welcome_paste_key(key), Side::None)
        }
        (WelcomeStage::EnterKey, AppEvent::KeyBackspace) => {
            (app.welcome_clear_key(), Side::PersistApiKey(String::new()))
        }
        (WelcomeStage::EnterKey, AppEvent::WelcomeOpenKeyHelp) => (app, Side::OpenKeyHelp),
        (WelcomeStage::EnterKey, AppEvent::Submit)
        | (WelcomeStage::EnterKey, AppEvent::KeyEnter) => {
            if app.welcome().key.chars().count() >= 20 {
                (app.with_screen(Screen::YourWords), Side::None)
            } else {
                (app, Side::None)
            }
        }
        _ => (app, Side::None),
    }
}

fn promote(app: &App, event: AppEvent) -> AppEvent {
    if app.modal().is_some() {
        return event;
    }
    match (app.screen(), &event) {
        (Screen::Welcome, AppEvent::NavPrev) => AppEvent::WelcomePrevLanguage,
        (Screen::Welcome, AppEvent::NavNext) => AppEvent::WelcomeNextLanguage,
        (Screen::Welcome, AppEvent::KeyChar('?')) => AppEvent::WelcomeOpenKeyHelp,
        (Screen::WhatIUnderstood, AppEvent::KeyChar('r'))
        | (Screen::WhatIUnderstood, AppEvent::KeyChar('R')) => AppEvent::RequestChange,
        (Screen::WhatIUnderstood, AppEvent::KeyChar('t'))
        | (Screen::WhatIUnderstood, AppEvent::KeyChar('T')) => {
            AppEvent::OverrideTarget(next_target(app.pair().target(), app.pair().support()))
        }
        (Screen::YourCards, AppEvent::KeyChar('R')) => AppEvent::RequestChange,
        (Screen::YourCards, AppEvent::KeyChar('r')) => {
            if app.cards_failed() > 0 {
                event
            } else {
                AppEvent::RequestChange
            }
        }
        (Screen::YourCards, AppEvent::KeyChar('n'))
        | (Screen::YourCards, AppEvent::KeyChar('N')) => {
            if all_finished(app) {
                AppEvent::NewBatch
            } else {
                event
            }
        }
        (Screen::WhatIUnderstood, AppEvent::KeyChar('l'))
        | (Screen::WhatIUnderstood, AppEvent::KeyChar('L')) => AppEvent::ToggleMyLanguage,
        (Screen::Done, AppEvent::KeyChar('n')) | (Screen::Done, AppEvent::KeyChar('N')) => {
            AppEvent::NewBatch
        }
        _ => event,
    }
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}

fn next_target(current: &str, support: &str) -> String {
    let order = ["en", "ru", "es", "de", "el", "zh"];
    let mut position = 0;
    for (index, code) in order.iter().enumerate() {
        if *code == current {
            position = index;
            break;
        }
    }
    for offset in 1..=order.len() {
        let candidate = order[(position + offset) % order.len()];
        if candidate != support {
            return String::from(candidate);
        }
    }
    String::from(current)
}

fn next_support(current: &str, direction: i32) -> String {
    let order = ["en", "ru", "es", "de", "el", "zh", "fr", "it", "ja"];
    let mut position: i32 = 0;
    for (index, code) in order.iter().enumerate() {
        if *code == current {
            position = index as i32;
            break;
        }
    }
    let next = (position + direction).rem_euclid(order.len() as i32) as usize;
    String::from(order[next])
}
