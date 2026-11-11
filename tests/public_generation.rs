//! External callers reuse generation with their own providers and local artifacts.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow};
use kamishibai::application::{CardProduction, GenerationRun};
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta,
    CardRevision, EngineEvent, GenerationCost, LanguagePair, Sense, SentenceLabelSelection,
    WordCandidate,
};
use tempfile::TempDir;

struct LocalProduction<'a> {
    directory: &'a Path,
    calls: RefCell<Vec<Artifact>>,
    failures: Cell<u8>,
    observed: RefCell<Vec<CardDraft>>,
}

impl<'a> LocalProduction<'a> {
    fn new(directory: &'a Path, failures: u8) -> Self {
        Self {
            directory,
            calls: RefCell::new(Vec::new()),
            failures: Cell::new(failures),
            observed: RefCell::new(Vec::new()),
        }
    }

    fn write(&self, term: &str, artifact: Artifact) -> Result<ArtifactFile> {
        let name = format!("{term}-{}.txt", artifact.label());
        let path = self.directory.join(&name);
        fs::write(&path, artifact.label())?;
        Ok(ArtifactFile::new(name, path, "local", false))
    }

    fn media(&self, draft: &CardDraft, artifact: Artifact) -> ArtifactAttempt<ArtifactFile> {
        self.calls.borrow_mut().push(artifact);
        ArtifactAttempt::unmetered(self.write(draft.term(), artifact))
    }
}

impl CardProduction for LocalProduction<'_> {
    fn generate_draft_meta_in(
        &self,
        _slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        self.observed.borrow_mut().push(draft.clone());
        if draft.rewrite().is_some() {
            return ArtifactAttempt::unmetered(Err(anyhow!("local production does not rewrite")));
        }
        self.calls.borrow_mut().push(Artifact::Meta);
        let meta = meta(draft.term());
        let result = self
            .store_card_meta(draft.term(), draft.understanding(), draft.pair(), &meta)
            .map(|file| {
                (
                    CardRevision::new(draft.term(), draft.understanding(), meta),
                    Some(file),
                )
            });
        ArtifactAttempt::unmetered(result)
    }

    fn generate_scene_in(&self, _slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.media(draft, Artifact::Scene)
    }

    fn generate_picture_in(
        &self,
        _slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<ArtifactFile> {
        self.calls.borrow_mut().push(Artifact::Picture);
        let result = if self.failures.get() > 0 {
            self.failures.set(self.failures.get() - 1);
            Err(anyhow!("temporary picture failure"))
        } else {
            self.write(draft.term(), Artifact::Picture)
        };
        ArtifactAttempt::new(result, Some(GenerationCost::from_nanos(37)))
    }

    fn generate_sound_in(&self, _slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.media(draft, Artifact::Sound)
    }

    fn store_card_meta(
        &self,
        term: &str,
        _understanding: &str,
        _pair: &LanguagePair,
        _meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.write(term, Artifact::Meta)
    }
}

struct DeletingProduction<'a> {
    production: &'a LocalProduction<'a>,
    removed: &'a Path,
}

impl CardProduction for DeletingProduction<'_> {
    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        self.production.generate_draft_meta_in(slot, draft)
    }

    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_scene_in(slot, draft)
    }

    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        let attempt = self.production.generate_picture_in(slot, draft);
        fs::remove_file(self.removed).expect("provider must remove its chosen artifact");
        attempt
    }

    fn generate_sound_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_sound_in(slot, draft)
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.production
            .store_card_meta(term, understanding, pair, meta)
    }
}

fn draft(term: &str) -> CardDraft {
    CardDraft::new(term, "to walk without hurry", LanguagePair::new("FR", "EN"))
}

fn meta(term: &str) -> CardMeta {
    CardMeta::new(
        "/flɑne/",
        "/nu flɑnɔ̃/",
        "to stroll",
        4,
        format!("Nous aimons {term}"),
        term,
        "walk at leisure",
        "a quiet afternoon",
        "We like to stroll",
    )
}

fn finish(run: &mut GenerationRun, production: &dyn CardProduction) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    for _ in 0..32 {
        let Some(step) = run
            .advance(production)
            .expect("completed files must stay valid")
        else {
            break;
        };
        events.push(step.into_parts().2);
    }
    events
}

#[test]
fn custom_production_reuses_queue_retries_costs_and_real_artifacts_without_publishing() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 1);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    let events = finish(&mut run, &production);
    let card = &run.drafts()[0];
    let artifacts = card.artifacts();
    let files = [
        artifacts.meta(),
        artifacts.sound(),
        artifacts.scene(),
        artifacts.picture(),
    ]
    .into_iter()
    .all(|slot| slot.file().is_some_and(|file| file.path().is_file()));
    assert_eq!(
        (
            production.calls.into_inner(),
            events,
            artifacts.cost(),
            files,
            run.state().expect("completed files must stay valid"),
        ),
        (
            vec![
                Artifact::Meta,
                Artifact::Sound,
                Artifact::Scene,
                Artifact::Picture,
                Artifact::Picture
            ],
            vec![
                EngineEvent::ArtifactReady {
                    card: 0,
                    artifact: Artifact::Meta
                },
                EngineEvent::ArtifactReady {
                    card: 0,
                    artifact: Artifact::Sound
                },
                EngineEvent::ArtifactReady {
                    card: 0,
                    artifact: Artifact::Scene
                },
                EngineEvent::RetryStarted {
                    card: 0,
                    artifact: Artifact::Picture,
                    attempt: 1
                },
                EngineEvent::ArtifactReady {
                    card: 0,
                    artifact: Artifact::Picture
                },
            ],
            Some(GenerationCost::from_nanos(74)),
            true,
            Some(EngineEvent::BatchReady),
        ),
        "external production lost engine ordering, retries, spend, or actual generated files"
    );
}

#[test]
fn required_metadata_generation_receives_reviewed_senses_tags_and_original_priorities() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let candidate = WordCandidate::with_senses(
        "canard",
        vec![
            Sense::plain("a duck"),
            Sense::tagged("a false report", "journalism"),
            Sense::tagged("a wrong note", "music"),
        ],
        2,
        true,
    );
    let chosen = CardDraft::from_candidate(&candidate, 2, LanguagePair::new("FR", "EN"));
    let mut run = GenerationRun::new(vec![chosen]).expect("reviewed batch must start");
    let _ = run
        .advance(&production)
        .expect("completed files must stay valid")
        .expect("metadata must be generated");
    let observed = production.observed.borrow();
    let draft = &observed[0];
    assert_eq!(
        draft
            .reviewed_senses()
            .iter()
            .enumerate()
            .map(|(index, sense)| (
                sense.understanding(),
                sense.tag(),
                draft.sense_priority(index)
            ))
            .collect::<Vec<_>>(),
        vec![
            ("a wrong note", Some("music"), 2),
            ("a duck", None, 0),
            ("a false report", Some("journalism"), 1)
        ],
        "the public metadata contract dropped reviewed context before invoking the provider"
    );
}

#[test]
fn cancellation_prevents_more_provider_calls_and_does_not_report_completion() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    let _ = run
        .advance(&production)
        .expect("completed files must stay valid")
        .expect("metadata must be generated");
    run.cancel();
    assert_eq!(
        (
            run.advance(&production)
                .expect("cancelled run must remain inert")
                .is_none(),
            run.next(),
            run.state().expect("cancelled state must remain inert"),
            production.calls.into_inner(),
        ),
        (true, None, None, vec![Artifact::Meta]),
        "a cancelled run called a provider again or advertised a completed batch"
    );
}

#[test]
fn cancelled_drafts_resume_without_repeating_finished_provider_calls() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    let _ = run
        .advance(&production)
        .expect("completed files must stay valid")
        .expect("metadata must be generated");
    run.cancel();
    let mut resumed = GenerationRun::new(run.into_drafts()).expect("cancelled drafts must resume");
    let _ = finish(&mut resumed, &production);
    assert_eq!(
        (
            production.calls.into_inner(),
            resumed.state().expect("completed files must stay valid")
        ),
        (
            vec![
                Artifact::Meta,
                Artifact::Sound,
                Artifact::Scene,
                Artifact::Picture
            ],
            Some(EngineEvent::BatchReady),
        ),
        "resuming cancellation repeated a completed artifact or failed to finish"
    );
}

#[test]
fn resume_cannot_accept_a_ready_media_slot_whose_file_was_deleted() {
    let refused = [Artifact::Sound, Artifact::Scene, Artifact::Picture].map(|artifact| {
        let directory = TempDir::new().expect("temporary output must exist");
        let production = LocalProduction::new(directory.path(), 0);
        let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
        let _ = finish(&mut run, &production);
        fs::remove_file(
            directory
                .path()
                .join(format!("flâner-{}.txt", artifact.label())),
        )
        .expect("generated media must be removable");
        GenerationRun::new(run.into_drafts()).is_err()
    });
    assert_eq!(
        refused,
        [true, true, true],
        "resume accepted a ready media slot whose file had disappeared"
    );
}

#[test]
fn resume_cannot_accept_empty_or_non_file_media() {
    let refused = [false, true].map(|directory_entry| {
        let directory = TempDir::new().expect("temporary output must exist");
        let production = LocalProduction::new(directory.path(), 0);
        let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
        let _ = finish(&mut run, &production);
        let picture = directory.path().join("flâner-picture.txt");
        fs::remove_file(&picture).expect("generated picture must be removable");
        if directory_entry {
            fs::create_dir(&picture).expect("directory collision must exist");
        } else {
            fs::write(&picture, b"").expect("empty media file must exist");
        }
        GenerationRun::new(run.into_drafts()).is_err()
    });
    assert_eq!(
        refused,
        [true, true],
        "resume accepted empty content or a directory as generated media"
    );
}

#[test]
fn completion_cannot_report_ready_after_generated_media_disappears() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    let _ = finish(&mut run, &production);
    fs::remove_file(directory.path().join("flâner-picture.txt"))
        .expect("generated picture must be removable");
    assert!(
        run.state().is_err(),
        "completion advertised a ready batch after an output file disappeared"
    );
}

#[test]
fn a_final_provider_call_cannot_report_success_after_removing_completed_media() {
    let refused = ["flâner-picture.txt", "errer-sound.txt"].map(|removed| {
        let directory = TempDir::new().expect("temporary output must exist");
        let production = LocalProduction::new(directory.path(), 0);
        let mut run = GenerationRun::new(vec![draft("errer"), draft("flâner")])
            .expect("fresh batch must start");
        for _ in 0..7 {
            let _ = run
                .advance(&production)
                .expect("preceding files must validate")
                .expect("preceding stage must run");
        }
        let removed = directory.path().join(removed);
        let deleting = DeletingProduction {
            production: &production,
            removed: &removed,
        };
        (
            run.advance(&deleting).is_err(),
            run.drafts()[1].artifacts().cost(),
        )
    });
    assert_eq!(
        refused,
        [(true, Some(GenerationCost::from_nanos(37))); 2],
        "a successful step escaped after output disappeared or lost already-spent provider cost"
    );
}

#[test]
fn lost_completed_media_stops_the_next_provider_call_with_an_explicit_error() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    for _ in 0..2 {
        let _ = run
            .advance(&production)
            .expect("completed files must validate")
            .expect("stage must run");
    }
    fs::remove_file(directory.path().join("flâner-sound.txt")).expect("audio must be removable");
    assert_eq!(
        (
            run.advance(&production).is_err(),
            production.calls.into_inner()
        ),
        (true, vec![Artifact::Meta, Artifact::Sound]),
        "a missing dependency allowed another paid provider operation"
    );
}

#[test]
fn ready_metadata_cannot_resume_without_its_in_memory_payload() {
    let directory = TempDir::new().expect("temporary output must exist");
    let path = directory.path().join("meta.json");
    fs::write(&path, b"{}").expect("metadata file must exist");
    let invalid = draft("flâner").with_artifacts(CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(ArtifactFile::new(
            "meta.json",
            path,
            "2 B",
            false,
        )),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    ));
    assert!(
        GenerationRun::new(vec![invalid]).is_err(),
        "a ready metadata marker resumed without actual card metadata"
    );
}

#[test]
fn in_memory_metadata_does_not_require_a_separate_json_artifact() {
    let ready = draft("flâner").with_meta(meta("flâner"), None);
    assert_eq!(
        GenerationRun::new(vec![ready])
            .expect("in-memory metadata must suffice")
            .next(),
        Some((0, Artifact::Sound)),
        "metadata without an optional JSON file was regenerated or rejected"
    );
}

#[test]
fn staged_adjustments_refuse_the_entire_batch_before_an_unrelated_card_runs() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let staged = draft("flâner")
        .with_meta(meta("flâner"), None)
        .staging_rewrite(SentenceLabelSelection::empty(), "use a question");
    let result = GenerationRun::new(vec![draft("errer"), staged]);
    let refused = match result {
        Ok(mut run) => {
            let _ = finish(&mut run, &production);
            false
        }
        Err(_) => true,
    };
    assert_eq!(
        (refused, production.calls.into_inner()),
        (true, Vec::<Artifact>::new()),
        "a pending adjustment let generation start or publish unchanged content"
    );
}

#[test]
fn required_metadata_generation_delivers_the_active_note_before_an_explicit_refusal() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 0);
    let rewriting = draft("flâner")
        .with_meta(meta("flâner"), None)
        .staging_rewrite(SentenceLabelSelection::empty(), "use a question")
        .starting_rewrite();
    let mut run = GenerationRun::new(vec![rewriting]).expect("activated rewrite must start");
    let step = run
        .advance(&production)
        .expect("completed files must stay valid")
        .expect("rewrite must be attempted");
    assert_eq!(
        (
            step.into_parts().3.is_some(),
            production.observed.borrow()[0]
                .rewrite()
                .map(|rewrite| String::from(rewrite.note())),
            production.calls.into_inner(),
        ),
        (
            true,
            Some(String::from("use a question")),
            Vec::<Artifact>::new()
        ),
        "the metadata provider lost the active note or its explicit refusal was discarded"
    );
}

#[test]
fn terminal_provider_failure_exhausts_only_the_existing_attempt_budget() {
    let directory = TempDir::new().expect("temporary output must exist");
    let production = LocalProduction::new(directory.path(), 9);
    let mut run = GenerationRun::new(vec![draft("flâner")]).expect("fresh batch must start");
    let _ = finish(&mut run, &production);
    assert_eq!(
        (
            production
                .calls
                .borrow()
                .iter()
                .filter(|kind| **kind == Artifact::Picture)
                .count(),
            run.state().expect("completed files must stay valid"),
            run.drafts()[0].artifacts().cost(),
        ),
        (
            4,
            Some(EngineEvent::BatchDone { failed_cards: 1 }),
            Some(GenerationCost::from_nanos(148))
        ),
        "public generation changed the four-attempt ceiling or omitted failed-call spend"
    );
}

struct Selected {
    card: usize,
    paused: bool,
}

struct AfterFailure<'a> {
    remaining: &'a Cell<u8>,
}

impl kamishibai::application::GenerationControl for AfterFailure<'_> {
    fn allows(&self, _card: usize) -> bool {
        true
    }

    fn paused(&self) -> bool {
        self.remaining.get() == 0
    }
}

impl kamishibai::application::GenerationControl for Selected {
    fn allows(&self, card: usize) -> bool {
        self.card == card
    }

    fn paused(&self) -> bool {
        self.paused
    }
}

#[test]
fn selected_generation_cannot_advance_another_card() {
    let directory = TempDir::new().expect("invariant: artifacts directory exists");
    let production = LocalProduction::new(directory.path(), 0);
    let mut run = GenerationRun::new(vec![draft("lanterne"), draft("rivage")])
        .expect("invariant: fresh cards start");
    let step = run
        .advance_in(
            &production,
            &Selected {
                card: 1,
                paused: false,
            },
        )
        .expect("selected artifact must settle")
        .expect("selected card must have work")
        .into_parts();
    assert_eq!(
        (
            step.0,
            step.1,
            run.drafts()[0].meta().is_none(),
            run.drafts()[1].meta().is_some()
        ),
        (1, Artifact::Meta, true, true),
        "selection advanced another card or renumbered its stable position"
    );
}

#[test]
fn paused_generation_cannot_spend_a_retry_or_lose_observed_cost() {
    let directory = TempDir::new().expect("invariant: artifacts directory exists");
    let production = LocalProduction::new(directory.path(), 1);
    let mut run = GenerationRun::new(vec![draft("rivage")]).expect("invariant: fresh card starts");
    for _ in 0..3 {
        run.advance(&production)
            .expect("preceding artifact must settle");
    }
    let step = run
        .advance_in(
            &production,
            &AfterFailure {
                remaining: &production.failures,
            },
        )
        .expect("cooperative pause must settle");
    let slot = run.drafts()[0].artifacts().picture();
    assert_eq!(
        (step.is_none(), run.next(), slot.tally().done(), slot.cost()),
        (
            true,
            Some((0, Artifact::Picture)),
            0,
            Some(GenerationCost::from_nanos(37))
        ),
        "a cooperative pause spent a retry, lost cost, or changed the next artifact"
    );
}

#[test]
fn standing_pause_cannot_start_any_artifact() {
    let mut observed = Vec::new();
    for preceding in 0..4 {
        let directory = TempDir::new().unwrap();
        let production = LocalProduction::new(directory.path(), 0);
        let mut run = GenerationRun::new(vec![draft("rivage")]).unwrap();
        for _ in 0..preceding {
            run.advance(&production).unwrap();
        }
        let before = serde_json::to_value(run.drafts()).unwrap();
        let calls = production.calls.borrow().clone();
        let next = run.next();
        let control = Selected {
            card: 0,
            paused: true,
        };
        let mut settled = 0;
        for _ in 0..273 {
            settled += usize::from(run.advance_in(&production, &control).unwrap().is_some());
        }
        let unchanged = (
            *production.calls.borrow() == calls,
            serde_json::to_value(run.drafts()).unwrap() == before,
        );
        let resumed = run
            .advance_in(
                &production,
                &Selected {
                    card: 0,
                    paused: false,
                },
            )
            .unwrap()
            .map(|step| {
                let (card, artifact, _, _) = step.into_parts();
                (card, artifact)
            });
        observed.push((settled, unchanged.0, unchanged.1, resumed == next));
    }
    assert_eq!(
        observed,
        vec![(0, true, true, true); 4],
        "standing pause called production, changed a draft, or lost the next artifact"
    );
}
