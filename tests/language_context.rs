//! Language pair must be visible on every fullscreen state and the user must
//! be able to change `my language` without leaving the flow. The change has
//! to persist across app restarts via the preference store.

use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, LanguagePair, WordCandidate,
};
use kamishibai::tui::{
    App, AppEvent, KeySource, Screen, Side, WelcomeStage, draw, to_app, transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::tempdir;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 24);
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

fn ready() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn meta_for(term: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source for {term}"),
        term,
        format!("hint for {term}"),
        format!("context for {term}"),
        format!("Example with {term}."),
    )
}

fn base() -> App {
    App::new(LanguagePair::new("en", "ru"))
}

#[test]
fn ctrl_l_on_welcome_language_step_cycles_the_language() {
    let app = App::new(LanguagePair::new("en", "en"))
        .opening_welcome(KeySource::Env, "123456789012345678901234567890");
    let event = to_app(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyModifiers::CONTROL,
    ))
    .expect("Ctrl+L must map");
    let (after, side) = transit(app, event);
    assert_eq!(
        (
            after.screen(),
            after.welcome().stage,
            after.pair().support().to_string(),
            side,
        ),
        (
            Screen::Welcome,
            WelcomeStage::PickLanguage,
            kamishibai::languages::catalog().codes()[1].to_string(),
            Side::None,
        ),
        "Ctrl+L on the first Welcome slide must cycle the language without persisting it"
    );
}

#[test]
fn welcome_language_step_renders_the_ctrl_l_hint() {
    let app = App::new(LanguagePair::new("en", "en"))
        .opening_welcome(KeySource::Env, "123456789012345678901234567890");
    let rendered = flat(&app);
    assert!(
        rendered.contains("[Ctrl+L] pick"),
        "Welcome language step must show Ctrl+L as an available language switch shortcut"
    );
}

#[test]
fn language_badge_is_consistent_across_every_fullscreen_screen() {
    let your_words = base().confirmed_target("en");
    let what_i_understood = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(vec![WordCandidate::new(
            "whilst",
            "neutral conjunction",
            true,
        )]);
    let your_cards = base()
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            CardDraft::new("whilst", "understanding", LanguagePair::new("en", "ru"))
                .with_meta(meta_for("whilst"), None)
                .with_artifacts(ready()),
        ]);
    let done = base()
        .with_screen(Screen::Done)
        .confirmed_target("en")
        .done_published("deck.apkg", "report.pdf", "out/");
    for app in [your_words, what_i_understood, your_cards, done] {
        let rendered = flat(&app);
        assert!(
            rendered.contains("→ EN"),
            "every fullscreen screen must render the compact language chip"
        );
    }
}

#[test]
fn your_words_shows_detecting_marker_before_target_is_confirmed() {
    let app = base();
    let rendered = flat(&app);
    assert!(
        rendered.contains("…"),
        "while target is pending the language chip must show a `…` placeholder"
    );
}

#[test]
fn picking_my_language_on_what_i_understood_persists_the_new_code() {
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker);
    assert_eq!(
        opened.modal(),
        Some(kamishibai::tui::ModalKind::PickMyLanguage),
        "OpenLanguagePicker on What I understood must open the picker modal"
    );
    let (after, side) = transit(opened, AppEvent::SetMyLanguage(String::from("es")));
    assert_eq!(
        (after.modal(), after.pair().support().to_string(), side,),
        (
            None,
            String::from("es"),
            Side::PersistMyLanguage(String::from("es")),
        ),
        "confirming a language pick must close the modal, swap support, and request persistence"
    );
}

#[test]
fn letter_l_on_done_is_not_a_hidden_language_shortcut() {
    let app = base()
        .with_screen(Screen::Done)
        .confirmed_target("en")
        .done_published("deck.apkg", "report.pdf", "out/");
    let (after, side) = transit(app, AppEvent::KeyChar('l'));
    assert_eq!(
        (after.pair().support().to_string(), side),
        (String::from("ru"), Side::None),
        "letter L on Done must not be a hidden language shortcut"
    );
}

#[test]
fn picker_modal_opens_with_the_active_language_preselected_and_arrows_cycle_through_the_catalog() {
    let app = base().confirmed_target("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker);
    let codes = kamishibai::languages::catalog().codes();
    let initial = codes
        .iter()
        .position(|code| *code == "ru")
        .expect("ru must be in the supported catalog");
    assert_eq!(
        opened.picker_cursor(),
        initial,
        "the picker must open with the cursor on the active `my` language"
    );
    let (after_right, _) = transit(opened, AppEvent::LanguagePickerNext);
    let (after_left_left, _) = transit(after_right, AppEvent::LanguagePickerPrev);
    assert_eq!(
        after_left_left.picker_cursor(),
        initial,
        "Prev after Next must return the cursor to the original chip"
    );
}

#[test]
fn picker_does_not_open_on_your_cards_or_done_because_the_pair_is_frozen() {
    for screen in [Screen::YourCards, Screen::Done] {
        let app = base().with_screen(screen).confirmed_target("en");
        let (after, side) = transit(app, AppEvent::OpenLanguagePicker);
        assert_eq!(
            (after.modal(), side),
            (None, Side::None),
            "OpenLanguagePicker on {screen:?} must not open the picker — the batch pair is frozen"
        );
    }
}

#[test]
fn picking_my_language_through_transit_writes_to_the_preference_store() {
    let home = tempdir().expect("temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("ru"))
        .expect("seed my language");
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker);
    let (after, side) = transit(opened, AppEvent::SetMyLanguage(String::from("es")));
    if let Side::PersistMyLanguage(code) = side {
        store
            .write(&Preferences::new(code))
            .expect("persist new code");
    }
    let restored = store.read().expect("reload preferences").my_language;
    assert_eq!(
        (after.pair().support().to_string(), restored),
        (String::from("es"), String::from("es")),
        "picking a language through the modal must update both the session pair and the persisted preference"
    );
}

#[test]
fn typing_letter_l_inside_your_words_does_not_rotate_the_language() {
    let mut state = base();
    for symbol in "apple".chars() {
        let event = kamishibai::tui::to_app(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(symbol),
            crossterm::event::KeyModifiers::NONE,
        ))
        .expect("char");
        state = transit(state, event).0;
    }
    assert_eq!(
        state.pair().support().to_string(),
        "ru",
        "letter L inside Your words blob must be treated as user input, not as a global shortcut"
    );
}
