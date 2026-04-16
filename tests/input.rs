//! Tests for strict input parsing.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use kamishibai::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};
use tempfile::TempDir;

/// Write one JSON document to a temporary file.
fn write(body: &str) -> Result<(TempDir, PathBuf)> {
    let directory = TempDir::new()?;
    let path = directory.path().join("kamishibai.json");
    fs::write(&path, body)?;
    Ok((directory, path))
}

/// Return one temporary path for one JSON string.
fn file(body: &str) -> Result<(TempDir, PathBuf)> {
    write(body)
}

/// Return one validated text fixture.
fn text(value: &str) -> NonEmptyText {
    NonEmptyText::new(value).expect("test text must be valid")
}

/// Return one validated language fixture.
fn code(value: &str) -> LanguageCode {
    LanguageCode::new(value).expect("test language must be valid")
}

/// Return one validated importance fixture.
fn score(value: u8) -> Importance {
    Importance::new(value).expect("test importance must be valid")
}

/// Valid entries keep the exact strict output shape.
#[test]
fn valid_entries_map_into_the_full_strict_shape() -> Result<()> {
    let (_directory, path) = file(
        r#"{"entries":[{"term":"кошка","meaning":"cat","pronunciation":"kæt","transcription":"kat","importance":7,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert_eq!(
        VocabularyDocument::load(&path)?,
        VocabularyDocument {
            entries: vec![VocabularyEntry {
                term: text("кошка"),
                meaning: text("cat"),
                pronunciation: text("kæt"),
                transcription: text("kat"),
                importance: score(7),
                source: VocabularySource {
                    sentence: text("Кошка спит на окне"),
                    lang: code("ru"),
                    highlight: text("Кошка"),
                    hint: text("домашнее животное"),
                    context: text("нейтральный стиль"),
                },
                target: VocabularyTarget {
                    sentence: text("The cat is sleeping on the windowsill"),
                    lang: code("en"),
                },
            }],
        },
        "valid entry did not map into the expected strict shape"
    );
    Ok(())
}

/// Missing required fields fail the whole document.
#[test]
fn missing_required_fields_fail_the_whole_document() -> Result<()> {
    let (_directory, path) = file(
        r#"{"entries":[{"term":"кошка","pronunciation":"kæt","transcription":"kat","importance":7,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert!(
        VocabularyDocument::load(&path)
            .unwrap_err()
            .to_string()
            .contains("missing field `meaning`"),
        "missing required fields no longer fail the whole document"
    );
    Ok(())
}

/// Empty strings fail the whole document.
#[test]
fn empty_strings_fail_the_whole_document() -> Result<()> {
    let (_directory, path) = file(
        r#"{"entries":[{"term":"кошка","meaning":"cat","pronunciation":" ","transcription":"kat","importance":7,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert!(
        VocabularyDocument::load(&path)
            .unwrap_err()
            .to_string()
            .contains("expected a non-empty string"),
        "empty strings no longer fail the whole document"
    );
    Ok(())
}

/// Unknown fields fail the whole document.
#[test]
fn unknown_fields_fail_the_whole_document() -> Result<()> {
    let (_directory, path) = file(
        r#"{"entries":[{"term":"кошка","meaning":"cat","pronunciation":"kæt","transcription":"kat","importance":7,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль","tone":"soft"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert!(
        VocabularyDocument::load(&path)
            .unwrap_err()
            .to_string()
            .contains("unknown field `tone`"),
        "unknown fields no longer fail the whole document"
    );
    Ok(())
}

/// Invalid importance values fail the whole document.
#[test]
fn invalid_importance_values_fail_the_whole_document() -> Result<()> {
    let (_directory, path) = file(
        r#"{"entries":[{"term":"кошка","meaning":"cat","pronunciation":"kæt","transcription":"kat","importance":11,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert!(
        VocabularyDocument::load(&path)
            .unwrap_err()
            .to_string()
            .contains("expected an integer from 1 to 10"),
        "invalid importance values no longer fail the whole document"
    );
    Ok(())
}

/// Non-object roots fail with the frozen error wording.
#[test]
fn non_object_roots_raise_the_frozen_error_message() -> Result<()> {
    let (_directory, path) = file(r#"[{"term":"broken"}]"#)?;
    assert_eq!(
        VocabularyDocument::load(&path).unwrap_err().to_string(),
        format!(
            "Expected a JSON object in '{}' but found array",
            path.display()
        ),
        "non-object roots did not raise the frozen validation message"
    );
    Ok(())
}

/// Missing entries arrays fail with the strict error wording.
#[test]
fn missing_entries_arrays_raise_the_strict_error_message() -> Result<()> {
    let (_directory, path) = file(r#"{}"#)?;
    assert_eq!(
        VocabularyDocument::load(&path).unwrap_err().to_string(),
        format!(
            "Invalid document in '{}': missing field `entries`",
            path.display()
        ),
        "missing entries arrays did not raise the strict validation message"
    );
    Ok(())
}

/// Empty entry lists fail with the strict error wording.
#[test]
fn empty_entry_lists_raise_the_strict_error_message() -> Result<()> {
    let (_directory, path) = file(r#"{"entries":[]}"#)?;
    assert_eq!(
        VocabularyDocument::load(&path).unwrap_err().to_string(),
        format!("Expected at least one entry in '{}'", path.display()),
        "empty entry lists did not raise the strict validation message"
    );
    Ok(())
}
