use crate::languages::catalog;

use super::app::App;
use super::disclosure::{DisclosureControls, DisclosureIntent};
use super::event::AppEvent;
use super::screen::{ModalKind, Screen, WelcomeFocus, WelcomeStage};
use super::sentence_editor::LabelEditorRow;

/// A side effect requested by a transition. The shell interprets it outside the pure function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Side {
    None,
    RunUnderstanding,
    RunBulkCorrection(String),
    PersistMyLanguageAndRunUnderstanding(String),
    StartGeneration,
    RegenerateFailed,
    RegenerateCards,
    PersistMyLanguage(String),
    /// Welcome key step: probe Gemini with the entered key. The shell runs the
    /// check off-thread, persists language + key only on success, then moves to
    /// `Your Words`; a rejected key stays on Welcome with an inline notice.
    ValidateKey(String),
    LoadEnvKey,
    /// Engine drained — kick off the (asynchronous) publish phase. The shell
    /// spawns a background thread that builds the .apkg and the .pdf, surfacing
    /// progress through the universal busy loader (`PublishingDeck` →
    /// `PublishingReport`).
    StartPublish,
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
        (Screen::YourWords, None, AppEvent::Generate) => {
            if app
                .blob()
                .chars()
                .any(|character| !character.is_whitespace())
            {
                (app, Side::RunUnderstanding)
            } else {
                (app, Side::None)
            }
        }
        (Screen::YourWords, None, AppEvent::KeyEnter) => (app.typed('\n'), Side::None),
        (Screen::YourWords, None, AppEvent::KeyChar(symbol)) => (app.typed(symbol), Side::None),
        (Screen::YourWords, None, AppEvent::KeyBackspace) => (app.rubbed(), Side::None),
        (Screen::YourWords, None, AppEvent::CursorLeft) => (app.cursor_left(), Side::None),
        (Screen::YourWords, None, AppEvent::CursorRight) => (app.cursor_right(), Side::None),
        (Screen::YourWords, None, AppEvent::NavPrev) => (app.cursor_up(), Side::None),
        (Screen::YourWords, None, AppEvent::NavNext) => (app.cursor_down(), Side::None),
        (Screen::YourWords, None, AppEvent::OpenLanguagePicker) => {
            (open_language_picker(app), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Generate) => start_generation(app),
        (Screen::WhatIUnderstood, None, AppEvent::Cancel)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::SentenceSettingsOpen)
            if app.expanded_sense().is_none() && !app.candidates().is_empty() =>
        {
            (app.sentence_settings_opened(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::SentenceSettingsFocus(row))
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_focused(row), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::SentenceSettingsChoose(row, index))
            if app.sentence_settings_editor().is_some() =>
        {
            (
                app.sentence_settings_focused(row)
                    .sentence_settings_chosen(index),
                Side::None,
            )
        }
        (Screen::WhatIUnderstood, None, AppEvent::SentenceSettingsAdvance(row, forward))
            if app.sentence_settings_editor().is_some() =>
        {
            (
                app.sentence_settings_focused(row)
                    .sentence_settings_advanced(forward),
                Side::None,
            )
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavPrev)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_row_previous(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavNext)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_row_next(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorLeft)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_advanced(false), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorRight)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_advanced(true), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Submit | AppEvent::KeyEnter)
            if app.sentence_settings_editor().is_some() =>
        {
            (app, Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar(_))
            if app.sentence_settings_editor().is_some() =>
        {
            (app, Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('s' | 'S'))
            if app.expanded_sense().is_none() && !app.candidates().is_empty() =>
        {
            (app.sentence_settings_opened(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Cancel) if app.expanded_sense().is_some() => {
            (app.senses_cancelled(), Side::None)
        }
        (Screen::WhatIUnderstood, None, event)
            if matches!(sense_controls(&app).intent(&event), DisclosureIntent::Close) =>
        {
            (app.senses_confirmed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, event)
            if sense_controls(&app).intent(&event) == DisclosureIntent::Action =>
        {
            if app.expanded_add_more_focused() {
                (app.with_modal(ModalKind::ChangeSomething), Side::None)
            } else {
                (app.sense_toggled(), Side::None)
            }
        }
        (Screen::WhatIUnderstood, None, event)
            if sense_controls(&app).intent(&event) == DisclosureIntent::Open =>
        {
            if app.selected_can_expand_senses() {
                (app.senses_expanded(), Side::None)
            } else {
                (app, Side::None)
            }
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorLeft) => (app, Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::CursorRight) => (app, Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::NavPrev) if app.expanded_sense().is_some() => {
            (app.sense_previous(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavNext) if app.expanded_sense().is_some() => {
            (app.sense_next(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('k'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('K'))
            if app.expanded_sense().is_some() =>
        {
            (app.sense_previous(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('j'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('J'))
            if app.expanded_sense().is_some() =>
        {
            (app.sense_next(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('d'))
        | (Screen::WhatIUnderstood, None, AppEvent::KeyChar('D')) => {
            let next = app.dropped_selected();
            if next.candidates().is_empty() {
                (
                    next.with_screen(Screen::YourWords)
                        .clear_blob()
                        .body_scroll_reset(),
                    Side::None,
                )
            } else {
                (next, Side::None)
            }
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
        (Screen::WhatIUnderstood, None, AppEvent::OpenLanguagePicker) => {
            (open_language_picker(app), Side::None)
        }
        (
            Screen::WhatIUnderstood,
            Some(ModalKind::ChangeSomething),
            AppEvent::SendCorrection(text),
        ) => (app, Side::RunBulkCorrection(text)),
        (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::Submit)
        | (Screen::WhatIUnderstood, Some(ModalKind::ChangeSomething), AppEvent::KeyEnter) => {
            let text = app.modal_buffer().to_string();
            if text.chars().any(|c| !c.is_whitespace()) {
                (app, Side::RunBulkCorrection(text))
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
        (_, Some(ModalKind::PickMyLanguage), AppEvent::LanguagePickerPrev) => {
            (app.picker_cursor_advanced(-1), Side::None)
        }
        (_, Some(ModalKind::PickMyLanguage), AppEvent::LanguagePickerNext) => {
            (app.picker_cursor_advanced(1), Side::None)
        }
        (_, Some(ModalKind::PickMyLanguage), AppEvent::SetMyLanguage(code))
            if can_pick_language(app.screen()) =>
        {
            let screen = app.screen();
            let next = app.close_modal().set_known(code.clone());
            if screen == Screen::WhatIUnderstood && !next.candidates().is_empty() {
                (next, Side::PersistMyLanguageAndRunUnderstanding(code))
            } else {
                (next, Side::PersistMyLanguage(code))
            }
        }
        (_, Some(ModalKind::PickMyLanguage), AppEvent::Cancel) => (app.close_modal(), Side::None),
        (_, Some(ModalKind::PickMyLanguage), _) => (app, Side::None),
        (Screen::YourCards, None, AppEvent::Cancel) if app.sentence_editor().is_some() => {
            (app.sentence_editor_closed(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelOpen(card, row))
            if app.card_tunable_at(card) =>
        {
            (
                app.card_revealed(card).sentence_editor_focused(row),
                Side::None,
            )
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelFocus(row)) if app.card_tunable() => {
            let next = if app.sentence_editor().is_some() {
                app.sentence_editor_focused(row)
            } else {
                app.sentence_editor_opened_for_register()
                    .sentence_editor_focused(row)
            };
            (next, Side::None)
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelChoose(row, index))
            if app.sentence_editor().is_some() =>
        {
            (
                app.sentence_editor_focused(row)
                    .sentence_editor_axis_chosen(index),
                Side::None,
            )
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelAdvance(row, forward))
            if app.sentence_editor().is_some() =>
        {
            (
                app.sentence_editor_focused(row)
                    .sentence_editor_axis_advanced(forward),
                Side::None,
            )
        }
        (Screen::YourCards, None, AppEvent::NavPrev) if app.sentence_editor().is_some() => {
            (app.sentence_editor_row_previous(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::NavNext) if app.sentence_editor().is_some() => {
            (app.sentence_editor_row_next(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::CursorLeft) if app.sentence_editor().is_some() => {
            let next = if sentence_note_focused(&app) {
                app.sentence_editor_cursor_left()
            } else {
                app.sentence_editor_axis_advanced(false)
            };
            (next, Side::None)
        }
        (Screen::YourCards, None, AppEvent::CursorRight) if app.sentence_editor().is_some() => {
            let next = if sentence_note_focused(&app) {
                app.sentence_editor_cursor_right()
            } else {
                app.sentence_editor_axis_advanced(true)
            };
            (next, Side::None)
        }
        (Screen::YourCards, None, AppEvent::KeyChar(symbol)) if app.sentence_editor().is_some() => {
            (app.sentence_editor_typed(symbol), Side::None)
        }
        (Screen::YourCards, None, AppEvent::KeyBackspace) if app.sentence_editor().is_some() => {
            (app.sentence_editor_rubbed(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::Submit | AppEvent::KeyEnter)
            if app.sentence_editor().is_some() =>
        {
            (app, Side::None)
        }
        (Screen::YourCards, None, AppEvent::Generate) if app.sentence_editor().is_some() => {
            (app.sentence_editor_closed(), Side::RegenerateCards)
        }
        (Screen::YourCards, None, AppEvent::KeyChar(' ')) if app.card_tunable() => {
            (app.sentence_editor_opened_for_register(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::NavPrev) => (app.card_selected_previous(), Side::None),
        (Screen::YourCards, None, AppEvent::NavNext) => (app.card_selected_next(), Side::None),
        (Screen::YourCards, None, event)
            if matches!(
                DisclosureControls::new(app.card_expanded()).intent(&event),
                DisclosureIntent::Open | DisclosureIntent::Close
            ) =>
        {
            (app.card_toggle_expanded(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::Generate) if !app.cards().is_empty() => {
            (app, Side::RegenerateCards)
        }
        (Screen::YourCards, None, AppEvent::BatchReady) => (app, Side::StartPublish),
        (Screen::YourCards, None, AppEvent::BatchDone { failed: _ }) => (app, Side::StartPublish),
        (Screen::Done, None, AppEvent::Generate) if app.cards_failed() > 0 => {
            (app.with_screen(Screen::YourCards), Side::RegenerateFailed)
        }
        (Screen::Done, None, AppEvent::Quit) => (app, Side::ExitApp),
        (_, _, AppEvent::Redraw) => (app, Side::None),
        (_, _, _) => (app, Side::None),
    }
}

fn sense_controls(app: &App) -> DisclosureControls {
    let controls = DisclosureControls::new(app.expanded_sense().is_some());
    if app.expanded_sense().is_some() {
        controls.with_action("select")
    } else {
        controls
    }
}

fn sentence_note_focused(app: &App) -> bool {
    app.sentence_editor()
        .is_some_and(|editor| editor.row() == LabelEditorRow::Note)
}

fn start_generation(app: App) -> (App, Side) {
    let app = app.senses_confirmed();
    if !app.candidates().iter().any(|candidate| candidate.ok()) {
        return (app, Side::None);
    }
    (app.with_screen(Screen::YourCards), Side::StartGeneration)
}

fn welcome(app: App, event: AppEvent) -> (App, Side) {
    let stage = app.welcome().stage;
    match (stage, event) {
        (WelcomeStage::PickLanguage, AppEvent::WelcomeNextLanguage) => {
            let next = next_known(app.pair().known(), 1);
            (app.set_known(next), Side::None)
        }
        (WelcomeStage::PickLanguage, AppEvent::WelcomePrevLanguage) => {
            let next = next_known(app.pair().known(), -1);
            (app.set_known(next), Side::None)
        }
        (WelcomeStage::PickLanguage, AppEvent::Submit)
        | (WelcomeStage::PickLanguage, AppEvent::KeyEnter) => {
            let language = app.pair().known().to_string();
            (app.welcome_advance(), Side::PersistMyLanguage(language))
        }
        (WelcomeStage::EnterKey, AppEvent::Cancel) => (app.welcome_step_back(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::CursorLeft) => (app.welcome_focus_prev(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::CursorRight) => (app.welcome_focus_next(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::WelcomeFocusTo(focus)) => {
            (app.welcome_focus(focus), Side::None)
        }
        (WelcomeStage::EnterKey, AppEvent::WelcomePasteKey(text)) => {
            (app.welcome_paste_key(text.trim().to_string()), Side::None)
        }
        (WelcomeStage::EnterKey, AppEvent::KeyChar(symbol)) => {
            let mut key = app.welcome().key.clone();
            key.push(symbol);
            (app.welcome_paste_key(key), Side::None)
        }
        (WelcomeStage::EnterKey, AppEvent::KeyBackspace) => (app.welcome_clear_key(), Side::None),
        (WelcomeStage::EnterKey, AppEvent::WelcomeLoadEnvKey) => (app, Side::LoadEnvKey),
        (WelcomeStage::EnterKey, AppEvent::Submit)
        | (WelcomeStage::EnterKey, AppEvent::KeyEnter) => welcome_submit(app),
        _ => (app, Side::None),
    }
}

/// Activate the focused control on the key step. Focus on `load from env`
/// pulls the key from the environment; otherwise an empty buffer just nudges
/// the user and a filled one is sent off for an API validity check.
fn welcome_submit(app: App) -> (App, Side) {
    if app.welcome().focus == WelcomeFocus::LoadEnv {
        return (app, Side::LoadEnvKey);
    }
    let key = app.welcome().key.trim().to_string();
    if key.is_empty() {
        return (app.welcome_notice("enter a key first"), Side::None);
    }
    (app, Side::ValidateKey(key))
}

fn promote(app: &App, event: AppEvent) -> AppEvent {
    if let Some(ModalKind::PickMyLanguage) = app.modal() {
        return promote_picker(app, event);
    }
    if app.modal().is_some() {
        return event;
    }
    match (app.screen(), &event) {
        (Screen::Welcome, AppEvent::NavPrev)
            if app.welcome().stage == WelcomeStage::PickLanguage =>
        {
            AppEvent::WelcomePrevLanguage
        }
        (Screen::Welcome, AppEvent::NavNext)
            if app.welcome().stage == WelcomeStage::PickLanguage =>
        {
            AppEvent::WelcomeNextLanguage
        }
        (Screen::Welcome, AppEvent::CursorLeft)
            if app.welcome().stage == WelcomeStage::PickLanguage =>
        {
            AppEvent::WelcomePrevLanguage
        }
        (Screen::Welcome, AppEvent::CursorRight)
            if app.welcome().stage == WelcomeStage::PickLanguage =>
        {
            AppEvent::WelcomeNextLanguage
        }
        (Screen::Welcome, AppEvent::OpenLanguagePicker)
            if app.welcome().stage == WelcomeStage::PickLanguage =>
        {
            AppEvent::WelcomeNextLanguage
        }
        _ => event,
    }
}

/// Reinterpret keys while the language picker modal is open: arrows cycle the
/// chip selection, Enter confirms it, plain Esc still cancels through the
/// generic modal dismissal arm.
fn promote_picker(app: &App, event: AppEvent) -> AppEvent {
    match event {
        AppEvent::NavPrev | AppEvent::CursorLeft => AppEvent::LanguagePickerPrev,
        AppEvent::NavNext | AppEvent::CursorRight => AppEvent::LanguagePickerNext,
        AppEvent::Submit | AppEvent::KeyEnter => {
            AppEvent::SetMyLanguage(picker_selected(app).to_string())
        }
        other => other,
    }
}

/// Return the language code currently highlighted in the picker modal.
///
/// Selection is stored on the app as the modal's persistent index. If the
/// app has no recorded picker index yet, fall back to the active support
/// language so the modal opens with the current pick highlighted.
pub fn picker_selected(app: &App) -> &'static str {
    let codes = catalog().codes();
    let cursor = app.picker_cursor();
    codes[cursor.min(codes.len() - 1)]
}

/// Compute the picker cursor for a given language code. Used when opening the
/// modal so the active language is pre-selected.
pub fn picker_cursor_for(code: &str) -> usize {
    let codes = catalog().codes();
    codes
        .iter()
        .position(|item| item.eq_ignore_ascii_case(code))
        .unwrap_or(0)
}

fn can_pick_language(screen: Screen) -> bool {
    matches!(screen, Screen::YourWords | Screen::WhatIUnderstood)
}

fn open_language_picker(app: App) -> App {
    let cursor = picker_cursor_for(app.pair().known());
    app.with_modal(ModalKind::PickMyLanguage)
        .with_picker_cursor(cursor)
}

fn next_known(current: &str, direction: i32) -> String {
    let codes = catalog().codes();
    let mut position: i32 = 0;
    for (index, code) in codes.iter().enumerate() {
        if *code == current {
            position = index as i32;
            break;
        }
    }
    let next = (position + direction).rem_euclid(codes.len() as i32) as usize;
    String::from(codes[next])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{CardDraft, CardMeta, LanguagePair};
    use crate::tui::screen::{KeySource, WelcomeFocus};

    fn enter_key(env_available: bool) -> App {
        App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
            WelcomeStage::EnterKey,
            KeySource::Empty,
            "",
            env_available,
        )
    }

    #[test]
    fn confirming_the_language_persists_it_before_the_key_step() {
        let app =
            App::new(LanguagePair::new("en", "ru")).opening_welcome(KeySource::Empty, "", false);
        let (next, side) = transit(app, AppEvent::KeyEnter);
        assert_eq!(
            (next.welcome().stage, side),
            (
                WelcomeStage::EnterKey,
                Side::PersistMyLanguage(String::from("ru")),
            ),
            "confirming the language must advance to the key step and persist that choice"
        );
    }

    #[test]
    fn submit_with_a_key_asks_for_validation() {
        let app = enter_key(false).welcome_paste_key("123456789012345678901234567890");
        let (_next, side) = transit(app, AppEvent::Submit);
        assert_eq!(
            side,
            Side::ValidateKey(String::from("123456789012345678901234567890")),
            "submitting a filled key must request a live validity check"
        );
    }

    #[test]
    fn submit_without_a_key_nudges_instead_of_validating() {
        let (next, side) = transit(enter_key(false), AppEvent::Submit);
        assert_eq!(
            (side, next.welcome().notice.clone()),
            (Side::None, Some(String::from("enter a key first"))),
            "submitting an empty key must nudge the user, not call the API"
        );
    }

    #[test]
    fn the_env_action_joins_focus_only_when_env_has_a_key() {
        let with_env = transit(enter_key(true), AppEvent::CursorRight).0;
        let without_env = transit(enter_key(false), AppEvent::CursorRight).0;
        assert_eq!(
            (with_env.welcome().focus, without_env.welcome().focus),
            (WelcomeFocus::LoadEnv, WelcomeFocus::Submit),
            "← → reaches load-from-env only when env offers a key; otherwise focus stays on submit"
        );
    }

    #[test]
    fn submit_while_the_env_action_is_focused_loads_from_env() {
        let app = transit(
            enter_key(true),
            AppEvent::WelcomeFocusTo(WelcomeFocus::LoadEnv),
        )
        .0;
        let (_next, side) = transit(app, AppEvent::Submit);
        assert_eq!(
            side,
            Side::LoadEnvKey,
            "submit on the focused env action must load the key from env, not validate"
        );
    }

    #[test]
    fn horizontal_arrows_move_focus_without_touching_language() {
        let before = enter_key(true);
        let known_before = before.pair().known().to_string();
        let after = transit(before, AppEvent::CursorRight).0;
        assert_eq!(
            (after.pair().known().to_string(), after.welcome().focus),
            (known_before, WelcomeFocus::LoadEnv),
            "← → must move control focus and leave the language untouched"
        );
    }

    #[test]
    fn vertical_arrows_on_the_key_step_do_nothing() {
        let down = transit(enter_key(true), AppEvent::NavNext).0;
        let up = transit(enter_key(true), AppEvent::NavPrev).0;
        assert_eq!(
            (down.welcome().focus, up.welcome().focus),
            (WelcomeFocus::Submit, WelcomeFocus::Submit),
            "↑↓ must not move focus on the key step — movement is horizontal only"
        );
    }

    #[test]
    fn typing_on_the_key_step_keeps_the_button_focus() {
        let app = transit(enter_key(false), AppEvent::KeyChar('A')).0;
        assert_eq!(
            (app.welcome().key.clone(), app.welcome().focus),
            (String::from("A"), WelcomeFocus::Submit),
            "typing fills the always-editable field and leaves focus on the button"
        );
    }

    #[test]
    fn typing_r_inside_the_sentence_note_does_not_reopen_the_editor() {
        let pair = LanguagePair::new("fr", "en");
        let opened = App::new(pair.clone())
            .with_screen(Screen::YourCards)
            .cards_started(vec![CardDraft::new("canard", "a duck", pair).with_meta(
                CardMeta::new(
                    "/canard/",
                    "/canard sentence/",
                    "duck",
                    5,
                    "source with canard",
                    "canard",
                    "hint",
                    "context",
                    "Example with canard.",
                ),
                None,
            )])
            .sentence_editor_opened_for_note();
        let typed = "rewrite".chars().fold(opened, |app, symbol| {
            transit(app, AppEvent::KeyChar(symbol)).0
        });
        assert_eq!(
            typed.sentence_editor().map(|editor| editor.note().value()),
            Some("rewrite"),
            "the R shortcut swallowed or reset an r typed inside the sentence note"
        );
    }
}
