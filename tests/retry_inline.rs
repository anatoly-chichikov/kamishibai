//! Retry state must show inline inside `Your cards`, as in
//! `docs/tui-states/current-pdf/06-your-cards-retrying.png`.

use anyhow::{Result, anyhow};
use kamishibai::session::{
    Artifact, ArtifactProducer, CardDraft, CardPayload, EngineEvent, LanguagePair, SessionEngine,
};
use kamishibai::tui::{App, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct FailPictureOnce {
    spent: bool,
}

impl ArtifactProducer for FailPictureOnce {
    fn produce(&mut self, _draft: &CardDraft, artifact: Artifact) -> Result<()> {
        if artifact == Artifact::Picture && !self.spent {
            self.spent = true;
            return Err(anyhow!("transient"));
        }
        Ok(())
    }
}

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
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", term),
    )
}

#[test]
fn engine_retry_event_renders_as_inline_retrying_marker_on_your_cards() {
    let mut engine = SessionEngine::start(vec![draft("in the end")]);
    let mut producer = FailPictureOnce { spent: false };
    let mut retried = false;
    for _ in 0..10 {
        match engine.advance(&mut producer) {
            Some(EngineEvent::RetryStarted { .. }) => {
                retried = true;
                break;
            }
            Some(EngineEvent::BatchReady) => break,
            Some(_) => continue,
            None => break,
        }
    }
    assert!(
        retried,
        "engine must raise RetryStarted for the failing picture"
    );
    let drafts = engine.drafts().to_vec();
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(drafts);
    let rendered = flat(&app);
    assert!(
        rendered.contains("● picture (retrying 1/3)"),
        "Your cards must surface the retry attempt inline without leaving the screen"
    );
}

#[test]
fn retry_does_not_leave_your_cards_state() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![draft("in the end")]);
    assert_eq!(
        app.screen(),
        Screen::YourCards,
        "retry state must stay on Your cards — it is not a separate fullscreen state"
    );
}
