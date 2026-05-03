//! Language pair must be visible on every fullscreen state and the user must
//! be able to change `my language` without leaving the flow. The change has
//! to persist across app restarts via the preference store.

use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardBody, CardDraft, LanguagePair, WordCandidate,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
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
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn body_for(term: &str) -> CardBody {
    CardBody::new(
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
                .with_body(body_for("whilst"), None)
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
fn toggle_my_language_on_what_i_understood_persists_the_new_code() {
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (after, side) = transit(app, AppEvent::KeyChar('L'));
    assert_eq!(
        (after.pair().support().to_string(), side),
        (
            String::from("es"),
            Side::PersistMyLanguage(String::from("es"))
        ),
        "L on What I understood must rotate `my language` and request persistence"
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
fn toggling_my_language_through_transit_writes_to_the_preference_store() {
    let home = tempdir().expect("temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("ru"))
        .expect("seed my language");
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en");
    let (after, side) = transit(app, AppEvent::KeyChar('L'));
    if let Side::PersistMyLanguage(code) = side {
        store
            .write(&Preferences::new(code))
            .expect("persist new code");
    }
    let restored = store.read().expect("reload preferences").my_language;
    assert_eq!(
        (after.pair().support().to_string(), restored),
        (String::from("es"), String::from("es")),
        "rotating `my language` in-flow must update both the session pair and the persisted preference"
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
