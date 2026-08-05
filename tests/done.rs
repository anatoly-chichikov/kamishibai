//! Integration flow for the `Done` screen (08-done.png).

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, GenerationCost,
    LanguagePair, WordCandidate,
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
    for nanos in [60_000_000, 150_000_000, 240_000_000, 321_000_000] {
        picture = picture.attempted_with(GenerationCost::from_nanos(nanos));
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

fn priced_file(name: &str, nanos: u64) -> ArtifactFile {
    ArtifactFile::new(name, PathBuf::from(format!("/tmp/{name}")), "1 B", false)
        .with_cost(GenerationCost::from_nanos(nanos))
}

fn priced_published() -> App {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(priced_file("scene.json", 2_000_000)),
        ArtifactSlot::fresh(Artifact::Picture)
            .succeeded_with(priced_file("picture.jpg", 67_300_000)),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(priced_file("audio.wav", 10_000_000)),
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
    let new_cards = rendered.find("[Esc] new cards").unwrap_or(usize::MAX);
    let quit = rendered.find("[Ctrl+C]").unwrap_or(usize::MAX);
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("APKG")
            && rendered.contains("PDF")
            && rendered.contains("[Esc] new cards")
            && rendered.contains("[Ctrl+C]")
            && rendered.contains("RU → EN")
            && new_cards < quit,
        "Done must show APKG/PDF labels, new-cards and quit hints, and the language chip"
    );
}

#[test]
fn armed_done_screen_asks_for_escape_again() {
    let rendered = flat(&published().with_new_batch_pending(true));
    assert!(
        rendered.contains("[Esc] again") && !rendered.contains("new cards"),
        "armed Done must make the second Escape confirmation visible: {rendered}"
    );
}

#[test]
fn starting_a_new_batch_clears_finished_work_but_keeps_the_language_direction() {
    let reset = failed_published()
        .seeded_blob("old words")
        .understood(vec![WordCandidate::new("wreck", "damage", true)])
        .with_quit_pending(true)
        .with_new_batch_pending(true)
        .starting_new_batch();
    assert_eq!(
        (
            reset.screen(),
            reset.pair().learning(),
            reset.pair().known(),
            reset.learning_pending(),
            reset.blob(),
            reset.candidates().len(),
            reset.cards().len(),
            reset.cards_failed(),
            reset.done_artifacts().deck.as_str(),
            reset.quit_pending(),
            reset.new_batch_pending(),
        ),
        (
            Screen::YourWords,
            "en",
            "ru",
            true,
            "",
            0,
            0,
            0,
            "",
            false,
            false,
        ),
        "a new batch retained finished cards or discarded the user's language direction"
    );
}

#[test]
fn done_screen_shortens_the_home_directory_in_its_folder_path() {
    let home = dirs::home_dir().expect("test user home must be available");
    let output = home.join("Documents").join("Kamishibai");
    let app = published().done_published(
        output.join("deck.apkg").to_string_lossy(),
        output.join("cards.pdf").to_string_lossy(),
        output.to_string_lossy(),
    );
    let rendered = flat(&app);
    assert!(
        rendered.contains("~/Documents/Kamishibai")
            && !rendered.contains(output.to_string_lossy().as_ref()),
        "Done did not shorten its output folder to a compact home-relative path: {rendered}"
    );
}

#[test]
fn a_long_output_path_does_not_displace_later_done_rows() {
    let output = format!("/mnt/{}", "very-long-output-segment/".repeat(12));
    let rendered = flat(&published().done_published(
        format!("{output}deck.apkg"),
        format!("{output}cards.pdf"),
        output,
    ));
    assert!(
        rendered.contains("APKG") && rendered.contains("PDF") && rendered.contains("[Ctrl+C]"),
        "a long output folder displaced the remaining Done layout: {rendered}"
    );
}

#[test]
fn done_footer_shows_simplified_subdollar_total() {
    let rendered = flat(&priced_published());
    assert!(
        rendered.contains("$0.08") && !rendered.contains("total cost"),
        "Done footer must show only a cent-rounded dollar total: {rendered}"
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
fn done_footer_includes_terminal_failure_spend() {
    let rendered = flat(&failed_published());
    assert!(
        rendered.contains("$0.32"),
        "Done footer omitted the Gemini spend from a terminally failed artifact: {rendered}"
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
