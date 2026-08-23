use crate::languages::catalog;
use crate::session::{MAX_INTAKE_WORDS, MAX_PLAN_CARDS, RawInputBatch};

use super::app::{App, ReviewFocus};
use super::disclosure::{DisclosureControls, DisclosureIntent};
use super::event::AppEvent;
use super::input::latin_key;
use super::picker::{LanguageChoice, PickerCursor, PickerSection, learning_target};
use super::screen::{ModalKind, Screen, WelcomeFocus, WelcomeStage};
use super::sentence_editor::{BatchSettingsRow, LabelEditorRow};

/// A side effect requested by a transition. The shell interprets it outside the pure function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Side {
    None,
    RunUnderstanding,
    RunBulkCorrection(String),
    /// Adopt one language pair and re-read the words under it. The shell saves
    /// the known half to preferences, rotates the session identity when the
    /// learning half moved, and reruns the understanding pass.
    AdoptLanguagesAndRunUnderstanding(LanguageChoice),
    /// Ask the shell to arm or confirm clearing the nonempty words input.
    ClearWords,
    StartGeneration,
    /// Ask the shell to arm or confirm stopping the active card engine.
    StopGeneration,
    RegenerateFailed,
    RegenerateCards,
    /// Adopt one language pair before any words were read. Only the known half
    /// is persisted; the learning half stays a session-local pin.
    AdoptLanguages(LanguageChoice),
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
        // A shown error is a notice, not a question: any key dismisses it. But
        // the one key the user reaches for after a failed batch is the one that
        // retries it, and spending that press on the dismissal makes a dead
        // batch feel unrecoverable. Generate dismisses and retries in one press.
        let cleared = app.error_cleared();
        if event != AppEvent::Generate {
            return (cleared, Side::None);
        }
        return transit(cleared, event);
    }
    if app.busy().is_some() && event != AppEvent::Redraw {
        return (app, Side::None);
    }
    let event = promote(&app, event);
    match (app.screen(), app.modal(), event) {
        (Screen::Welcome, _, e) => welcome(app, e),
        (Screen::YourWords, None, AppEvent::Generate) => {
            let batch = RawInputBatch::new(app.blob());
            if batch.has_content() && batch.word_count() <= MAX_INTAKE_WORDS {
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
        (Screen::YourWords, None, AppEvent::OpenPreferredLanguagePicker) => {
            (open_language_picker(app, PickerSection::Known), Side::None)
        }
        (Screen::YourWords, None, AppEvent::OpenLanguagePicker(section)) => {
            (open_language_picker(app, section), Side::None)
        }
        (Screen::YourWords, None, AppEvent::Cancel) if !app.blob().is_empty() => {
            (app, Side::ClearWords)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Generate) => start_generation(app),
        (Screen::WhatIUnderstood, None, AppEvent::Cancel)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::SentenceSettingsOpen)
            if !app.candidates().is_empty() =>
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
            if app.sentence_settings_editor() == Some(BatchSettingsRow::Types) =>
        {
            (app.sentence_settings_closed(), Side::None)
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
        (Screen::WhatIUnderstood, None, AppEvent::KeyEnter)
            if app.sentence_settings_editor().is_some() =>
        {
            (app.sentence_settings_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Submit)
            if app.sentence_settings_editor().is_some() =>
        {
            (app, Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('c' | 'C'))
            if !app.candidates().is_empty() =>
        {
            if app.any_sense_list_open() || app.sentence_settings_editor().is_some() {
                (
                    app.sentence_settings_closed().sense_lists_collapsed(),
                    Side::None,
                )
            } else {
                (app.sense_lists_expanded_all(), Side::None)
            }
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar(_))
            if app.sentence_settings_editor().is_some() =>
        {
            (app, Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('s' | 'S'))
            if matches!(app.review_focus(), ReviewFocus::Head(_))
                && !app.candidates().is_empty() =>
        {
            (app.sentence_settings_opened(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Cancel) if app.focused_sense_list_open() => {
            (app.sense_list_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::Cancel) => {
            let choice = LanguageChoice::new(app.pair().known().to_string(), learning_target(None));
            (
                app.languages_adopted(&choice)
                    .with_screen(Screen::YourWords),
                Side::None,
            )
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyEnter)
            if matches!(app.review_focus(), ReviewFocus::Sense { .. }) =>
        {
            (app.sense_list_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyEnter) => {
            (app.sense_list_toggled(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar(' '))
            if app.expanded_add_more_focused() =>
        {
            (app.with_modal(ModalKind::ChangeSomething), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar(' '))
            if matches!(app.review_focus(), ReviewFocus::Sense { .. }) =>
        {
            (app.sense_toggled(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorRight)
            if !app.focused_sense_list_open() =>
        {
            (app.sense_list_toggled(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorLeft) if app.focused_sense_list_open() => {
            (app.sense_list_closed(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::CursorLeft) => (app, Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::CursorRight) => (app, Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::NavPrev)
            if app.review_focus() == ReviewFocus::Head(0) && !app.candidates().is_empty() =>
        {
            (
                app.sentence_settings_opened()
                    .sentence_settings_focused(BatchSettingsRow::Types),
                Side::None,
            )
        }
        // Dropping a word is offered on the head row, where the footer names
        // it. Inside an open sense list the same letter would throw away the
        // whole word the user is busy picking meanings for, and nothing on
        // screen would have warned them.
        (Screen::WhatIUnderstood, None, AppEvent::KeyChar('d' | 'D'))
            if matches!(app.review_focus(), ReviewFocus::Head(_)) =>
        {
            let next = app.dropped_selected();
            if next.candidates().is_empty() {
                // Emptying the list leaves nothing to review, so we fall back
                // to the words — the same place Esc goes, and with the same
                // words still in the box. One letter must not be the only way
                // to lose everything that was typed.
                (
                    next.with_screen(Screen::YourWords).body_scroll_reset(),
                    Side::None,
                )
            } else {
                (next, Side::None)
            }
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavPrev) => {
            (app.review_focus_previous(), Side::None)
        }
        (Screen::WhatIUnderstood, None, AppEvent::NavNext) => (app.review_focus_next(), Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::OpenPreferredLanguagePicker) => (
            open_language_picker(app, PickerSection::Learning),
            Side::None,
        ),
        (Screen::WhatIUnderstood, None, AppEvent::OpenLanguagePicker(section)) => {
            (open_language_picker(app, section), Side::None)
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
        (_, Some(ModalKind::PickLanguages), AppEvent::LanguagePickerPrev) => {
            (app.picker_cursor_advanced(-1), Side::None)
        }
        (_, Some(ModalKind::PickLanguages), AppEvent::LanguagePickerNext) => {
            (app.picker_cursor_advanced(1), Side::None)
        }
        (_, Some(ModalKind::PickLanguages), AppEvent::LanguagePickerFocus(section)) => {
            (app.picker_facing(section), Side::None)
        }
        (_, Some(ModalKind::PickLanguages), AppEvent::LanguagePickerPoint(section, index)) => {
            (app.picker_chosen(section, index), Side::None)
        }
        (_, Some(ModalKind::PickLanguages), AppEvent::Cancel) => (app.close_modal(), Side::None),
        (_, Some(ModalKind::PickLanguages), AppEvent::SetLanguages(choice))
            if can_pick_language(app.screen()) =>
        {
            adopt_languages(app.close_modal(), choice)
        }
        (_, Some(ModalKind::PickLanguages), _) => (app, Side::None),
        (Screen::WhatIUnderstood, None, AppEvent::SetLanguages(choice)) => {
            adopt_languages(app, choice)
        }
        (Screen::YourCards, None, AppEvent::Cancel) if app.sentence_editor().is_some() => {
            (app.sentence_editor_parked(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::Cancel) if app.card_expanded() => {
            (app.card_toggle_expanded(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::Cancel)
            if !app.cards().is_empty() && !app.can_start_new_batch() =>
        {
            (app, Side::StopGeneration)
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelOpen(card, row))
            if app.card_tunable_at(card) =>
        {
            (
                app.card_revealed(card).sentence_editor_opened_for(row),
                Side::None,
            )
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelFocus(row)) if app.card_tunable() => {
            (live_sentence_editor(app, row), Side::None)
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelChoose(row, index))
            if app.card_tunable() =>
        {
            (
                live_sentence_editor(app, row).sentence_editor_axis_chosen(index),
                Side::None,
            )
        }
        (Screen::YourCards, None, AppEvent::SentenceLabelAdvance(row, forward))
            if app.card_tunable() =>
        {
            (
                live_sentence_editor(app, row).sentence_editor_axis_advanced(forward),
                Side::None,
            )
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
        (Screen::YourCards, None, AppEvent::KeyChar('c' | 'C'))
            if !sentence_note_focused(&app) && !app.cards().is_empty() =>
        {
            if app.any_card_expanded() {
                (app.cards_collapsed(), Side::None)
            } else {
                (app.cards_expanded_all(), Side::None)
            }
        }
        (Screen::YourCards, None, AppEvent::KeyChar(symbol)) if app.sentence_editor().is_some() => {
            (app.sentence_editor_typed(symbol), Side::None)
        }
        (Screen::YourCards, None, AppEvent::KeyBackspace) if app.sentence_editor().is_some() => {
            (app.sentence_editor_rubbed(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::Submit) if app.sentence_editor().is_some() => {
            (app, Side::None)
        }
        (Screen::YourCards, None, AppEvent::Generate) if app.sentence_editor().is_some() => {
            (app.sentence_editor_closed(), Side::RegenerateCards)
        }
        (Screen::YourCards, None, AppEvent::KeyChar(' ')) if !app.cards().is_empty() => {
            (app.card_toggle_expanded(), Side::None)
        }
        (Screen::YourCards, None, AppEvent::NextUnfinished) => (app.card_jumped(true), Side::None),
        (Screen::YourCards, None, AppEvent::PreviousUnfinished) => {
            (app.card_jumped(false), Side::None)
        }
        (Screen::YourCards, None, AppEvent::NavPrev) => (app.card_focus_previous(), Side::None),
        (Screen::YourCards, None, AppEvent::NavNext) => (app.card_focus_next(), Side::None),
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

/// Return the app with the focused card's editor live on one row, opening it
/// when the card was merely displaying those rows. A click on a tune control
/// says "tune this" and "pick this" at once, so it must not need the block to
/// already own the keyboard.
fn live_sentence_editor(app: App, row: LabelEditorRow) -> App {
    if app.sentence_editor().is_some() {
        return app.sentence_editor_focused(row);
    }
    app.sentence_editor_opened_for(row)
}

fn sentence_note_focused(app: &App) -> bool {
    app.sentence_editor()
        .is_some_and(|editor| editor.row() == LabelEditorRow::Note)
}

fn start_generation(app: App) -> (App, Side) {
    if !app.candidates().iter().any(|candidate| candidate.ok()) {
        return (app, Side::None);
    }
    if app.review_cards() > MAX_PLAN_CARDS {
        return (
            app.review_noticed(format!(
                "over the {MAX_PLAN_CARDS}-card limit — deselect senses"
            )),
            Side::None,
        );
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
        (WelcomeStage::PickLanguage, AppEvent::WelcomeLanguageAt(index)) => {
            match PickerSection::Known.code_at(index) {
                Some(code) => (app.set_known(code), Side::None),
                None => (app, Side::None),
            }
        }
        (WelcomeStage::PickLanguage, AppEvent::Submit)
        | (WelcomeStage::PickLanguage, AppEvent::KeyEnter) => {
            let choice = LanguageChoice::new(app.pair().known().to_string(), learning_target(None));
            (app.welcome_advance(), Side::AdoptLanguages(choice))
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
        (WelcomeStage::EnterKey, AppEvent::KeyBackspace) => (app.welcome_rubbed_key(), Side::None),
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

/// Reinterpret one key by the context it arrives in, before the screen match.
///
/// The two screens that own plain-letter hotkeys take no text, so a letter
/// they receive is folded to the Latin key it was typed on and `C`, `S`, `D`
/// and the `j`/`k` walk answer on any layout. Every screen that does take text
/// — the words editor, the Welcome key field, any modal, and the focused
/// rewrite note — is left out and keeps the codepoint the user typed.
fn promote(app: &App, event: AppEvent) -> AppEvent {
    if let Some(ModalKind::PickLanguages) = app.modal() {
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
        (
            Screen::Welcome,
            AppEvent::OpenPreferredLanguagePicker | AppEvent::OpenLanguagePicker(_),
        ) if app.welcome().stage == WelcomeStage::PickLanguage => AppEvent::WelcomeNextLanguage,
        (Screen::WhatIUnderstood, AppEvent::KeyChar(symbol)) => walk(latin_key(*symbol)),
        (Screen::YourCards, AppEvent::KeyChar(symbol)) if !sentence_note_focused(app) => {
            walk(latin_key(*symbol))
        }
        _ => event,
    }
}

/// Fold the vim walk onto the arrow events.
///
/// `j` and `k` used to be spelled out in the review screen's own match arms,
/// which meant they worked on exactly one of the two list screens and behaved
/// slightly differently from the arrows even there. Folding them here makes
/// them an alias in the plain sense: every arm downstream sees one event, so
/// the two keys cannot drift apart from the arrows again.
fn walk(key: char) -> AppEvent {
    match key {
        'j' => AppEvent::NavNext,
        'k' => AppEvent::NavPrev,
        other => AppEvent::KeyChar(other),
    }
}

/// Reinterpret keys while the language pair modal is open.
///
/// The modal is two side-by-side vertical lists, so the axes follow the shape:
/// `↑/↓` move inside the focused column, `←/→` name which column has focus,
/// Enter confirms both at once, and plain Esc still cancels through the generic
/// modal dismissal arm.
fn promote_picker(app: &App, event: AppEvent) -> AppEvent {
    match event {
        AppEvent::NavPrev => AppEvent::LanguagePickerPrev,
        AppEvent::NavNext => AppEvent::LanguagePickerNext,
        AppEvent::CursorLeft => AppEvent::LanguagePickerFocus(PickerSection::Known),
        AppEvent::CursorRight => AppEvent::LanguagePickerFocus(PickerSection::Learning),
        AppEvent::Submit | AppEvent::KeyEnter => AppEvent::SetLanguages(picker_selected(app)),
        other => other,
    }
}

/// Return the pair currently highlighted across both halves of the modal.
pub fn picker_selected(app: &App) -> LanguageChoice {
    app.picker_cursor().choice()
}

fn can_pick_language(screen: Screen) -> bool {
    matches!(screen, Screen::YourWords | Screen::WhatIUnderstood)
}

/// Adopt one confirmed pair. A choice that changes nothing is swallowed, so
/// confirming the modal by reflex never costs a provider call.
fn adopt_languages(app: App, choice: LanguageChoice) -> (App, Side) {
    if settled(&app, &choice) {
        return (app, Side::None);
    }
    let rereads = rereads(&app, &choice);
    if rereads && RawInputBatch::new(app.blob()).word_count() > MAX_INTAKE_WORDS {
        return (
            app.review_noticed("too many words to re-read — start a new batch"),
            Side::None,
        );
    }
    let next = app.languages_adopted(&choice);
    if rereads {
        return (next, Side::AdoptLanguagesAndRunUnderstanding(choice));
    }
    (next, Side::AdoptLanguages(choice))
}

/// Return whether the choice already describes the current state.
fn settled(app: &App, choice: &LanguageChoice) -> bool {
    app.pair().known().eq_ignore_ascii_case(choice.known())
        && app.learning_target() == choice.learning()
}

/// Return whether the words on screen have to be read again.
///
/// Pinning the language already showing costs nothing to re-read — the
/// candidates in view were understood as exactly that language. Dropping a pin
/// does rerun: `auto` means "I was wrong, decide for me".
fn rereads(app: &App, choice: &LanguageChoice) -> bool {
    if app.screen() != Screen::WhatIUnderstood || app.candidates().is_empty() {
        return false;
    }
    if !app.pair().known().eq_ignore_ascii_case(choice.known()) {
        return true;
    }
    match choice.pinned() {
        None => app.learning_pin().is_some(),
        Some(code) => app.learning_pending() || !app.pair().learning().eq_ignore_ascii_case(code),
    }
}

fn open_language_picker(app: App, section: PickerSection) -> App {
    let cursor = PickerCursor::opening(app.pair().known(), app.learning_pin(), section);
    app.with_modal(ModalKind::PickLanguages)
        .with_picker_cursor(cursor)
}

fn next_known(current: &str, direction: i32) -> String {
    let codes = catalog().codes();
    let mut position: i32 = 0;
    for (index, code) in codes.iter().enumerate() {
        if code.eq_ignore_ascii_case(current) {
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
    fn backspace_on_the_key_rubs_out_one_character() {
        let typed = enter_key(false).welcome_paste_key("abcd");
        let rubbed = transit(typed, AppEvent::KeyBackspace).0;
        assert_eq!(
            rubbed.welcome().key,
            "abc",
            "backspace wiped the whole key on the one screen where a typo is what you came to fix"
        );
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
                Side::AdoptLanguages(LanguageChoice::new(
                    "ru".to_string(),
                    crate::application::LearningTarget::Detect,
                )),
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
