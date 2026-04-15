//! Tests for Anki note formatting and APKG writing.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use kamishibai::anki::{
    CardModel, HtmlLineBreaks, NoteFormat, StableId, Transcription, VocabularyDeck, VocabularyNote,
};
use kamishibai::input::NormalizedEntry;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::ZipArchive;

fn reference(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("reference")
        .join("manifests")
        .join(name)
}

fn entries() -> Vec<NormalizedEntry> {
    serde_json::from_str(
        fs::read_to_string(reference("normalized/mixed-target-deck.json"))
            .expect("reference entries must exist")
            .as_str(),
    )
    .expect("reference entries must parse")
}

fn apkg() -> Value {
    serde_json::from_str(
        fs::read_to_string(reference("apkg.json"))
            .expect("reference apkg manifest must exist")
            .as_str(),
    )
    .expect("reference apkg manifest must parse")
}

fn media(directory: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = directory.join(name);
            fs::write(&path, name.as_bytes()).expect("media fixture must be writable");
            path
        })
        .collect()
}

fn manifest(path: &Path) -> Result<Value> {
    let file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let names = archive.file_names().map(String::from).collect::<Vec<_>>();
    let mut media = String::new();
    archive.by_name("media")?.read_to_string(&mut media)?;
    let directory = TempDir::new()?;
    let database = directory.path().join("collection.anki2");
    fs::write(&database, {
        let mut bytes = Vec::new();
        archive
            .by_name("collection.anki2")?
            .read_to_end(&mut bytes)?;
        bytes
    })?;
    let conn = Connection::open(database)?;
    let row = conn.query_row("SELECT models, decks FROM col", [], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let notes = conn
        .prepare("SELECT flds FROM notes ORDER BY id")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|item| {
            Value::Array(
                item.split('\u{1f}')
                    .map(|part| Value::String(String::from(part)))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    let decks = serde_json::from_str::<Value>(row.1.as_str())?;
    let models = serde_json::from_str::<Value>(row.0.as_str())?;
    let model = models
        .as_object()
        .and_then(|items| items.values().next())
        .cloned()
        .expect("saved apkg must contain one model");
    let deck = decks
        .as_object()
        .and_then(|items| {
            items.values().find(|item| {
                item.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name == "Kamishibai Deck")
            })
        })
        .cloned()
        .expect("saved apkg must contain the target deck");
    let mut names = names;
    names.sort();
    Ok(json!({
        "deck": {
            "id": deck["id"],
            "name": deck["name"],
        },
        "media": serde_json::from_str::<Value>(media.as_str())?,
        "model": {
            "fields": model["flds"].as_array().expect("model fields must exist").iter().map(|item| item["name"].clone()).collect::<Vec<_>>(),
            "id": model["id"].as_str().expect("model id must exist").parse::<i64>().expect("model id must be numeric"),
            "name": model["name"],
            "template": model["tmpls"][0].clone(),
        },
        "notes": notes,
        "zip_entries": names,
    }))
}

/// Stable deck identifiers match the frozen reference manifest.
#[test]
fn stable_deck_identifiers_match_the_frozen_reference_manifest() {
    let reference = apkg();
    assert_eq!(
        (
            StableId::new("Kamishibai Deck").value(),
            CardModel::new().model().id,
        ),
        (
            reference["deck"]["id"]
                .as_i64()
                .expect("deck id must exist"),
            reference["model"]["id"]
                .as_i64()
                .expect("model id must exist"),
        ),
        "stable deck identifiers no longer match the frozen reference manifest"
    );
}

/// Transcription formatting keeps the frozen slash semantics.
#[test]
fn transcription_formatting_keeps_the_frozen_slash_semantics() {
    assert_eq!(
        (
            Transcription::new("ˈtɛst").formatted(),
            Transcription::new("/λόγος/").formatted(),
            Transcription::new("").formatted(),
        ),
        (
            String::from("/ˈtɛst/"),
            String::from("/λόγος/"),
            String::new(),
        ),
        "transcription formatting no longer keeps the frozen slash semantics"
    );
}

/// HTML line-break formatting keeps the frozen newline semantics.
#[test]
fn html_line_break_formatting_keeps_the_frozen_newline_semantics() {
    assert_eq!(
        (
            HtmlLineBreaks::new("α\nβ").formatted(),
            HtmlLineBreaks::new("日本語").formatted(),
            HtmlLineBreaks::new("").formatted(),
        ),
        (
            String::from("α<br>β"),
            String::from("日本語"),
            String::new(),
        ),
        "HTML line-break formatting no longer keeps the frozen newline semantics"
    );
}

/// The card model keeps the frozen field order and template contract.
#[test]
fn the_card_model_keeps_the_frozen_field_order_and_template_contract() {
    let reference = apkg();
    let model = CardModel::new().model();
    assert_eq!(
        (
            model.id,
            model.name,
            model.fields,
            json!({
                "afmt": model.template.afmt,
                "bafmt": model.template.bafmt,
                "bfont": model.template.bfont,
                "bqfmt": model.template.bqfmt,
                "bsize": model.template.bsize,
                "did": model.template.did,
                "name": model.template.name,
                "ord": model.template.ord,
                "qfmt": model.template.qfmt,
            }),
        ),
        (
            reference["model"]["id"]
                .as_i64()
                .expect("reference model id must exist"),
            String::from(
                reference["model"]["name"]
                    .as_str()
                    .expect("reference model name must exist"),
            ),
            reference["model"]["fields"]
                .as_array()
                .expect("reference fields must exist")
                .iter()
                .map(|item| String::from(item.as_str().expect("field name must exist")))
                .collect::<Vec<_>>(),
            reference["model"]["template"].clone(),
        ),
        "the card model no longer keeps the frozen field order and template contract"
    );
}

/// The card model identifier matches the published model name.
#[test]
fn the_card_model_identifier_matches_the_published_model_name() {
    let model = CardModel::new().model();
    assert_eq!(
        model.id,
        StableId::new(model.name.clone()).value(),
        "the card model identifier no longer matches the published model name"
    );
}

/// Vocabulary notes keep the frozen first-note payload.
#[test]
fn vocabulary_notes_keep_the_frozen_first_note_payload() {
    let note = VocabularyNote::new(CardModel::new().model());
    let reference = apkg();
    let entry = entries()
        .into_iter()
        .next()
        .expect("reference entry must exist");
    assert_eq!(
        note.note(
            &entry,
            "[sound:9c0f6ba9a5a7.wav]",
            "<img src='51a07a17e75e.jpg' style='max-width: 100%; height: auto; border-radius: 10px'>",
        )
        .fields,
        reference["notes"][0]
            .as_array()
            .expect("reference note must exist")
            .iter()
            .map(|item| String::from(item.as_str().expect("note field must exist")))
            .collect::<Vec<_>>(),
        "vocabulary notes no longer keep the frozen first note payload"
    );
}

/// Deck attachment keeps insertion order while deduplicating paths.
#[test]
fn deck_attachment_keeps_insertion_order_while_deduplicating_paths() {
    let directory = TempDir::new().expect("temp directory must exist");
    let first = directory.path().join("α.wav");
    let second = directory.path().join("β.jpg");
    fs::write(&first, b"one").expect("first media fixture must exist");
    fs::write(&second, b"two").expect("second media fixture must exist");
    let mut deck = VocabularyDeck::new(
        StableId::new("Kamishibai Deck").value(),
        "Kamishibai Deck",
        VocabularyNote::new(CardModel::new().model()),
        Vec::<PathBuf>::new(),
    );
    deck.attach(first.clone());
    deck.attach(first);
    deck.attach(second.clone());
    assert_eq!(
        deck.media(),
        [
            second
                .parent()
                .expect("media parent must exist")
                .join("α.wav"),
            second
        ]
        .as_slice(),
        "deck attachment no longer keeps insertion order while deduplicating paths"
    );
}

/// Saved APKG archives keep the frozen structural snapshot.
#[test]
fn saved_apkg_archives_keep_the_frozen_structural_snapshot() -> Result<()> {
    let directory = TempDir::new()?;
    let output = directory.path().join("mixed-target.apkg");
    let media = media(
        directory.path(),
        &[
            "9c0f6ba9a5a7.wav",
            "51a07a17e75e.jpg",
            "24d2497f1d81.wav",
            "179104d071c6.jpg",
        ],
    );
    let reference = apkg();
    let entries = entries();
    let mut deck = VocabularyDeck::new(
        StableId::new("Kamishibai Deck").value(),
        "Kamishibai Deck",
        VocabularyNote::new(CardModel::new().model()),
        Vec::<PathBuf>::new(),
    );
    for path in &media {
        deck.attach(path.clone());
    }
    deck.add(
        &entries[0],
        "[sound:9c0f6ba9a5a7.wav]",
        "<img src='51a07a17e75e.jpg' style='max-width: 100%; height: auto; border-radius: 10px'>",
    );
    deck.add(
        &entries[1],
        "[sound:24d2497f1d81.wav]",
        "<img src='179104d071c6.jpg' style='max-width: 100%; height: auto; border-radius: 10px'>",
    );
    deck.save(&output)?;
    assert_eq!(
        manifest(&output)?,
        reference,
        "saved APKG archives no longer keep the frozen structural snapshot"
    );
    Ok(())
}
