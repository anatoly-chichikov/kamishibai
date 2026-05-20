//! Session engine integration: meta → scene → picture → sound queue with retries.
//!
//! No Gemini, no disk, no network. Tests drive the engine directly via
//! `next_target` + `applied_*` instead of the old producer trait.

use std::collections::HashMap;

use anyhow::anyhow;
use kamishibai::session::{
    Artifact, ArtifactFile, CardDraft, CardMeta, EngineEvent, LanguagePair, SessionEngine,
};

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

fn file_for(draft: &CardDraft, artifact: Artifact) -> ArtifactFile {
    let name = format!("{}-{}.txt", draft.term(), artifact.label());
    let path = std::env::temp_dir().join(&name);
    ArtifactFile::new(name, path, "1 B", false)
}

fn run(
    engine: &mut SessionEngine,
    mut step: impl FnMut(usize, Artifact) -> StepOutcome,
) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    for _ in 0..32 {
        if let Some((card, kind)) = engine.next_target() {
            let outcome = step(card, kind);
            let event = match (kind, outcome) {
                (Artifact::Meta, StepOutcome::MetaOk(meta)) => {
                    engine.applied_meta(card, Ok((meta, None)))
                }
                (Artifact::Meta, StepOutcome::Fail) => {
                    engine.applied_meta(card, Err(anyhow!("transient")))
                }
                (kind, StepOutcome::MediaOk(file)) => engine.applied_media(card, kind, Ok(file)),
                (kind, StepOutcome::Fail) => {
                    engine.applied_media(card, kind, Err(anyhow!("transient")))
                }
                _ => unreachable!(),
            };
            events.push(event);
            continue;
        }
        if let Some(event) = engine.batch_state() {
            events.push(event);
            return events;
        }
        return events;
    }
    events
}

enum StepOutcome {
    MetaOk(CardMeta),
    MediaOk(ArtifactFile),
    Fail,
}

#[test]
fn happy_path_produces_each_artifact_in_order_and_reports_batch_ready() {
    let mut engine = SessionEngine::start(vec![draft("whilst"), draft("wreck")]);
    let drafts: Vec<CardDraft> = engine.drafts().to_vec();
    let events = run(&mut engine, |card, artifact| match artifact {
        Artifact::Meta => StepOutcome::MetaOk(meta_for(drafts[card].term())),
        kind => StepOutcome::MediaOk(file_for(&drafts[card], kind)),
    });
    let kinds: Vec<Artifact> = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ArtifactReady { artifact, .. } => Some(*artifact),
            _ => None,
        })
        .collect();
    assert_eq!(
        (
            events.last(),
            kinds.len(),
            kinds[0],
            kinds[1],
            kinds[2],
            kinds[3]
        ),
        (
            Some(&EngineEvent::BatchReady),
            8,
            Artifact::Meta,
            Artifact::Sound,
            Artifact::Scene,
            Artifact::Picture,
        ),
        "engine must produce meta → sound → scene → picture per card and end with BatchReady"
    );
}

#[test]
fn transient_scene_failures_retry_up_to_three_times_before_moving_on() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    let drafts: Vec<CardDraft> = engine.drafts().to_vec();
    let mut scene_calls: HashMap<String, u8> = HashMap::new();
    let events = run(&mut engine, |card, artifact| match artifact {
        Artifact::Meta => StepOutcome::MetaOk(meta_for(drafts[card].term())),
        Artifact::Scene => {
            let count = scene_calls
                .entry(String::from(drafts[card].term()))
                .or_insert(0);
            *count += 1;
            if *count <= 2 {
                StepOutcome::Fail
            } else {
                StepOutcome::MediaOk(file_for(&drafts[card], Artifact::Scene))
            }
        }
        kind => StepOutcome::MediaOk(file_for(&drafts[card], kind)),
    });
    let retries = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                EngineEvent::RetryStarted {
                    artifact: Artifact::Scene,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        (retries, events.last()),
        (2, Some(&EngineEvent::BatchReady)),
        "two transient scene failures must raise two RetryStarted events before scene finally succeeds"
    );
}

#[test]
fn terminal_scene_failure_discards_picture_and_completes_remaining() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    let drafts: Vec<CardDraft> = engine.drafts().to_vec();
    let events = run(&mut engine, |card, artifact| match artifact {
        Artifact::Meta => StepOutcome::MetaOk(meta_for(drafts[card].term())),
        Artifact::Scene => StepOutcome::Fail,
        kind => StepOutcome::MediaOk(file_for(&drafts[card], kind)),
    });
    let exhausted = events.iter().any(|event| {
        matches!(
            event,
            EngineEvent::RetryExhausted {
                artifact: Artifact::Scene,
                ..
            }
        )
    });
    let picture_discarded = engine.drafts()[0].artifacts().picture().discarded();
    assert_eq!(
        (exhausted, picture_discarded, events.last()),
        (
            true,
            true,
            Some(&EngineEvent::BatchDone { failed_cards: 1 })
        ),
        "terminal scene failure must discard picture and emit BatchDone with the failed card counted"
    );
}
