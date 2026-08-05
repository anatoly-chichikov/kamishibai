//! Retry state must stay inside `Your cards` with history summarized by card.

use anyhow::{Result, anyhow};
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, AttemptFault, CardDraft, CardMeta, EngineEvent,
    GenerationCost, LanguagePair, SessionEngine,
};
use kamishibai::tui::{App, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 44);
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

fn draft(term: &str) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
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

fn file_for(term: &str, kind: Artifact) -> ArtifactFile {
    let name = format!("{term}-{}.txt", kind.label());
    let path = std::env::temp_dir().join(&name);
    ArtifactFile::new(name, path, "1 B", false)
}

#[test]
fn engine_retry_event_renders_only_a_card_head_attempt_summary() -> Result<()> {
    let mut engine = SessionEngine::start(vec![draft("in the end")]);
    engine.applied_meta(0, Ok((meta_for("in the end"), None)));
    engine.applied_media(
        0,
        Artifact::Scene,
        Ok(file_for("in the end", Artifact::Scene)),
    );
    let event = engine.applied_media_attempt(
        0,
        Artifact::Picture,
        ArtifactAttempt::new(
            Err(anyhow!("transient")),
            Some(GenerationCost::from_nanos(123_400_000)),
        ),
    );
    assert!(
        matches!(
            event,
            EngineEvent::RetryStarted {
                artifact: Artifact::Picture,
                ..
            }
        ),
        "engine must raise RetryStarted for the failing picture"
    );
    let drafts = engine.drafts().to_vec();
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(drafts);
    let rendered = flat(&app);
    assert!(
        rendered.contains("$.1234  ↻1")
            && !rendered.contains("retry 1/3")
            && !rendered.contains("1 ✗")
            && !rendered.contains("paused")
            && rendered.contains("$0.12"),
        "your cards must summarize retry history once on its card head: {rendered}"
    );
    Ok(())
}

#[test]
fn rejected_attempt_names_its_reason_in_the_expanded_card() -> Result<()> {
    let mut engine = SessionEngine::start(vec![draft("in the end")]);
    engine.applied_meta(0, Ok((meta_for("in the end"), None)));
    engine.applied_media(
        0,
        Artifact::Scene,
        Ok(file_for("in the end", Artifact::Scene)),
    );
    engine.applied_media_attempt(
        0,
        Artifact::Picture,
        ArtifactAttempt::new(Err(anyhow!("rejected")), None).with_fault(AttemptFault::new(
            "border",
            "White border missing on: bottom",
            Some(std::env::temp_dir().join("attempt-0001.jpg")),
        )),
    );
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(engine.drafts().to_vec())
        .card_revealed(0);
    let rendered = flat(&app);
    assert!(
        rendered.contains("rejected attempts")
            && rendered.contains("attempt-0001.jpg")
            && rendered.contains("border · White border missing on: bottom"),
        "the expanded card must name why the picture was rejected and which frame it was: {rendered}"
    );
    Ok(())
}

#[test]
fn retry_does_not_leave_your_cards_state() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(vec![draft("in the end")]);
    assert_eq!(
        app.screen(),
        Screen::YourCards,
        "retry state must stay on Your cards — it is not a separate fullscreen state"
    );
}
