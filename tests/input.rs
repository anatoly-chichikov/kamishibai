//! Tests for normalized input parsing.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use kamishibai::input::{NormalizedEntry, Vocabulary, VocabularyMapping};
use tempfile::TempDir;

/// Write one JSON document to a temporary file.
fn write(body: &str) -> Result<(TempDir, PathBuf)> {
    let directory = TempDir::new()?;
    let path = directory.path().join("kamishibai.json");
    fs::write(&path, body)?;
    Ok((directory, path))
}

/// Return the shared vocabulary reader for one JSON string.
fn reader(body: &str) -> Result<(TempDir, Vocabulary<VocabularyMapping>)> {
    let (directory, path) = write(body)?;
    Ok((directory, Vocabulary::new(path, VocabularyMapping)))
}

/// Valid entries keep the exact normalized output shape.
#[test]
fn valid_entries_map_into_the_full_normalized_shape() -> Result<()> {
    let (_directory, vocabulary) = reader(
        r#"{"entries":[{"term":"кошка","meaning":"cat","pronunciation":"kæt","transcription":"kat","importance":7,"source":{"sentence":"Кошка спит на окне","lang":"ru","highlight":"Кошка","hint":"домашнее животное","context":"нейтральный стиль"},"target":{"sentence":"The cat is sleeping on the windowsill","lang":"en"}}]}"#,
    )?;
    assert_eq!(
        vocabulary.entries(None)?,
        vec![NormalizedEntry {
            word: String::from("кошка"),
            pronunciation: String::from("kæt"),
            translation: String::from("cat"),
            example: String::from("The cat is sleeping on the windowsill"),
            source_lang: String::from("ru"),
            target_lang: String::from("en"),
            sentence: String::from("Кошка спит на окне"),
            highlight: String::from("Кошка"),
            hint: String::from("домашнее животное"),
            context: String::from("нейтральный стиль"),
            importance: String::from("7"),
            transcription: String::from("kat")
        }],
        "valid entry did not map into the expected normalized shape"
    );
    Ok(())
}

/// Mixed valid and invalid rows keep only the valid entries.
#[test]
fn mixed_rows_keep_only_valid_entries() -> Result<()> {
    let (_directory, vocabulary) = reader(
        r#"{"entries":[{"term":"broken","source":{"sentence":"Есть источник","lang":"ru"},"target":{"lang":"en"}},{"term":"valid","source":{"sentence":"Валидное","lang":"ru"},"target":{"sentence":"Valid","lang":"en"}},{"source":{"sentence":"Без term","lang":"ru"},"target":{"sentence":"No term","lang":"en"}}]}"#,
    )?;
    assert_eq!(
        vocabulary.entries(None)?.len(),
        1,
        "invalid rows were not filtered out of the normalized result"
    );
    Ok(())
}

/// Null optionals coalesce into empty strings.
#[test]
fn null_optionals_become_empty_strings() -> Result<()> {
    let (_directory, vocabulary) = reader(
        r#"{"entries":[{"term":"test","pronunciation":null,"importance":null,"source":{"sentence":"Предложение","lang":"ru","context":null},"target":{"sentence":"Sentence","lang":"en"}}]}"#,
    )?;
    assert_eq!(
        vocabulary.entries(None)?,
        vec![NormalizedEntry {
            word: String::from("test"),
            pronunciation: String::new(),
            translation: String::new(),
            example: String::from("Sentence"),
            source_lang: String::from("ru"),
            target_lang: String::from("en"),
            sentence: String::from("Предложение"),
            highlight: String::new(),
            hint: String::new(),
            context: String::new(),
            importance: String::new(),
            transcription: String::new()
        }],
        "null optionals did not coalesce into empty strings"
    );
    Ok(())
}

/// Non-object roots fail with the frozen error wording.
#[test]
fn non_object_roots_raise_the_frozen_error_message() -> Result<()> {
    let (_directory, vocabulary) = reader(r#"[{"term":"broken"}]"#)?;
    assert_eq!(
        vocabulary.document().unwrap_err().to_string(),
        format!(
            "Expected a JSON object in '{}' but found list",
            vocabulary.path().display()
        ),
        "non-object roots did not raise the frozen validation message"
    );
    Ok(())
}

/// Missing entries arrays fail with the frozen error wording.
#[test]
fn missing_entries_arrays_raise_the_frozen_error_message() -> Result<()> {
    let (_directory, vocabulary) = reader(r#"{"items":[]}"#)?;
    assert_eq!(
        vocabulary.document().unwrap_err().to_string(),
        format!(
            "Expected an 'entries' array in '{}'",
            vocabulary.path().display()
        ),
        "missing entries arrays did not raise the frozen validation message"
    );
    Ok(())
}

/// Empty valid results fail with the frozen error wording.
#[test]
fn empty_valid_results_raise_the_frozen_error_message() -> Result<()> {
    let (_directory, vocabulary) = reader(
        r#"{"entries":[{"term":"broken","source":{"sentence":"Есть источник","lang":"ru"},"target":{"lang":"en"}}]}"#,
    )?;
    assert_eq!(
        vocabulary.entries(None).unwrap_err().to_string(),
        format!(
            "No valid entries found in '{}'; each entry requires 'term', 'source.sentence', 'source.lang', 'target.sentence', and 'target.lang'",
            vocabulary.path().display()
        ),
        "empty normalized results did not raise the frozen validation message"
    );
    Ok(())
}
