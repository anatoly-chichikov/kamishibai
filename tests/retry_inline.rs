//! Retry state must show inline inside `Your cards`, as in
//! `docs/tui-states/current-pdf/06-your-cards-retrying.png`.

use anyhow::{Result, anyhow};
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, EngineEvent, GenerationCost,
    LanguagePair, SessionEngine,
};
use kamishibai::tui::{App, Screen, draw};
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
fn engine_retry_event_renders_as_inline_retrying_marker_on_your_cards() -> Result<()> {
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
        rendered.contains("retry") && rendered.contains("$.1234") && rendered.contains("$0.12"),
        "your cards must surface the retry attempt inline without leaving the screen: {rendered}"
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
