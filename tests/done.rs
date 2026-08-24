//! Integration flow for the `Done` screen (08-done.png).

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, GenerationCost,
    LanguagePair, WordCandidate,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

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

fn buffer_of(app: &App) -> Buffer {
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

fn row_of(buffer: &Buffer, token: &str) -> u16 {
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if line.contains(token) {
            return row;
        }
    }
    panic!("invariant: {token} must be rendered somewhere on the screen");
}

fn style_of(app: &App, token: &str) -> (Color, Color, Modifier) {
    let buffer = buffer_of(app);
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(byte) = line.find(token) {
            let column = u16::try_from(line[..byte].chars().count()).expect("column must fit");
            let cell = &buffer[(column, row)];
            return (cell.fg, cell.bg, cell.modifier);
        }
    }
    panic!("invariant: {token} must be rendered somewhere on the screen");
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
    let rendered = flat(&failed_published().with_new_batch_pending(true));
    assert!(
        rendered.contains("[Esc] again")
            && !rendered.contains("new cards")
            && !rendered.contains("[Ctrl+G] regenerate"),
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
        rendered.contains("[Ctrl+G] regenerate"),
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

#[test]
fn done_reports_the_batch_in_the_same_styles_as_the_cards_screen() {
    let done = priced_published();
    let cards = priced_published().with_screen(Screen::YourCards);
    assert_eq!(
        (
            style_of(&done, "1/1"),
            style_of(&done, " ready"),
            style_of(&done, "step 3/3"),
            style_of(&done, "$0."),
        ),
        (
            style_of(&cards, "0/0"),
            style_of(&cards, " ready"),
            style_of(&cards, "step 3/3"),
            style_of(&cards, "$0."),
        ),
        "one batch must not report itself in two different weights on its two final views"
    );
}

#[test]
fn a_reopened_done_row_lights_its_term_without_any_artifacts_to_ask() {
    let reopened = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::Done)
        .confirmed_learning("en")
        .cards_started(vec![CardDraft::new(
            "wreck",
            "understanding for wreck",
            LanguagePair::new("en", "ru"),
        )])
        .done_published_counted(
            String::from("en_2026-04-17_183029.apkg"),
            String::from("en_2026-04-17_183029.pdf"),
            String::from("kamishibai-out/"),
            1,
            0,
        );
    assert_eq!(
        style_of(&reopened, "wreck").0,
        Color::Rgb(0xe6, 0xe3, 0xda),
        "a reopened session carries no artifact slots, so asking them left every shipped card reading as unbuilt"
    );
}

#[test]
fn a_built_done_row_lights_its_term_while_a_broken_one_stays_quiet() {
    let built = style_of(&priced_published(), "wreck");
    assert_eq!(
        (built.0, built.2, style_of(&failed_published(), "wreck").0),
        (
            Color::Rgb(0xe6, 0xe3, 0xda),
            Modifier::empty(),
            Color::Rgb(0x5a, 0x59, 0x53)
        ),
        "the done list must mark a finished card by ink alone, the same way the cards screen does"
    );
}

#[test]
fn a_body_without_a_cursor_carries_no_weight_at_all() {
    let buffer = buffer_of(&priced_published());
    let header = row_of(&buffer, "your cards");
    let weighted = (0..buffer.area.height)
        .filter(|row| *row != header)
        .flat_map(|row| (0..buffer.area.width).map(move |column| (column, row)))
        .filter(|position| buffer[*position].modifier.contains(Modifier::BOLD))
        .count();
    assert_eq!(
        weighted, 0,
        "the done body has no cursor to carry, so nothing below its header may be weighted"
    );
}
