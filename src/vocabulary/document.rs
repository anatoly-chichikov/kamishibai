//! Input parsing for schema-driven vocabulary entries.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::vocabulary::VocabularyEntry;

/// Map raw JSON rows into normalized entries.
pub trait FieldMapping {
    /// Return one normalized entry or no entry when the row is invalid.
    fn map(&self, row: &Value) -> Option<VocabularyEntry>;
}

/// Map schema-driven JSON rows into the normalized entry shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct VocabularyMapping;

impl FieldMapping for VocabularyMapping {
    /// Return one normalized entry or no entry when the row is invalid.
    fn map(&self, row: &Value) -> Option<VocabularyEntry> {
        let record = row.as_object()?;
        let source = record.get("source")?.as_object()?;
        let target = record.get("target")?.as_object()?;
        if !truthy(record.get("term"))
            || !truthy(source.get("sentence"))
            || !truthy(source.get("lang"))
            || !truthy(target.get("sentence"))
            || !truthy(target.get("lang"))
        {
            return None;
        }
        Some(VocabularyEntry {
            word: scalar(record.get("term"))?,
            pronunciation: optional(record.get("pronunciation")),
            translation: optional(record.get("meaning")),
            example: scalar(target.get("sentence"))?,
            source_lang: scalar(source.get("lang"))?,
            target_lang: scalar(target.get("lang"))?,
            sentence: scalar(source.get("sentence"))?,
            highlight: optional(source.get("highlight")),
            hint: optional(source.get("hint")),
            context: optional(source.get("context")),
            importance: optional(record.get("importance")),
            transcription: optional(record.get("transcription")),
        })
    }
}

/// Read and normalize one vocabulary JSON document.
#[derive(Clone, Debug)]
pub struct VocabularyDocument<M> {
    path: PathBuf,
    mapping: M,
}

impl<M> VocabularyDocument<M>
where
    M: FieldMapping,
{
    /// Create a vocabulary reader for one filesystem path.
    pub fn new(path: impl Into<PathBuf>, mapping: M) -> Self {
        Self {
            path: path.into(),
            mapping,
        }
    }

    /// Load and validate the root JSON document.
    pub fn document(&self) -> Result<Value> {
        let data = serde_json::from_str::<Value>(&fs::read_to_string(&self.path)?)?;
        if !data.is_object() {
            bail!(
                "Expected a JSON object in '{}' but found {}",
                self.path.display(),
                kind(&data)
            );
        }
        if !matches!(data.get("entries"), Some(Value::Array(_))) {
            bail!("Expected an 'entries' array in '{}'", self.path.display());
        }
        Ok(data)
    }

    /// Load, filter, and return normalized entries.
    pub fn entries(&self, document: Option<&Value>) -> Result<Vec<VocabularyEntry>> {
        let owned;
        let data = match document {
            Some(value) => value,
            None => {
                owned = self.document()?;
                &owned
            }
        };
        let items = data
            .get("entries")
            .and_then(Value::as_array)
            .expect("validated document must contain an entries array");
        let result = items
            .iter()
            .filter_map(|row| self.mapping.map(row))
            .collect::<Vec<_>>();
        if result.is_empty() {
            bail!(
                "No valid entries found in '{}'; each entry requires 'term', 'source.sentence', 'source.lang', 'target.sentence', and 'target.lang'",
                self.path.display()
            );
        }
        Ok(result)
    }

    /// Return the source document path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None => false,
        Some(Value::Null) => false,
        Some(Value::Bool(item)) => *item,
        Some(Value::Number(item)) => item
            .as_i64()
            .map(|value| value != 0)
            .or_else(|| item.as_u64().map(|value| value != 0))
            .or_else(|| item.as_f64().map(|value| value != 0.0))
            .unwrap_or(false),
        Some(Value::String(item)) => !item.is_empty(),
        Some(Value::Array(item)) => !item.is_empty(),
        Some(Value::Object(item)) => !item.is_empty(),
    }
}

fn scalar(value: Option<&Value>) -> Option<String> {
    if !truthy(value) {
        return None;
    }
    value.map(string)
}

fn optional(value: Option<&Value>) -> String {
    match value {
        Some(item) if truthy(Some(item)) => string(item),
        _ => String::new(),
    }
}

fn string(value: &Value) -> String {
    match value {
        Value::String(item) => item.clone(),
        Value::Number(item) => item.to_string(),
        Value::Bool(item) => item.to_string(),
        Value::Null => String::new(),
        Value::Array(item) => serde_json::to_string(item).expect("array values must serialize"),
        Value::Object(item) => serde_json::to_string(item).expect("object values must serialize"),
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(item) if item.is_i64() || item.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
