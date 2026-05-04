//! Integration render tests for the `Your cards` screen (04-your-cards.png)
//! and its two inline variants (retry, failure).

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardBody, CardDraft, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture).attempted();
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
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
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn body_for(term: &str) -> CardBody {
    CardBody::new(
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
    .with_body(body_for(term), None)
    .with_artifacts(artifacts)
}

fn seeded(drafts: Vec<CardDraft>) -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(drafts)
}

#[test]
fn your_cards_lists_each_card_with_a_bare_term_head_and_artifact_check_marks() {
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
            && rendered.contains("whilst")
            && rendered.contains("at the end")
            && rendered.contains("in the end")
            && rendered.contains("wreck")
            && rendered.contains("✓ scene")
            && rendered.contains("✓ picture")
            && rendered.contains("RU → EN")
            && !rendered.contains("Example with"),
        "collapsed view must show only terms in the card head, never leak body sentences: {rendered}"
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
fn expanded_card_shows_body_preview_only_no_duplicate_artifact_pane() {
    let start = seeded(vec![draft("whilst", ready_artifacts())]);
    let expanded = transit(start, AppEvent::Submit).0;
    let rendered = flat(&expanded);
    let artifact_lines = rendered.matches("scene").count();
    assert!(
        rendered.contains("target")
            && rendered.contains("source")
            && rendered.contains("hint")
            && rendered.contains("meaning")
            && artifact_lines <= 1,
        "expanded row must reveal the body preview without duplicating the step list: {rendered}"
    );
}

#[test]
fn lowercase_d_discards_the_first_unfinished_artifact_on_the_focused_card() {
    let start = seeded(vec![draft("in the end", retrying_artifacts())]);
    let next = transit(start, AppEvent::KeyChar('d')).0;
    assert!(
        next.cards()[0].artifacts().picture().discarded(),
        "lowercase d on Your cards must discard the first unfinished focused artifact"
    );
}
