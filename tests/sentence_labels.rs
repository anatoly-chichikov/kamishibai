//! Integration contracts for sentence attribution and the inline label editor.

use std::fs;

use kamishibai::session::{
    AxisSet, CardMeta, CardMetaCache, LanguagePair, Register, SentenceAxis, SentenceKind,
    SentenceLabelSelection, SentenceLabels, SentenceLevel,
};

#[test]
fn legacy_effort_tokens_reopen_as_neighbouring_cefr_levels() {
    let levels = [
        "easy",
        "takes practice",
        "balanced",
        "challenging",
        "stretch",
    ]
    .map(|token| {
        serde_json::from_str::<SentenceLevel>(format!("\"{token}\"").as_str())
            .expect("legacy effort level must decode")
    });
    assert_eq!(
        levels,
        [
            SentenceLevel::A2,
            SentenceLevel::B1,
            SentenceLevel::B1,
            SentenceLevel::B2,
            SentenceLevel::B2,
        ],
        "legacy effort tokens stopped reopening on their neighbouring CEFR levels"
    );
}

#[test]
fn cefr_scale_serializes_all_visible_tokens_in_lowercase() {
    let tokens = [
        SentenceLevel::A1,
        SentenceLevel::A2,
        SentenceLevel::B1,
        SentenceLevel::B2,
        SentenceLevel::C1,
        SentenceLevel::C2,
    ]
    .into_iter()
    .map(|level| serde_json::to_string(&level).expect("sentence level must encode"))
    .collect::<Vec<_>>();
    assert_eq!(
        tokens,
        ["\"a1\"", "\"a2\"", "\"b1\"", "\"b2\"", "\"c1\"", "\"c2\""],
        "CEFR scale emitted uppercase or non-CEFR tokens into storage"
    );
}

#[test]
fn legacy_pending_level_pin_reopens_and_reserializes_as_lowercase_b1() {
    let selection = serde_json::from_value::<SentenceLabelSelection>(serde_json::json!({
        "values": {
            "register": "casual",
            "level": "balanced",
            "kind": "statement"
        },
        "pinned": ["level"],
        "approx": ["level"]
    }))
    .expect("legacy pending level selection must decode");
    let encoded = serde_json::to_value(&selection).expect("migrated selection must encode");
    assert_eq!(
        (
            selection.level(),
            selection.pinned().contains(SentenceAxis::Level),
            selection.approx().contains(SentenceAxis::Level),
            encoded["values"]["level"].as_str(),
        ),
        (Some(SentenceLevel::B1), true, true, Some("b1")),
        "legacy pending level state lost its pin or failed to normalize to lowercase b1"
    );
}

#[test]
fn card_meta_cache_round_trips_sentence_labels_without_changing_policy() {
    let directory = tempfile::tempdir().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let labels = SentenceLabels::new(
        Register::Casual,
        SentenceLevel::B1,
        SentenceKind::Statement,
        AxisSet::from_axes([SentenceAxis::Register]),
        AxisSet::from_axes([SentenceAxis::Register]),
    );
    let meta = CardMeta::new(
        "liŋ.ɡe.ʁe",
        "sɔ̃ paʁ.fœ̃ liŋ.ɡe.ʁe dɑ̃ lə ku.lwaʁ",
        "linger",
        6,
        "Her perfume lingered in the hall",
        "lingered",
        "A trace stays after its source has gone",
        "Usage context",
        "Son parfum lingerait dans le couloir",
    )
    .with_sentence_labels(labels.clone());
    CardMetaCache::new(directory.path())
        .store("lingerer", "to linger · casual", &pair, &meta)
        .expect("labeled meta must store");
    let loaded = CardMetaCache::new(directory.path())
        .load("lingerer", "to linger · casual", &pair)
        .expect("labeled meta must load")
        .expect("labeled meta must exist");
    assert_eq!(
        loaded.sentence_labels(),
        Some(&labels),
        "the meta cache dropped sentence labels or their client-owned pin state"
    );
}

#[test]
fn current_policy_meta_without_labels_reopens_as_legacy_metadata() {
    let directory = tempfile::tempdir().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let cache = CardMetaCache::new(directory.path());
    let labeled = CardMeta::new(
        "a.nɔ̃s",
        "il a.nɔ̃s la nu.vɛl",
        "announce",
        5,
        "He announces the news",
        "announces",
        "Make information public",
        "Common public communication",
        "Il annonce la nouvelle",
    )
    .with_sentence_labels(SentenceLabels::new(
        Register::Neutral,
        SentenceLevel::B1,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::default(),
    ));
    let (_, path, _) = cache
        .store("annoncer", "to announce", &pair, &labeled)
        .expect("current labeled meta must store");
    let mut document: serde_json::Value = serde_json::from_slice(
        fs::read(path.as_path())
            .expect("current meta file must remain readable")
            .as_slice(),
    )
    .expect("current meta file must decode");
    document
        .as_object_mut()
        .expect("current meta document must be an object")
        .remove("labels");
    fs::write(
        path.as_path(),
        serde_json::to_vec_pretty(&document).expect("legacy-shaped meta must encode"),
    )
    .expect("legacy-shaped meta must store");
    let loaded = cache
        .load("annoncer", "to announce", &pair)
        .expect("legacy-shaped current meta must load")
        .expect("legacy-shaped current meta must exist");
    let reused = cache
        .store("annoncer", "to announce", &pair, &labeled)
        .expect("current legacy-shaped meta must remain reusable");
    assert_eq!(
        (loaded.sentence_labels(), reused.2),
        (None, true),
        "a current-policy meta document without labels stopped behaving as readable legacy metadata"
    );
}
