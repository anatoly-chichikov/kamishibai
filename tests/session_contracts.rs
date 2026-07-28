//! Word-first session contracts end-to-end: covers both
//! `Your words -> What I understood` and `What I understood -> Your cards`
//! with fully mocked LLM passes. No network, no real Gemini.

use anyhow::Result;
use kamishibai::session::{
    Artifact, ArtifactFile, BulkCorrection, CardArtifacts, CardDraft, CardMeta, GenerationCost,
    LanguagePair, LearningDetection, LearningGuess, LearningTarget, RawInputBatch, ScriptDetection,
    Sense, SenseCorrection, SessionState, Understanding, Understood, WordCandidate, to_document,
    to_entry,
};

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(
        &self,
        _raw: &RawInputBatch,
        _my: &str,
        _target: &LearningTarget,
    ) -> Result<Understood> {
        Ok(Understood::new(
            LearningGuess::new("en", false),
            vec![
                WordCandidate::new(
                    "whilst",
                    "neutral conjunction; British English, slightly bookish",
                    true,
                ),
                WordCandidate::new(
                    "wreck",
                    "ambiguous between noun (the remains of a destroyed ship) and verb (to destroy); context suggests verb",
                    true,
                ),
            ],
        ))
    }
}

struct FakeBulk;

impl BulkCorrection for FakeBulk {
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        _comment: &str,
        _pair: &LanguagePair,
    ) -> Result<SenseCorrection> {
        Ok(SenseCorrection::adding(vec![Sense::plain(format!(
            "{}; user clarified verb sense",
            candidate.understanding()
        ))]))
    }
}

fn meta_for(candidate: &WordCandidate) -> CardMeta {
    let term = candidate.term();
    CardMeta::new(
        format!("/{term}/"),
        format!("/sentence with {term}/"),
        format!("local meaning of {term}"),
        5,
        format!("Russian translation of a sentence with {term}"),
        term,
        format!("vivid recall image for {term}"),
        format!("usage notes around {term}"),
        format!("English example with {term}."),
    )
}

fn draft_for(candidate: &WordCandidate, pair: &LanguagePair) -> CardDraft {
    CardDraft::new(candidate.term(), candidate.understanding(), pair.clone())
        .with_meta(meta_for(candidate), None)
}

#[test]
fn your_words_to_what_i_understood_flow_builds_session_with_confirmed_candidates() {
    let raw = RawInputBatch::new("whilst\nwreck");
    let guess = ScriptDetection
        .detect(raw.text(), &kamishibai::languages::catalog())
        .expect("detection must succeed");
    let pair = LanguagePair::new(guess.code(), "ru");
    let understood = FakeUnderstanding
        .understand(&raw, pair.known(), &LearningTarget::Detect)
        .expect("understanding must succeed");
    let session =
        SessionState::starting(pair.clone(), raw).confirming(understood.candidates().to_vec());
    assert_eq!(
        (
            session.pair().label(),
            session.confirmed().len(),
            session.confirmed()[0].term(),
        ),
        (String::from("RU → EN"), 2, "whilst"),
        "flow must attach the detected pair and mocked candidates to session state"
    );
}

#[test]
fn bulk_correction_pass_replaces_candidate_metadata_without_touching_pair() {
    let pair = LanguagePair::new("en", "ru");
    let before = FakeUnderstanding
        .understand(
            &RawInputBatch::new("whilst\nwreck"),
            pair.known(),
            &LearningTarget::Detect,
        )
        .expect("understanding must succeed")
        .candidates()
        .to_vec();
    let after = FakeBulk
        .correct_bulk(&before[1], "#2 — глагол", &pair)
        .expect("bulk correction must succeed");
    assert!(
        after
            .senses()
            .first()
            .expect("bulk result must add one sense")
            .understanding()
            .contains("verb sense"),
        "bulk correction pass must apply user comment to the targeted candidate"
    );
}

#[test]
fn what_i_understood_to_your_cards_flow_bridges_drafts_into_vocabulary_document() {
    let pair = LanguagePair::new("en", "ru");
    let candidates = FakeUnderstanding
        .understand(
            &RawInputBatch::new("whilst\nwreck"),
            pair.known(),
            &LearningTarget::Detect,
        )
        .expect("understanding must succeed")
        .candidates()
        .to_vec();
    let drafts: Vec<CardDraft> = candidates
        .iter()
        .map(|candidate| draft_for(candidate, &pair))
        .collect();
    let document = to_document(&drafts).expect("bridge must yield a vocabulary document");
    assert_eq!(
        (
            document.entries.len(),
            document.entries[0].target.lang.as_str().to_string(),
            document.entries[0].source.lang.as_str().to_string(),
        ),
        (2, String::from("en"), String::from("ru")),
        "bridge must carry both languages explicitly into the internal vocabulary document"
    );
}

#[test]
fn card_artifacts_start_unready_with_one_try_and_three_retries_each() {
    let artifacts = CardArtifacts::default();
    let slot = artifacts.scene();
    assert_eq!(
        (
            slot.kind(),
            slot.ready(),
            slot.tally().retries(),
            slot.tally().retry(),
            slot.tally().done(),
        ),
        (Artifact::Scene, false, 3, None, 0),
        "fresh card artifacts must start unready on an unnumbered first try with three retries left"
    );
}

#[test]
fn terminal_failure_waits_for_the_first_try_and_all_three_retries() {
    let spent = (0..4).scan(
        kamishibai::session::ArtifactSlot::fresh(Artifact::Picture),
        |slot, _| {
            *slot = slot.clone().attempted();
            Some(slot.failed_terminally())
        },
    );
    assert_eq!(
        spent.collect::<Vec<_>>(),
        vec![false, false, false, true],
        "the slot gave up before its three retries were spent, or never gave up at all"
    );
}

#[test]
fn failed_artifact_slot_keeps_the_latest_cumulative_cost_without_a_file() {
    let first = GenerationCost::from_nanos(120_000_000);
    let latest = GenerationCost::from_nanos(310_000_000);
    let slot = kamishibai::session::ArtifactSlot::fresh(Artifact::Picture)
        .attempted_with(first)
        .attempted_with(latest);
    assert_eq!(
        slot.cost(),
        Some(latest),
        "failed artifact slot lost the latest cumulative Gemini spend"
    );
}

#[test]
fn successful_file_cost_replaces_the_failed_attempt_total_without_double_counting() {
    let file = ArtifactFile::new(
        "picture.jpg",
        std::env::temp_dir().join("picture.jpg"),
        "1 B",
        false,
    )
    .with_cost(GenerationCost::from_nanos(450_000_000));
    let slot = kamishibai::session::ArtifactSlot::fresh(Artifact::Picture)
        .attempted_with(GenerationCost::from_nanos(130_000_000))
        .succeeded_with(file);
    assert_eq!(
        slot.cost(),
        Some(GenerationCost::from_nanos(450_000_000)),
        "successful cumulative file cost was added to the failed-attempt total twice"
    );
}

#[test]
fn bridge_from_single_draft_fills_both_languages() {
    let pair = LanguagePair::new("en", "ru");
    let candidate = WordCandidate::new("wreck", "verb sense — to destroy a vehicle", true);
    let draft = draft_for(&candidate, &pair);
    let entry = to_entry(&draft).expect("bridge must succeed");
    assert_eq!(
        (
            entry.target.lang.as_str().to_string(),
            entry.source.lang.as_str().to_string(),
        ),
        (String::from("en"), String::from("ru")),
        "single-draft bridge must carry target and support language codes explicitly"
    );
}
