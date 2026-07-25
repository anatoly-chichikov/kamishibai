//! Session engine integration: meta → scene → picture → sound queue with retries.
//!
//! No Gemini, no disk, no network. Tests drive the engine directly via
//! `next_target` + `applied_*` instead of the old producer trait.

use std::collections::HashMap;

use anyhow::anyhow;
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, EngineEvent, GenerationCost,
    LanguagePair, SessionEngine,
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

#[test]
fn metered_media_retries_keep_cumulative_cost_and_success_does_not_double_count_it() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    engine.applied_media_attempt(
        0,
        Artifact::Picture,
        ArtifactAttempt::new(
            Err(anyhow!("first failure")),
            Some(GenerationCost::from_nanos(120_000_000)),
        ),
    );
    let first = engine.drafts()[0].artifacts().picture().cost();
    engine.applied_media_attempt(
        0,
        Artifact::Picture,
        ArtifactAttempt::unmetered(Err(anyhow!("unmetered failure"))),
    );
    let second = engine.drafts()[0].artifacts().picture().cost();
    let file = file_for(&engine.drafts()[0], Artifact::Picture)
        .with_cost(GenerationCost::from_nanos(220_000_000));
    engine.applied_media_attempt(
        0,
        Artifact::Picture,
        ArtifactAttempt::new(Ok(file), Some(GenerationCost::from_nanos(220_000_000))),
    );
    assert_eq!(
        (
            first,
            second,
            engine.drafts()[0].artifacts().picture().cost(),
        ),
        (
            Some(GenerationCost::from_nanos(120_000_000)),
            Some(GenerationCost::from_nanos(120_000_000)),
            Some(GenerationCost::from_nanos(340_000_000)),
        ),
        "media retry accounting lost, reset, or double-counted cumulative spend"
    );
}

#[test]
fn terminal_media_failure_keeps_the_last_cumulative_cost() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    for nanos in [90_000_000, 120_000_000, 180_000_000, 60_000_000] {
        engine.applied_media_attempt(
            0,
            Artifact::Picture,
            ArtifactAttempt::new(
                Err(anyhow!("picture rejected")),
                Some(GenerationCost::from_nanos(nanos)),
            ),
        );
    }
    let picture = engine.drafts()[0].artifacts().picture();
    assert_eq!(
        (picture.failed_terminally(), picture.cost()),
        (true, Some(GenerationCost::from_nanos(450_000_000)),),
        "terminal media failure discarded its metered Gemini spend"
    );
}

#[test]
fn metered_meta_failure_keeps_its_cumulative_cost() {
    let mut engine = SessionEngine::start(vec![draft("whilst")]);
    engine.applied_meta_attempt(
        0,
        ArtifactAttempt::new(
            Err(anyhow!("meta rejected")),
            Some(GenerationCost::from_nanos(8_000_000)),
        ),
    );
    assert_eq!(
        engine.drafts()[0].artifacts().meta().cost(),
        Some(GenerationCost::from_nanos(8_000_000)),
        "meta retry accounting vanished at the engine boundary"
    );
}
