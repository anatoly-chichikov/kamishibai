//! Session engine integration: scene → picture → sound queue with retries.
//!
//! All artifact production is mocked. No Gemini, no disk, no network.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactProducer, CardDraft, CardPayload, EngineEvent, LanguagePair,
    SessionEngine,
};

fn draft(term: &str) -> CardDraft {
    CardDraft::new(
        term,
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", term),
    )
}

struct AlwaysReady;

impl ArtifactProducer for AlwaysReady {
    fn produce(&mut self, draft: &CardDraft, artifact: Artifact) -> Result<ArtifactFile> {
        Ok(file(draft, artifact))
    }
}

struct FailFirstTwoScenes {
    calls: HashMap<String, u8>,
}

impl FailFirstTwoScenes {
    fn new() -> Self {
        Self {
            calls: HashMap::new(),
        }
    }
}

impl ArtifactProducer for FailFirstTwoScenes {
    fn produce(&mut self, draft: &CardDraft, artifact: Artifact) -> Result<ArtifactFile> {
        if artifact != Artifact::Scene {
            return Ok(file(draft, artifact));
        }
        let count = self.calls.entry(String::from(draft.term())).or_insert(0);
        *count += 1;
        if *count <= 2 {
            return Err(anyhow!("scene producer transient error"));
        }
        Ok(file(draft, artifact))
    }
}

struct FailSceneAlways;

impl ArtifactProducer for FailSceneAlways {
    fn produce(&mut self, draft: &CardDraft, artifact: Artifact) -> Result<ArtifactFile> {
        if artifact == Artifact::Scene {
            return Err(anyhow!("scene blocked"));
        }
        Ok(file(draft, artifact))
    }
}

fn file(draft: &CardDraft, artifact: Artifact) -> ArtifactFile {
    ArtifactFile::new(
        format!("{}-{}.txt", draft.term(), artifact.label()),
        "1 B",
        false,
    )
}

#[test]
fn happy_path_produces_each_artifact_in_order_and_reports_batch_ready() {
    let mut engine = SessionEngine::start(vec![draft("whilst"), draft("wreck")]);
    let mut producer = AlwaysReady;
    let mut events: Vec<EngineEvent> = Vec::new();
    for _ in 0..10 {
        match engine.advance(&mut producer) {
            Some(EngineEvent::BatchReady) => {
                events.push(EngineEvent::BatchReady);
                break;
            }
            Some(event) => events.push(event),
            None => break,
        }
    }
    let kinds: Vec<Artifact> = events
        .iter()
        .take(6)
        .filter_map(|event| match event {
            EngineEvent::ArtifactReady { artifact, .. } => Some(*artifact),
            _ => None,
        })
        .collect();
    assert_eq!(
        (events.last(), kinds.len(), kinds[0], kinds[1], kinds[2]),
        (
            Some(&EngineEvent::BatchReady),
            6,
            Artifact::Scene,
            Artifact::Picture,
            Artifact::Sound,
        ),
        "engine must produce scene → picture → sound per card and end with BatchReady"
    );
}

#[test]
fn transient_scene_failures_retry_up_to_three_times_before_moving_on() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    let mut producer = FailFirstTwoScenes::new();
    let mut events: Vec<EngineEvent> = Vec::new();
    for _ in 0..20 {
        match engine.advance(&mut producer) {
            Some(EngineEvent::BatchReady) => {
                events.push(EngineEvent::BatchReady);
                break;
            }
            Some(event) => events.push(event),
            None => break,
        }
    }
    let retries = events
        .iter()
        .filter(|event| matches!(event, EngineEvent::RetryStarted { .. }))
        .count();
    assert_eq!(
        (retries, events.last()),
        (2, Some(&EngineEvent::BatchReady)),
        "two transient failures must raise two RetryStarted events before the scene finally succeeds"
    );
}

#[test]
fn terminal_failure_after_three_attempts_closes_batch_with_failure_summary() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    let mut producer = FailSceneAlways;
    let mut events: Vec<EngineEvent> = Vec::new();
    for _ in 0..20 {
        match engine.advance(&mut producer) {
            Some(EngineEvent::BatchReady) => break,
            Some(event) => events.push(event.clone()),
            None => break,
        }
        if let Some(EngineEvent::BatchDone { .. }) = events.last() {
            break;
        }
    }
    let exhausted = events
        .iter()
        .any(|event| matches!(event, EngineEvent::RetryExhausted { .. }));
    assert_eq!(
        (exhausted, events.last()),
        (true, Some(&EngineEvent::BatchDone { failed_cards: 1 })),
        "three failed attempts must raise RetryExhausted and then BatchDone with the failed card counted"
    );
}
