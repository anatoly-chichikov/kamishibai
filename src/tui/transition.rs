use super::app::App;
use super::event::AppEvent;
use super::screen::{ModalKind, Screen};

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
    ExitApp,
}

/// Pure transition function: given the current app and one event, produce the
/// next app plus an optional side effect. No IO, no Gemini calls.
pub fn transit(app: App, event: AppEvent) -> (App, Side) {
    let event = promote(&app, event);
    match (app.screen(), app.modal(), event) {
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
        (Screen::YourWords, None, AppEvent::KeyChar(symbol)) => (app.typed(symbol), Side::None),
        (Screen::YourWords, None, AppEvent::KeyBackspace) => (app.rubbed(), Side::None),
        (Screen::YourWords, None, AppEvent::ToggleMyLanguage) => {
            let next = app.toggle_support();
            let code = next.pair().support().to_string();
            (next, Side::PersistMyLanguage(code))
        }
        (Screen::WhatIUnderstood, None, AppEvent::Submit) => {
            if app.candidates().is_empty() {
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
        (
            Screen::WhatIUnderstood,
            Some(ModalKind::ChangeSomething),
            AppEvent::SendCorrection(text),
        ) => (app.close_modal(), Side::RunBulkCorrection(text)),
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::Submit) => {
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
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::SendCorrection(text)) => {
            (app.close_modal(), Side::RunCardCorrection(text))
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::Submit) => {
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
        (Screen::YourCards, None, AppEvent::BatchReady) => {
            (app.with_screen(Screen::Done), Side::None)
        }
        (Screen::YourCards, None, AppEvent::BatchDone { failed: _ }) => {
            (app.with_screen(Screen::Done), Side::None)
        }
        (Screen::Done, None, AppEvent::NewBatch) => (app.fresh_batch(), Side::None),
        (Screen::Done, None, AppEvent::Quit) => (app, Side::ExitApp),
        (_, _, AppEvent::Redraw) => (app, Side::None),
        (_, _, _) => (app, Side::None),
    }
}

fn promote(app: &App, event: AppEvent) -> AppEvent {
    if app.modal().is_some() {
        return event;
    }
    match (app.screen(), &event) {
        (Screen::WhatIUnderstood, AppEvent::KeyChar('r'))
        | (Screen::WhatIUnderstood, AppEvent::KeyChar('R'))
        | (Screen::YourCards, AppEvent::KeyChar('r'))
        | (Screen::YourCards, AppEvent::KeyChar('R')) => AppEvent::RequestChange,
        (Screen::Done, AppEvent::KeyChar('n')) | (Screen::Done, AppEvent::KeyChar('N')) => {
            AppEvent::NewBatch
        }
        (Screen::Done, AppEvent::KeyChar('q')) | (Screen::Done, AppEvent::KeyChar('Q')) => {
            AppEvent::Quit
        }
        _ => event,
    }
}
