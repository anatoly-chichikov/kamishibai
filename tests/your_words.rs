//! Integration flow for `app start -> Your words -> What I understood`.
//!
//! Exercises the real input mapper, transition function, renderer, and the
//! language pair the shell persists. No real Gemini: the understanding pass
//! is mocked with `ScriptDetection`, which is the deterministic fallback the
//! production code uses when the LLM is unavailable.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::languages::catalog;
use kamishibai::session::{LanguagePair, LearningDetection, ScriptDetection};
use kamishibai::tui::{
    App, AppEvent, BusyKind, Screen, Side, draw, scroll_body_width, scroll_viewport, to_app,
    transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tempfile::tempdir;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn flatten(app: &App) -> String {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("test backend must boot");
    terminal
        .draw(|frame| draw(frame, app))
        .expect("draw must succeed");
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

fn long_blob(count: usize) -> String {
    (1..=count)
        .map(|index| format!("word-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn apply_side(app: App, side: Side) -> App {
    match side {
        Side::RunUnderstanding => {
            let guess = ScriptDetection
                .detect(app.blob(), &catalog())
                .expect("detection must succeed");
            app.with_screen(Screen::WhatIUnderstood)
                .confirmed_learning(guess.code())
        }
        _ => app,
    }
}

#[test]
fn long_pasted_your_words_scrolls_to_the_cursor_line() {
    let area = Rect::new(0, 0, 80, 12);
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob(long_blob(60));
    let viewport = scroll_viewport(&app, area);
    let width = scroll_body_width(area);
    let app = app.body_scroll_to_selection(viewport, width);
    let flat = flatten(&app);
    assert!(
        flat.contains("word-60") && !flat.contains("word-01"),
        "long pasted word lists must render the cursor end of the scrollable editor: {flat}"
    );
}

#[test]
fn repeated_enter_on_your_words_moves_the_editor_scroll() {
    let mut app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("word-01")
        .body_scroll_to_selection(4, 72);
    for _ in 0..12 {
        app = transit(app, AppEvent::KeyEnter)
            .0
            .body_scroll_to_selection(4, 72);
    }
    assert!(
        app.body_scroll() > 0,
        "typing past the visible editor height must advance the body scroll"
    );
}

#[test]
fn your_words_renders_placeholder_tagline_and_language_pair() {
    let app = App::new(LanguagePair::new("en", "ru")).confirmed_learning("en");
    let flat = flatten(&app);
    assert!(
        flat.contains("words you want to learn")
            && flat.contains("each word becomes a small learning scene")
            && flat.contains("step 1/3")
            && (flat.contains("RU→EN") || flat.contains("RU → EN")),
        "your words screen must render the PDF labels and a language chip on the right: {flat}"
    );
}

#[test]
fn your_words_footer_shows_language_shortcut() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let flat = flatten(&app);
    assert!(
        flat.contains("[Ctrl+L] language"),
        "your words footer must reveal the language picker shortcut: {flat}"
    );
}

#[test]
fn your_words_footer_keeps_paste_shortcut() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let flat = flatten(&app);
    assert!(
        flat.contains("[Cmd+V] paste"),
        "your words footer must keep the paste shortcut: {flat}"
    );
}

#[test]
fn armed_words_clear_makes_escape_the_only_primary_action() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("whilst")
        .with_word_clear_pending(true);
    let rendered = flatten(&app);
    assert!(
        rendered.contains("[Esc] again") && !rendered.contains("[Ctrl+G] continue"),
        "armed words clear competed with another primary footer action: {rendered}"
    );
}

#[test]
fn busy_loader_covers_the_current_screen_with_request_status() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .busy_started(BusyKind::Understanding)
        .busy_elapsed(Duration::from_millis(320));
    let flat = flatten(&app);
    assert!(
        flat.contains("ai is working") && flat.contains("understanding your words"),
        "busy loader must cover the current screen with a visible request status"
    );
}

#[test]
fn busy_loader_owns_keyboard_input_until_request_finishes() {
    let app = App::new(LanguagePair::new("en", "ru")).busy_started(BusyKind::Understanding);
    let event = to_app(press(KeyCode::Char('x'))).expect("char must map");
    let (next, side) = transit(app, event);
    assert_eq!(
        (
            next.blob().to_string(),
            next.busy().map(|busy| busy.kind()),
            side,
        ),
        (String::new(), Some(BusyKind::Understanding), Side::None),
        "busy loader must suppress ordinary keyboard input until the request finishes"
    );
}

#[test]
fn understanding_busy_overlay_keeps_the_your_words_background() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("deed\nhonor");
    let (after_submit, side) = transit(app, AppEvent::Generate);
    let loading = after_submit.busy_started(BusyKind::Understanding);
    let flat = flatten(&loading);
    assert!(
        side == Side::RunUnderstanding
            && loading.screen() == Screen::YourWords
            && flat.contains("words you want to learn")
            && !flat.contains("what i understood"),
        "understanding loader must keep the previous screen behind it until Gemini returns: {flat}"
    );
}

#[test]
fn recoverable_error_overlay_keeps_the_message_visible() {
    let app = App::new(LanguagePair::new("en", "ru")).error_shown("INTERNAL: boom");
    let flat = flatten(&app);
    assert!(
        flat.contains("can't reach gemini")
            && flat.contains("INTERNAL: boom")
            && flat.contains("press any key to dismiss"),
        "recoverable Gemini errors must render as an in-app overlay"
    );
}

#[test]
fn recoverable_error_overlay_wraps_long_timeout_messages() {
    let app = App::new(LanguagePair::new("en", "ru")).error_shown(
        "error sending request for url https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent: operation timed out while understanding a large word list",
    );
    let flat = flatten(&app);
    assert!(
        flat.contains("operation timed out")
            && flat.contains("large word list")
            && flat.contains("press any key to dismiss"),
        "long Gemini timeout messages must remain visible inside the overlay: {flat}"
    );
}

/// The key a user reaches for after a dead batch is the one that retries it.
/// Spending that press on dismissing the notice is what made a failed batch feel
/// unrecoverable, so Generate must dismiss and retry in the same press.
#[test]
fn generate_over_a_shown_error_dismisses_it_and_retries_in_one_press() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("back in the day")
        .error_shown("no completed cards to publish");
    let (after, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        (after.error().is_some(), side),
        (false, Side::RunUnderstanding),
        "Ctrl+G over an error was spent dismissing it instead of retrying"
    );
}

/// Every other key still just clears the notice.
#[test]
fn any_other_key_over_a_shown_error_only_dismisses_it() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("back in the day")
        .error_shown("no completed cards to publish");
    let (after, side) = transit(app, AppEvent::NavNext);
    assert_eq!(
        (after.error().is_some(), side),
        (false, Side::None),
        "dismissing an error must not carry the dismissing key into the screen"
    );
}

/// A one-line failure must not open a near-fullscreen box. The panel is sized
/// from the message, so it stays an overlay you can see the screen behind.
#[test]
fn a_short_error_keeps_the_overlay_small() {
    let app = App::new(LanguagePair::new("en", "ru")).error_shown("no completed cards to publish");
    let painted = flatten(&app)
        .lines()
        .filter(|line| line.contains('\u{2502}') || line.contains('\u{250c}'))
        .count();
    assert!(
        painted < 10,
        "a one-line error opened a {painted}-row overlay instead of sizing to its message"
    );
}

/// A long message still gets the room it needs, so shrinking the panel did not
/// turn into clipping.
#[test]
fn a_long_error_still_gets_the_room_it_needs() {
    let app = App::new(LanguagePair::new("en", "ru")).error_shown(
        "error sending request for url https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent: operation timed out while understanding a large word list",
    );
    let flat = flatten(&app);
    assert!(
        flat.contains("operation timed out") && flat.contains("large word list"),
        "sizing the overlay to its message clipped a long one: {flat}"
    );
}

#[test]
fn ctrl_l_on_your_words_opens_the_language_picker() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (next, side) = transit(
        app,
        to_app(modified(KeyCode::Char('l'), KeyModifiers::CONTROL)).expect("map"),
    );
    assert_eq!(
        (next.modal(), side),
        (Some(kamishibai::tui::ModalKind::PickLanguages), Side::None,),
        "Ctrl+L on Your words must open the language picker modal without persisting yet"
    );
}

#[test]
fn cmd_l_on_your_words_opens_the_language_picker() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (next, side) = transit(
        app,
        to_app(modified(KeyCode::Char('l'), KeyModifiers::SUPER)).expect("map"),
    );
    assert_eq!(
        (next.modal(), side),
        (Some(kamishibai::tui::ModalKind::PickLanguages), Side::None,),
        "Cmd+L on Your words must open the language picker modal in kitty-protocol terminals"
    );
}

#[test]
fn enter_on_empty_blob_inserts_newline_and_stays_on_your_words() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let event = to_app(press(KeyCode::Enter)).expect("Enter must map");
    let (next, side) = transit(app, event);
    assert_eq!(
        (next.screen(), next.blob().to_string(), side),
        (Screen::YourWords, String::from("\n"), Side::None),
        "plain Enter on an empty blob must insert a newline without side effects"
    );
}

#[test]
fn left_arrow_on_your_words_inserts_before_the_last_character() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("harbor");
    let (after_left, _) = transit(app, to_app(press(KeyCode::Left)).expect("Left must map"));
    let (after_type, side) = transit(
        after_left,
        to_app(press(KeyCode::Char('u'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("harbour"), Side::None),
        "left arrow on Your words must move the text cursor before the next insertion"
    );
}

#[test]
fn left_arrow_on_your_words_moves_over_utf8_characters() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("окно");
    let (after_left, _) = transit(app, to_app(press(KeyCode::Left)).expect("Left must map"));
    let (after_type, side) = transit(
        after_left,
        to_app(press(KeyCode::Char('!'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("окн!о"), Side::None),
        "left arrow on Your words must move by UTF-8 character boundaries instead of bytes"
    );
}

#[test]
fn up_arrow_on_your_words_inserts_on_the_previous_line() {
    let app = App::new(LanguagePair::new("en", "ru")).seeded_blob("moon\nship");
    let (after_up, _) = transit(app, to_app(press(KeyCode::Up)).expect("Up must map"));
    let (after_type, side) = transit(
        after_up,
        to_app(press(KeyCode::Char('!'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("moon!\nship"), Side::None),
        "up arrow on Your words must move the text cursor onto the previous line"
    );
}

#[test]
fn arrows_on_empty_your_words_materialize_the_requested_position() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let (after_down, _) = transit(app, to_app(press(KeyCode::Down)).expect("Down must map"));
    let (after_right, _) = transit(
        after_down,
        to_app(press(KeyCode::Right)).expect("Right must map"),
    );
    let (after_type, side) = transit(
        after_right,
        to_app(press(KeyCode::Char('x'))).expect("char must map"),
    );
    assert_eq!(
        (after_type.blob().to_string(), side),
        (String::from("\n x"), Side::None),
        "arrows on empty Your words must let the first typed character land at the chosen row and column"
    );
}

#[test]
fn typing_and_pressing_ctrl_g_waits_then_advances_to_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"));
    let mut state = app;
    for symbol in "окно".chars() {
        let event = to_app(press(KeyCode::Char(symbol))).expect("char must map");
        let (next, _) = transit(state, event);
        state = next;
    }
    let submit =
        to_app(modified(KeyCode::Char('g'), KeyModifiers::CONTROL)).expect("Ctrl+G must map");
    let (after_submit, side) = transit(state, submit);
    let submitted_screen = after_submit.screen();
    let resolved = apply_side(after_submit, side.clone());
    assert_eq!(
        (
            submitted_screen,
            resolved.screen(),
            side,
            resolved.learning_pending(),
            resolved.pair().learning().to_string(),
        ),
        (
            Screen::YourWords,
            Screen::WhatIUnderstood,
            Side::RunUnderstanding,
            false,
            String::from("ru"),
        ),
        "Ctrl+G must request understanding without changing screens until the result arrives"
    );
}

#[test]
fn preference_store_feeds_my_language_into_the_initial_pair() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("es"))
        .expect("must persist my language");
    let persisted = store.read().expect("must reload my language").my_language;
    let app = App::new(LanguagePair::new("en", persisted.as_str()));
    assert_eq!(
        app.pair().known(),
        "es",
        "persisted my language must feed into the initial pair at app start"
    );
}
