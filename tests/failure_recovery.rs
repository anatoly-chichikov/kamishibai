//! Recovery flow for failed cards (`07-your-cards-couldnt-finish.png`).

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, GenerationCost, LanguagePair,
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
    for nanos in [60_000_000, 150_000_000, 240_000_000, 321_000_000] {
        picture = picture.attempted_with(GenerationCost::from_nanos(nanos));
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn exhausted(slot: ArtifactSlot) -> ArtifactSlot {
    let mut spent = slot;
    while !spent.failed_terminally() {
        spent = spent.attempted();
    }
    spent
}

fn failed_meta() -> CardArtifacts {
    CardArtifacts::from_parts(
        exhausted(ArtifactSlot::fresh(Artifact::Meta)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn failed_scene() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        exhausted(ArtifactSlot::fresh(Artifact::Scene)),
        ArtifactSlot::fresh(Artifact::Picture).discard(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn failed_sound() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        exhausted(ArtifactSlot::fresh(Artifact::Sound)),
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

fn seeded() -> App {
    seeded_with(failed_picture())
}

fn seeded_with(artifacts: CardArtifacts) -> App {
    let draft = CardDraft::new(
        "wreck",
        "verb sense — destroyed vehicle",
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for("wreck"), None)
    .with_artifacts(artifacts);
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(vec![draft])
}

#[test]
fn your_cards_surfaces_failure_banner_when_any_card_fails_terminally() {
    let app = seeded();
    let rendered = flat(&app);
    let manga = rendered
        .lines()
        .find(|line| line.contains("✗ manga"))
        .expect("the failed artifact must own a row");
    assert!(
        !manga.contains("unfinished"),
        "your cards must mark the failed manga row with a bare ✗: {rendered}"
    );
}

#[test]
fn ctrl_g_emits_regenerate_cards_when_any_card_failed_terminally() {
    let app = seeded();
    let (_, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        side,
        Side::RegenerateCards,
        "Ctrl+G on Your cards must regenerate the current card state even when failures are visible"
    );
}

#[test]
fn lowercase_r_is_inert_even_with_failures_present() {
    let app = seeded();
    let (after, side) = transit(app, AppEvent::KeyChar('r'));
    assert!(
        !after.card_expanded() && after.sentence_editor().is_none() && side == Side::None,
        "lowercase r opened the sentence editor or emitted an action after the shortcut was removed"
    );
}

#[test]
fn uppercase_r_is_inert_even_with_failures_present() {
    let app = seeded();
    let (after, side) = transit(app, AppEvent::KeyChar('R'));
    assert!(
        !after.card_expanded() && after.sentence_editor().is_none() && side == Side::None,
        "uppercase R opened the sentence editor or emitted an action after the shortcut was removed"
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
            card.artifacts().picture().cost(),
            card.artifacts().sound().ready(),
        ),
        (
            true,
            false,
            0,
            Some(GenerationCost::from_nanos(321_000_000)),
            false,
        ),
        "regenerating failed cards must reset retry state without erasing already billed spend"
    );
}

#[test]
fn regenerate_meta_failure_resets_all_meta_dependents() {
    let app = seeded_with(failed_meta());
    let recovered = app.cards_reset_failures();
    let card = &recovered.cards()[0];
    assert_eq!(
        (
            card.artifacts().meta().failed_terminally(),
            card.artifacts().scene().ready(),
            card.artifacts().picture().ready(),
            card.artifacts().sound().ready(),
        ),
        (false, false, false, false),
        "regenerating a failed meta must reset meta and all generated media"
    );
}

#[test]
fn regenerate_scene_failure_resets_picture_but_keeps_sound() {
    let app = seeded_with(failed_scene());
    let recovered = app.cards_reset_failures();
    let card = &recovered.cards()[0];
    assert_eq!(
        (
            card.artifacts().meta().ready(),
            card.artifacts().scene().failed_terminally(),
            card.artifacts().picture().discarded(),
            card.artifacts().sound().ready(),
        ),
        (true, false, false, true),
        "regenerating a failed scene must reset scene and picture while keeping independent artifacts"
    );
}

#[test]
fn regenerate_sound_failure_keeps_ready_visual_artifacts() {
    let app = seeded_with(failed_sound());
    let recovered = app.cards_reset_failures();
    let card = &recovered.cards()[0];
    assert_eq!(
        (
            card.artifacts().meta().ready(),
            card.artifacts().scene().ready(),
            card.artifacts().picture().ready(),
            card.artifacts().sound().failed_terminally(),
        ),
        (true, true, true, false),
        "regenerating a failed sound must keep ready visual artifacts"
    );
}

#[test]
fn regenerate_failed_clears_stale_done_artifacts() {
    let app = seeded().done_published("old.apkg", "old.pdf", "old-out");
    let recovered = app.cards_reset_failures();
    assert_eq!(
        recovered.done_artifacts().deck.as_str(),
        "",
        "regenerating failed cards must clear stale published outputs"
    );
}

#[test]
fn recovery_keeps_the_user_on_your_cards() {
    let app = seeded();
    let (after, _) = transit(app, AppEvent::Generate);
    assert_eq!(
        after.screen(),
        Screen::YourCards,
        "regenerating failed cards must stay inline on Your cards"
    );
}

#[test]
fn enter_on_failure_banner_toggles_expansion_without_leaving_your_cards() {
    let app = seeded();
    let (after, side) = transit(app, AppEvent::KeyEnter);
    assert_eq!(
        (after.screen(), after.card_expanded(), side),
        (Screen::YourCards, true, Side::None),
        "Enter on the failure banner must just toggle expansion and stay on YourCards"
    );
}

#[test]
fn escape_closes_an_expanded_failure_before_reaching_the_batch_lifecycle() {
    let opened = transit(seeded(), AppEvent::KeyEnter).0;
    let (closed, side) = transit(opened.clone(), AppEvent::Cancel);
    assert_eq!(
        (
            closed.screen(),
            opened.card_expanded(),
            opened.sentence_editor().is_none(),
            closed.card_expanded(),
            side,
        ),
        (Screen::YourCards, true, true, false, Side::None),
        "Escape on an expanded failure bypassed the card's own disclosure and reached the batch lifecycle"
    );
}
