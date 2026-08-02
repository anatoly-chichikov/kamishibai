//! Session engine integration: meta → scene → picture → sound queue with retries.
//!
//! No Gemini, no disk, no network. Tests drive the engine directly via
//! `next_target` + `applied_*` instead of the old producer trait.

use std::collections::HashMap;

use anyhow::anyhow;
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, ArtifactSlot, AxisSet, CardArtifacts, CardDraft,
    CardMeta, CardRevision, EngineEvent, GenerationCost, LanguagePair, Register, SentenceAxis,
    SentenceKind, SentenceLabelSelection, SentenceLabels, SentenceLevel, SessionEngine,
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

fn labeled_meta_for(term: &str) -> CardMeta {
    meta_for(term).with_sentence_labels(SentenceLabels::new(
        Register::Neutral,
        SentenceLevel::B1,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::default(),
    ))
}

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
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

#[test]
fn staged_rewrite_preserves_current_card_until_batch_start() {
    let current = draft("whilst")
        .with_meta(labeled_meta_for("whilst"), None)
        .with_artifacts(ready_artifacts());
    let selection = SentenceLabelSelection::from_labels(
        current
            .meta()
            .and_then(CardMeta::sentence_labels)
            .expect("labeled metadata must expose its baseline"),
    )
    .choosing(SentenceAxis::Register, 2);
    let staged = current.staging_rewrite(selection, "make it official");
    let started = staged.clone().starting_rewrite();
    assert_eq!(
        (
            staged.meta().map(CardMeta::target_sentence),
            staged.artifacts().all_ready(),
            staged
                .rewrite()
                .and_then(|rewrite| rewrite.previous())
                .map(CardMeta::target_sentence),
            started.meta().is_none(),
            started.artifacts().meta().complete(),
            started.artifacts().sound().complete(),
            started.artifacts().scene().complete(),
            started.artifacts().picture().complete(),
            started.rewrite().is_some(),
        ),
        (
            Some("Example with whilst."),
            true,
            Some("Example with whilst."),
            true,
            false,
            false,
            false,
            false,
            true,
        ),
        "staging invalidated the current card early or batch start failed to enqueue its full rewrite"
    );
}

#[test]
fn returning_to_baseline_with_a_blank_note_removes_the_staged_rewrite() {
    let current = draft("whilst")
        .with_meta(
            meta_for("whilst").with_sentence_labels(SentenceLabels::new(
                Register::Neutral,
                SentenceLevel::B1,
                SentenceKind::Statement,
                AxisSet::from_axes([SentenceAxis::Register]),
                AxisSet::from_axes([SentenceAxis::Register]),
            )),
            None,
        )
        .with_artifacts(ready_artifacts());
    let baseline = SentenceLabelSelection::from_labels(
        current
            .meta()
            .and_then(CardMeta::sentence_labels)
            .expect("labeled metadata must expose its baseline"),
    );
    let changed = baseline.choosing(SentenceAxis::Register, 2);
    let staged = current.staging_rewrite(changed, "make it official");
    let same_token = staged
        .clone()
        .staging_rewrite(baseline.choosing(SentenceAxis::Register, 0), " \n ");
    let restored = same_token.clone().staging_rewrite(baseline.clone(), " \n ");
    assert_eq!(
        (
            same_token.rewrite().is_some(),
            same_token
                .rewrite()
                .map(|rewrite| rewrite.selection().approx().is_empty()),
            restored.rewrite(),
            restored.meta().map(CardMeta::target_sentence),
            restored.artifacts().all_ready(),
        ),
        (true, Some(true), None, Some("Example with whilst."), true),
        "auto-unstage ignored baseline pin state or failed after full label restoration"
    );
}

#[test]
fn staged_rewrite_holds_a_ready_engine_until_batch_start() {
    let staged = draft("whilst")
        .with_meta(labeled_meta_for("whilst"), None)
        .with_artifacts(ready_artifacts())
        .staging_rewrite(SentenceLabelSelection::default(), "make it official");
    let engine = SessionEngine::start(vec![staged]);
    assert_eq!(
        (engine.next_target(), engine.batch_state()),
        (None, None),
        "a staged rewrite let the engine publish before Ctrl+G activated the batch"
    );
}

#[test]
fn restoring_a_legacy_axis_can_return_the_selection_to_none() {
    let baseline = SentenceLabelSelection::empty();
    let changed = baseline.choosing(SentenceAxis::Register, 2);
    let restored = changed.restoring(SentenceAxis::Register, &baseline);
    assert_eq!(
        (
            restored.token(SentenceAxis::Register),
            restored.pinned().contains(SentenceAxis::Register),
            restored.approx().contains(SentenceAxis::Register),
        ),
        (None, false, false),
        "legacy axis restoration retained a value, pin, or approximation"
    );
}

#[test]
fn revision_meta_success_replaces_identity_and_clears_the_queued_rewrite() {
    let queued = draft("whilst")
        .with_meta(meta_for("whilst"), None)
        .staging_rewrite(SentenceLabelSelection::default(), "make it shorter")
        .starting_rewrite();
    let mut engine = SessionEngine::start(vec![queued]);
    let revision = CardRevision::new(
        "while",
        "during the time that, in a shorter form",
        meta_for("while"),
    );
    let event =
        engine.applied_revision_attempt(0, ArtifactAttempt::unmetered(Ok((revision, None))));
    let revised = &engine.drafts()[0];
    assert_eq!(
        (
            event,
            revised.term(),
            revised.understanding(),
            revised.rewrite(),
            revised.artifacts().meta().ready(),
        ),
        (
            EngineEvent::ArtifactReady {
                card: 0,
                artifact: Artifact::Meta,
            },
            "while",
            "during the time that, in a shorter form",
            None,
            true,
        ),
        "revision meta success kept stale identity, rewrite state, or pending metadata"
    );
}
