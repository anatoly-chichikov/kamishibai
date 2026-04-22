//! Integration render tests for the `Your cards` screen (04-your-cards.png)
//! and its two inline variants (retry, failure).

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardPayload, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 40);
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

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture).attempted();
    CardArtifacts::from_parts(
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
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn draft(term: &str, artifacts: CardArtifacts) -> CardDraft {
    CardDraft::new(
        term,
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", term),
    )
    .with_artifacts(artifacts)
}

fn seeded(drafts: Vec<CardDraft>) -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(drafts)
}

#[test]
fn your_cards_lists_each_card_with_artifact_check_marks_and_status_summary() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("Your cards")
            && rendered.contains("group 1 of 1")
            && rendered.contains("2/4 ready")
            && rendered.contains("whilst")
            && rendered.contains("at the end")
            && rendered.contains("in the end")
            && rendered.contains("wreck")
            && rendered.contains("✓ scene")
            && rendered.contains("✓ picture")
            && rendered.contains("○ picture")
            && rendered.contains("kamishibai · EN → RU"),
        "Your cards must render every draft, its artifact check marks, and the `ready/total` status"
    );
}

#[test]
fn retry_state_shows_retrying_count_inline_inside_the_card_row() {
    let app = seeded(vec![draft("in the end", retrying_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("↻ picture 1/3"),
        "retrying state must be rendered inline without leaving the Your cards screen"
    );
}

#[test]
fn failure_banner_appears_when_any_card_exhausts_its_retries() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("1 card couldn't finish after 3 tries")
            && rendered.contains("[r] regenerate failed")
            && rendered.contains("✗ picture"),
        "Your cards must surface the failure banner and the recovery key when a card fails terminally"
    );
}

#[test]
fn arrows_and_enter_navigate_and_toggle_expansion_of_the_focused_card() {
    let start = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
    ]);
    let after_down = transit(start.clone(), AppEvent::NavNext).0;
    let expanded = transit(after_down.clone(), AppEvent::Submit).0;
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
fn expanded_card_shows_front_back_and_change_this_card_hint() {
    let start = seeded(vec![draft("whilst", ready_artifacts())]);
    let expanded = transit(start, AppEvent::Submit).0;
    let rendered = flat(&expanded);
    assert!(
        rendered.contains("── front")
            && rendered.contains("── back")
            && rendered.contains("── files")
            && rendered.contains("[R]")
            && rendered.contains("change this card")
            && rendered.contains("drop picture / scene / sound"),
        "the expanded row must reveal front/back sections, files list, and the per-card editor hint"
    );
}
