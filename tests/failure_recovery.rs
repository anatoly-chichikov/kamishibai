//! Recovery flow for failed cards (`07-your-cards-couldnt-finish.png`).

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardPayload, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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

fn failed_picture() -> CardArtifacts {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for _ in 0..3 {
        picture = picture.attempted();
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn seeded() -> App {
    let draft = CardDraft::new(
        "wreck",
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", "wreck"),
    )
    .with_artifacts(failed_picture());
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![draft])
}

#[test]
fn your_cards_surfaces_failure_banner_when_any_card_fails_terminally() {
    let app = seeded();
    let rendered = flat(&app);
    assert!(
        rendered.contains("1 card couldn't finish after 3 tries")
            && rendered.contains("[r] regenerate failed")
            && rendered.contains("✗ picture"),
        "Your cards must render the failure banner and the [r] key hint when any card failed"
    );
}

#[test]
fn lowercase_r_emits_regenerate_failed_when_any_card_failed_terminally() {
    let app = seeded();
    let (_, side) = transit(app, AppEvent::KeyChar('r'));
    assert_eq!(
        side,
        Side::RegenerateFailed,
        "lowercase r on Your cards must emit the RegenerateFailed side-effect when a card failed"
    );
}

#[test]
fn uppercase_r_opens_change_this_card_even_with_failures_present() {
    let app = seeded();
    let (after, _) = transit(app, AppEvent::KeyChar('R'));
    assert_eq!(
        after.modal(),
        Some(kamishibai::tui::ModalKind::ChangeThisCard),
        "uppercase R must still open Change this card even when a failure banner is visible"
    );
}

#[test]
fn regenerate_failed_resets_only_the_failed_slots_and_keeps_ready_slots() {
    let app = seeded();
    let recovered = app.cards_reset_failures();
    let card = &recovered.cards()[0];
    assert_eq!(
        (
            card.artifacts().scene().ready(),
            card.artifacts().picture().failed_terminally(),
            card.artifacts().picture().tally().done(),
            card.artifacts().sound().ready(),
        ),
        (true, false, 0, false),
        "regenerating failed cards must reset only the failed artifact and keep the rest untouched"
    );
}

#[test]
fn recovery_keeps_the_user_on_your_cards() {
    let app = seeded();
    let (after, _) = transit(app, AppEvent::KeyChar('r'));
    assert_eq!(
        after.screen(),
        Screen::YourCards,
        "regenerating failed cards must stay inline on Your cards"
    );
}
