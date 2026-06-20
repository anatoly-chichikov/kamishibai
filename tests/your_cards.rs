//! Integration render tests for the `Your cards` screen (04-your-cards.png)
//! and its two inline variants (retry, failure).

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, GenerationCost,
    LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 50);
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

fn selected_label_highlighted(app: &App, needle: &str) -> bool {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = rendered[..start].chars().count() as u16;
            return (0..needle.chars().count()).all(|offset| {
                buffer[(column + offset as u16, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f)
            });
        }
    }
    false
}

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn priced_file(name: &str, nanos: u64) -> ArtifactFile {
    ArtifactFile::new(name, PathBuf::from(format!("/tmp/{name}")), "1 B", false)
        .with_cost(GenerationCost::from_nanos(nanos))
}

fn priced_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(priced_file("scene.json", 2_000_000)),
        ArtifactSlot::fresh(Artifact::Picture)
            .succeeded_with(priced_file("picture.jpg", 67_300_000)),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(priced_file("audio.wav", 10_000_000)),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture).attempted();
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn failed_artifacts() -> CardArtifacts {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for _ in 0..3 {
        picture = picture.attempted();
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn meta_for(term: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source sentence with {term}"),
        term,
        format!("vivid hint for {term}"),
        format!("usage notes for {term}"),
        format!("Example with {term}."),
    )
}

fn draft(term: &str, artifacts: CardArtifacts) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for(term), None)
    .with_artifacts(artifacts)
}

fn seeded(drafts: Vec<CardDraft>) -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(drafts)
}

#[test]
fn your_cards_lists_each_card_with_term_meta_preview_head_and_artifact_check_marks() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("building your cards")
            && rendered.contains("2/4 ready")
            && rendered.contains("whilst → Example with whilst.")
            && rendered.contains("at the end → Example with at the end.")
            && rendered.contains("in the end → Example with in the end.")
            && rendered.contains("wreck → Example with wreck.")
            && rendered.contains("✓ scene")
            && rendered.contains("✓ picture")
            && rendered.contains("RU → EN")
            && rendered.contains("[↑↓] nav")
            && rendered.contains("[Enter] expand")
            && rendered.contains("[R] change")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[D] drop")
            && !rendered.contains("working…")
            && !rendered.contains("queued"),
        "each generated card must reveal its meta sentence on the head row right after the term: {rendered}"
    );
}

#[test]
fn your_cards_done_footer_carries_expand_change_and_regenerate_hints() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", ready_artifacts()),
        draft("wreck", ready_artifacts()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("[↑↓] nav")
            && rendered.contains("[Enter] expand")
            && rendered.contains("[R] change")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("new batch")
            && !rendered.contains("[D] drop")
            && !rendered.contains("working…"),
        "all-done footer must offer expand/change/regenerate hints and no new-batch or drop hooks: {rendered}"
    );
}

#[test]
fn your_cards_shows_card_asset_and_total_costs_when_finished() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("whilst → Example with whilst.  $.0808")
            && rendered.contains("meta.json          1 B  $.0015")
            && rendered.contains("audio.wav          1 B  $.0100")
            && rendered.contains("scene.json         1 B  $.0020")
            && rendered.contains("picture.jpg        1 B  $.0673")
            && rendered.contains("total cost $.0808"),
        "finished cards must show per-card, per-asset, and total Gemini costs: {rendered}"
    );
}

#[test]
fn selected_card_cost_keeps_the_row_highlight_background() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    assert!(
        selected_label_highlighted(&app, "  $.0808"),
        "selected card cost must not punch a dark gap through the row highlight"
    );
}

#[test]
fn ctrl_g_on_ready_cards_requests_regeneration() {
    let app = seeded(vec![draft("whilst", ready_artifacts())]);
    let (_, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        side,
        Side::RegenerateCurrent,
        "Ctrl+G on ready cards must request regeneration so publish can be rebuilt"
    );
}

#[test]
fn untouched_card_shows_only_a_dim_term_with_no_step_rows() {
    let single = CardDraft::new(
        "ancient",
        "understanding for ancient",
        LanguagePair::new("en", "ru"),
    );
    let app = seeded(vec![single]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("ancient")
            && !rendered.contains("meta")
            && !rendered.contains("audio")
            && !rendered.contains("scene")
            && !rendered.contains("picture")
            && !rendered.contains("queued"),
        "untouched card must collapse to its term row alone, no artifact lines: {rendered}"
    );
}

#[test]
fn retry_state_shows_retrying_count_inline_inside_the_card_row() {
    let app = seeded(vec![draft("in the end", retrying_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("retry 2/3") || rendered.contains("retry 1/3"),
        "retrying state must be rendered inline without leaving the your cards screen: {rendered}"
    );
}

#[test]
fn failure_banner_appears_when_any_card_exhausts_its_retries() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("gave up") && rendered.contains("✗") && rendered.contains("picture"),
        "your cards must mark the card as `gave up` and show the ✗ on the failed step: {rendered}"
    );
}

#[test]
fn arrows_and_enter_navigate_and_toggle_expansion_of_the_focused_card() {
    let start = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
    ]);
    let after_down = transit(start.clone(), AppEvent::NavNext).0;
    let expanded = transit(after_down.clone(), AppEvent::KeyEnter).0;
    assert_eq!(
        (
            start.card_selected(),
            after_down.card_selected(),
            start.card_expanded(),
            expanded.card_expanded(),
        ),
        (0, 1, false, true),
        "arrows must move the cursor and Enter must toggle expansion on the focused card"
    );
}

#[test]
fn expanding_a_card_keeps_the_existing_scroll_position() {
    let start = seeded(
        ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(|term| draft(term, ready_artifacts()))
            .collect(),
    );
    let selected = (0..5).fold(start, |app, _| {
        transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(8, 120)
    });
    let before = selected.body_scroll();
    let expanded = transit(selected, AppEvent::KeyEnter)
        .0
        .body_scroll_to_selection(8, 120);
    assert_eq!(
        expanded.body_scroll(),
        before,
        "expanding a visible card must not jump the scroll to the bottom of the detail pane"
    );
}

#[test]
fn expanded_card_shows_meta_preview_only_no_duplicate_artifact_pane() {
    let start = seeded(vec![draft("whilst", ready_artifacts())]);
    let expanded = transit(start, AppEvent::KeyEnter).0;
    let rendered = flat(&expanded);
    let artifact_lines = rendered.matches("scene").count();
    assert!(
        rendered.contains("target")
            && rendered.contains("source")
            && rendered.contains("hint")
            && rendered.contains("meaning")
            && artifact_lines <= 1,
        "expanded row must reveal the meta preview without duplicating the step list: {rendered}"
    );
}
