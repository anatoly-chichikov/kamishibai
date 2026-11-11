//! Roundtrip trusted domain checkpoints without losing generation provenance.

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, AttemptFault, AttemptPenalties, AttemptScorecard,
    CardArtifacts, CardDraft, GenerationCost, LanguagePair, Sense, SentenceLabelSelection,
    WordCandidate,
};

#[test]
fn persisted_candidate_cannot_drop_translations_tags_or_reviewed_selection() {
    let candidate = WordCandidate::with_selected_senses(
        "лук",
        vec![
            Sense::translated("onion", "a vegetable", Some(String::from("food"))),
            Sense::translated("bow", "a weapon", Some(String::from("archery"))),
            Sense::translated("bow", "a curved object", None),
        ],
        vec![0, 2],
        true,
    );
    let bytes = serde_json::to_vec(&candidate).expect("reviewed candidate must serialize");
    let restored: WordCandidate =
        serde_json::from_slice(&bytes).expect("trusted candidate must restore");
    assert_eq!(
        restored, candidate,
        "checkpoint lost original input, sense translation, domain tag, or selected meanings"
    );
}

#[test]
fn persisted_draft_cannot_reset_attempts_usage_or_pending_adjustment() {
    let fault = AttemptFault::new(
        "topology",
        "missing one panel",
        Some(PathBuf::from("artifact-history/attempt-3.jpg")),
        Some(AttemptScorecard::new(
            61,
            false,
            AttemptPenalties::new(32, 2, 5, 0),
        )),
    );
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture)
            .faulted(fault)
            .accounted(GenerationCost::unknown() + GenerationCost::from_nanos(371_923)),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(
            ArtifactFile::new("audio.wav", "artifacts/audio.wav", "149 KB", false)
                .with_cost(GenerationCost::from_nanos(619_003)),
        ),
    );
    let draft = CardDraft::from_candidate(
        &WordCandidate::with_selected_senses(
            "лук",
            vec![
                Sense::translated("bow", "a weapon", Some(String::from("archery"))),
                Sense::translated("bow", "a curved object", None),
            ],
            vec![0, 1],
            true,
        ),
        1,
        LanguagePair::new("EN", "RU"),
    )
    .with_artifacts(artifacts)
    .staging_rewrite(
        SentenceLabelSelection::default(),
        "use the mechanical meaning",
    );
    let bytes = serde_json::to_vec(&draft).expect("draft checkpoint must serialize");
    let restored: CardDraft = serde_json::from_slice(&bytes).expect("trusted draft must restore");
    assert_eq!(
        restored, draft,
        "checkpoint changed selected sense priorities, retry history, partial prices, completed files, or staged adjustment"
    );
}
