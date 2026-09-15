//! Exercise concrete library composition through caller-owned paths and profiles.

use std::fs;
use std::path::Path;

use kamishibai::application::{CardProduction, CardUseCases};
use kamishibai::gemini::{GeminiClient, GeminiProfile, HttpTransport, StageModels, workflow};
use kamishibai::generation::artifact_cache::VOICE_FILE;
use kamishibai::session::{CardDraft, CardMeta};
use tempfile::TempDir;

fn configured(root: &Path, endpoint: &str) -> impl CardUseCases + Clone {
    let profile = GeminiProfile::from_models(endpoint, StageModels::default())
        .expect("explicit profile must be valid");
    workflow(
        GeminiClient::from_profile("unused-local-test-key", HttpTransport::new(), profile),
        root.join("cache"),
        root.join("output"),
        None,
    )
}

fn draft(sentence: &str) -> CardDraft {
    let meta = CardMeta::new(
        "ka.naʁ",
        "sample",
        "duck",
        5,
        "I see a duck",
        "duck",
        "a bird",
        "by the pond",
        sentence,
    );
    CardDraft::new(
        "canard",
        "a water bird",
        kamishibai::session::LanguagePair::new("FR", "EN"),
    )
    .with_meta(meta, None)
}

fn seed(production: &impl CardProduction, draft: &CardDraft, voice: &[u8]) {
    let file = production
        .store_card_meta(
            draft.term(),
            draft.understanding(),
            draft.pair(),
            draft.meta().expect("seed draft must have metadata"),
        )
        .expect("metadata must store without provider access");
    fs::write(
        file.path()
            .parent()
            .expect("metadata must have a parent")
            .join(VOICE_FILE),
        voice,
    )
    .expect("cached voice must store");
}

#[test]
fn differently_configured_workflows_cannot_share_metadata_or_audio() {
    let root = TempDir::new().expect("temporary workspace must exist");
    let first = configured(root.path(), "http://127.0.0.1:1/first/models");
    let second = configured(root.path(), "http://127.0.0.1:1/second/models");
    let first_draft = draft("Un canard traverse le jardin");
    let second_draft = draft("Le canard plonge dans le lac");
    seed(&first, &first_draft, b"first-voice");
    seed(&second, &second_draft, b"second-voice");
    let results = [(&first, &first_draft), (&second, &second_draft)].map(|(production, draft)| {
        let (meta, _) = production
            .generate_meta_in(0, draft.term(), draft.understanding(), draft.pair(), None)
            .into_result()
            .expect("own cached metadata must load without a network call");
        let audio = production
            .generate_sound_in(0, draft)
            .into_result()
            .expect("own cached audio must load without a network call");
        (
            String::from(meta.target_sentence()),
            fs::read(audio.path()).expect("audio must read"),
        )
    });
    assert_eq!(
        results,
        [
            (
                String::from("Un canard traverse le jardin"),
                b"first-voice".to_vec()
            ),
            (
                String::from("Le canard plonge dans le lac"),
                b"second-voice".to_vec()
            ),
        ],
        "one profile reused another profile's metadata or pronunciation"
    );
}

#[test]
fn identical_profiles_in_different_jobs_cannot_share_mutable_card_artifacts() {
    let root = TempDir::new().expect("temporary workspace must exist");
    let first = configured(&root.path().join("job-a"), "http://127.0.0.1:1/models");
    let second = configured(&root.path().join("job-b"), "http://127.0.0.1:1/models");
    let draft = draft("Un canard traverse le jardin");
    seed(&first, &draft, b"job-a-voice");
    seed(&second, &draft, b"job-b-voice");
    let audio = [&first, &second].map(|production| {
        let file = production
            .generate_sound_in(0, &draft)
            .into_result()
            .expect("job-owned cached audio must load");
        fs::read(file.path()).expect("audio must read")
    });
    assert_eq!(
        audio,
        [b"job-a-voice".to_vec(), b"job-b-voice".to_vec()],
        "job-owned roots mixed mutable artifacts from different requests"
    );
}
