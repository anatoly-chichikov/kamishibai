//! Integration flow for the `Done` screen (08-done.png).

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(100, 20);
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

fn published() -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::Done)
        .confirmed_learning("en")
        .done_published(
            "en_2026-04-17_183029.apkg",
            "en_2026-04-17_183029.pdf",
            "kamishibai-out/",
        )
}

fn failed_published() -> App {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for _ in 0..3 {
        picture = picture.attempted();
    }
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    );
    let meta = CardMeta::new(
        "/wreck/",
        "/wreck sentence/",
        "meaning of wreck",
        5,
        "source for wreck",
        "wreck",
        "hint for wreck",
        "context for wreck",
        "Example with wreck.",
    );
    let draft = CardDraft::new("wreck", "verb sense", LanguagePair::new("en", "ru"))
        .with_meta(meta, None)
        .with_artifacts(artifacts);
    published().cards_started(vec![draft])
}

#[test]
fn done_screen_lists_short_artifact_labels_and_quit_hint() {
    let rendered = flat(&published());
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("APKG")
            && rendered.contains("PDF")
            && rendered.contains("[Ctrl+C]")
            && rendered.contains("RU → EN")
            && !rendered.contains("new batch"),
        "Done must show APKG/PDF labels, quit hint, and the language chip — no new-batch hook"
    );
}

#[test]
fn quit_from_done_requests_app_exit() {
    let app = published();
    let (_, side) = transit(app, AppEvent::Quit);
    assert_eq!(
        side,
        Side::ExitApp,
        "Q on Done must request the shell to exit the application"
    );
}

#[test]
fn done_with_failed_cards_offers_regenerate() {
    let rendered = flat(&failed_published());
    assert!(
        rendered.contains("[Ctrl+G] Regenerate"),
        "Done with failed cards must expose Ctrl+G Regenerate: {rendered}"
    );
}

#[test]
fn ctrl_g_from_done_restarts_failed_cards_on_your_cards() {
    let app = failed_published();
    let (next, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        (next.screen(), side),
        (Screen::YourCards, Side::RegenerateFailed),
        "Ctrl+G on Done must return to YourCards and restart failed artifacts"
    );
}
