//! Integration flow for `Your cards -> Change this card -> updated Your cards`.

use anyhow::Result;
use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardCorrection, CardDraft, CardPayload, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, ModalKind, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct FakeCardCorrection;

impl CardCorrection for FakeCardCorrection {
    fn correct_card(
        &self,
        draft: &CardDraft,
        _comment: &str,
        _pair: &LanguagePair,
    ) -> Result<CardDraft> {
        Ok(draft.clone().recomposed(CardPayload::new(
            "updated front",
            "updated back",
            draft.payload().hint(),
            draft.payload().highlight(),
        )))
    }
}

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 28);
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
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn draft(term: &str) -> CardDraft {
    CardDraft::new(
        term,
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", term),
    )
    .with_artifacts(ready())
}

fn seeded() -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![draft("whilst"), draft("wreck")])
}

#[test]
fn request_change_on_your_cards_opens_per_card_modal() {
    let app = seeded();
    let (opened, _) = transit(app, AppEvent::RequestChange);
    let rendered = flat(&opened);
    assert!(
        opened.modal() == Some(ModalKind::ChangeThisCard)
            && rendered.contains("How should I change this card?")
            && rendered.contains("applies to this card only"),
        "R on Your cards must open the per-card modal with the right prompt"
    );
}

#[test]
fn submit_on_per_card_modal_emits_card_correction_and_closes_overlay() {
    let app = seeded()
        .with_modal(ModalKind::ChangeThisCard)
        .typed('v')
        .typed('e')
        .typed('r')
        .typed('b');
    let (after, side) = transit(app, AppEvent::Submit);
    assert_eq!(
        (after.modal(), side),
        (None, Side::RunCardCorrection(String::from("verb"))),
        "Enter on Change this card must emit RunCardCorrection with the typed buffer"
    );
}

#[test]
fn correction_result_reaches_focused_card_without_touching_neighbors() {
    let app = seeded();
    let focused = app.cards()[app.card_selected()].clone();
    let updated = FakeCardCorrection
        .correct_card(&focused, "verb", app.pair())
        .expect("mock card correction");
    let patched = app.card_patched_artifacts(ready());
    let with_updated = patched.clone().cards_started({
        let mut drafts = patched.cards().to_vec();
        drafts[0] = updated;
        drafts
    });
    let rendered = flat(&with_updated.card_toggle_expanded());
    assert!(
        rendered.contains("updated front")
            && rendered.contains("updated back")
            && rendered.contains("wreck"),
        "per-card correction must update only the focused draft and leave siblings intact"
    );
}

#[test]
fn escape_on_per_card_modal_closes_without_running_correction() {
    let app = seeded().with_modal(ModalKind::ChangeThisCard).typed('x');
    let (after, side) = transit(app, AppEvent::Cancel);
    assert_eq!(
        (after.modal(), side),
        (None, Side::None),
        "Esc must close the per-card modal without emitting a correction side-effect"
    );
}
