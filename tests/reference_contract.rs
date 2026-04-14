//! Tests for frozen Rust reference artifacts.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

/// Return one reference manifest path.
fn reference(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("reference")
        .join("manifests")
        .join(name)
}

/// Return one parsed JSON reference manifest.
fn json(name: &str) -> Value {
    serde_json::from_str(
        fs::read_to_string(reference(name))
            .expect("reference manifest must exist")
            .as_str(),
    )
    .expect("reference manifest must parse")
}

/// Normalized reference entries keep the full twelve-field contract.
#[test]
fn normalized_reference_entries_keep_the_full_twelve_field_contract() {
    assert_eq!(
        json("normalized/single-target-en.json")[0]
            .as_object()
            .map(|item| item.keys().cloned().collect::<Vec<_>>())
            .map(|mut item| {
                item.sort();
                item
            }),
        Some(vec![
            String::from("context"),
            String::from("example"),
            String::from("highlight"),
            String::from("hint"),
            String::from("importance"),
            String::from("pronunciation"),
            String::from("sentence"),
            String::from("source_lang"),
            String::from("target_lang"),
            String::from("transcription"),
            String::from("translation"),
            String::from("word"),
        ]),
        "normalized reference entries no longer keep the full twelve-field contract"
    );
}

/// The APKG reference manifest keeps the mixed-target deck fallback name.
#[test]
fn the_apkg_reference_manifest_keeps_the_mixed_target_deck_fallback_name() {
    assert_eq!(
        json("apkg.json")["deck"]["name"].as_str(),
        Some("Kamishibai Deck"),
        "the apkg reference manifest no longer keeps the mixed-target deck fallback name"
    );
}

/// The APKG reference manifest keeps the eleven note fields in order.
#[test]
fn the_apkg_reference_manifest_keeps_the_eleven_note_fields_in_order() {
    assert_eq!(
        json("apkg.json")["model"]["fields"].as_array().map(|item| {
            item.iter()
                .map(|field| {
                    String::from(field.as_str().expect("reference field must be a string"))
                })
                .collect::<Vec<_>>()
        }),
        Some(vec![
            String::from("SourceSentence"),
            String::from("Term"),
            String::from("Pronunciation"),
            String::from("Meaning"),
            String::from("TargetSentence"),
            String::from("Importance"),
            String::from("Audio"),
            String::from("Illustration"),
            String::from("Hint"),
            String::from("Context"),
            String::from("PronunciationAll"),
        ]),
        "the apkg reference manifest no longer keeps the eleven note fields in order"
    );
}

/// The report reference manifest keeps the Chinese font-selection case.
#[test]
fn the_report_reference_manifest_keeps_the_chinese_font_selection_case() {
    assert_eq!(
        json("report.json")["entries"].as_array().map(|item| {
            item.iter()
                .filter(|entry| entry["target_lang"].as_str() == Some("zh"))
                .map(|entry| {
                    String::from(
                        entry["font"]
                            .as_str()
                            .expect("report font entry must be a string"),
                    )
                })
                .collect::<Vec<_>>()
        }),
        Some(vec![String::from("Hiragino Sans GB")]),
        "the report reference manifest no longer keeps the Chinese font-selection case"
    );
}
