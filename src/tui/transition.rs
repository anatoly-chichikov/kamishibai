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
    match (app.screen(), app.modal(), event) {
        (Screen::YourWords, None, AppEvent::Submit) => (
            app.with_screen(Screen::WhatIUnderstood),
            Side::RunUnderstanding,
        ),
        (Screen::YourWords, None, AppEvent::ToggleMyLanguage) => {
            let next = app.toggle_support();
            let code = next.pair().support().to_string();
            (next, Side::PersistMyLanguage(code))
        }
        (Screen::WhatIUnderstood, None, AppEvent::Submit) => {
            (app.with_screen(Screen::YourCards), Side::StartGeneration)
        }
        (Screen::WhatIUnderstood, None, AppEvent::RequestChange) => {
            (app.with_modal(ModalKind::ChangeSomething), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::OverrideTarget(code)) => {
            (app.override_target(code), Side::None)
        }
        (
            Screen::WhatIUnderstood,
            Some(ModalKind::ChangeSomething),
            AppEvent::SendCorrection(text),
        ) => (app.close_modal(), Side::RunBulkCorrection(text)),
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::Cancel) => {
            (app.close_modal(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::RequestChange) => {
            (app.with_modal(ModalKind::ChangeThisCard), Side::None)
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::SendCorrection(text)) => {
            (app.close_modal(), Side::RunCardCorrection(text))
        }
        (Screen::YourCards, Some(ModalKind::ChangeThisCard), AppEvent::Cancel) => {
            (app.close_modal(), Side::None)
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
