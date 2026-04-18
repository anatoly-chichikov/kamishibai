//! Word-first session contracts end-to-end: covers both
//! `Your words -> What I understood` and `What I understood -> Your cards`
//! with fully mocked LLM passes. No network, no real Gemini.

use anyhow::Result;
use kamishibai::session::{
    Artifact, BulkCorrection, CandidateKind, CardArtifacts, CardDraft, CardPayload, LanguagePair,
    RawInputBatch, ScriptDetection, SessionState, TargetDetection, TargetGuess, Understanding,
    Understood, WordCandidate, to_document, to_entry,
};

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(&self, _raw: &RawInputBatch, _my: &str) -> Result<Understood> {
        Ok(Understood::new(
            TargetGuess::new("en", false),
            vec![
                WordCandidate::new(
                    "whilst",
                    CandidateKind::Other(String::from("formal conjunction")),
                    "«пока, в то время как» · BrE",
                    "formal, bookish usage",
                ),
                WordCandidate::new(
                    "wreck",
                    CandidateKind::Other(String::from("noun / verb")),
                    "обломки · разрушать",
                    "context suggests verb sense",
                ),
            ],
        ))
    }
}

struct FakeBulk;

impl BulkCorrection for FakeBulk {
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        _comment: &str,
        _pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        let mut patched = candidates.to_vec();
        if let Some(last) = patched.last_mut() {
            *last = WordCandidate::new(
                last.term(),
                CandidateKind::Other(String::from("verb")),
                "разбить (машину)",
                "user clarified verb sense",
            );
        }
        Ok(patched)
    }
}

fn draft_for(candidate: &WordCandidate, pair: &LanguagePair) -> CardDraft {
    let payload = CardPayload::new(
        format!(
            "Пока она говорила, я {} думал о своём.",
            candidate.preview()
        ),
        format!(
            "While she was speaking, I was thinking of my own stuff. ({})",
            candidate.preview()
        ),
        candidate.note(),
        candidate.term(),
    );
    CardDraft::new(candidate.term(), pair.clone(), payload)
}

#[test]
fn your_words_to_what_i_understood_flow_builds_session_with_confirmed_candidates() {
    let raw = RawInputBatch::new("whilst\nwreck");
    let guess = ScriptDetection
        .detect(raw.text(), &kamishibai::languages::catalog())
        .expect("detection must succeed");
    let pair = LanguagePair::new(guess.code(), "ru");
    let understood = FakeUnderstanding
        .understand(&raw, pair.support())
        .expect("understanding must succeed");
    let session =
        SessionState::starting(pair.clone(), raw).confirming(understood.candidates().to_vec());
    assert_eq!(
        (
            session.pair().label(),
            session.confirmed().len(),
            session.confirmed()[0].term(),
        ),
        (String::from("EN → RU"), 2, "whilst"),
        "flow must attach the detected pair and mocked candidates to session state"
    );
}

#[test]
fn bulk_correction_pass_replaces_candidate_metadata_without_touching_pair() {
    let pair = LanguagePair::new("en", "ru");
    let before = FakeUnderstanding
        .understand(&RawInputBatch::new("whilst\nwreck"), pair.support())
        .expect("understanding must succeed")
        .candidates()
        .to_vec();
    let after = FakeBulk
        .correct_bulk(&before, "#2 — глагол", &pair)
        .expect("bulk correction must succeed");
    assert_eq!(
        after
            .last()
            .expect("bulk result must keep the row")
            .preview(),
        "разбить (машину)",
        "bulk correction pass must apply user comment to the targeted candidate"
    );
}

#[test]
fn what_i_understood_to_your_cards_flow_bridges_drafts_into_vocabulary_document() {
    let pair = LanguagePair::new("en", "ru");
    let candidates = FakeUnderstanding
        .understand(&RawInputBatch::new("whilst\nwreck"), pair.support())
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
fn card_artifacts_start_unready_with_three_attempt_ceiling_each() {
    let artifacts = CardArtifacts::default();
    let slot = artifacts.scene();
    assert_eq!(
        (
            slot.kind(),
            slot.ready(),
            slot.tally().ceiling(),
            slot.tally().done(),
        ),
        (Artifact::Scene, false, 3, 0),
        "fresh card artifacts must start unready with a three-attempt budget per slot"
    );
}

#[test]
fn terminal_failure_is_recognisable_after_three_failed_attempts() {
    let mut slot = kamishibai::session::ArtifactSlot::fresh(Artifact::Picture);
    for _ in 0..3 {
        slot = slot.attempted();
    }
    assert!(
        slot.failed_terminally(),
        "three spent attempts without success must mark the slot as terminally failed"
    );
}

#[test]
fn bridge_from_single_draft_fills_both_languages() {
    let pair = LanguagePair::new("en", "ru");
    let candidate = WordCandidate::new(
        "wreck",
        CandidateKind::Other(String::from("verb")),
        "разбить (машину)",
        "verb sense",
    );
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
