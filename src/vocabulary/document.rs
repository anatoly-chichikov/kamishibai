//! Input loading for schema-driven vocabulary entries.

use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::vocabulary::VocabularyDocument;

impl VocabularyDocument {
    /// Load one strict vocabulary document from the filesystem.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
        if !data.is_object() {
            bail!(
                "Expected a JSON object in '{}' but found {}",
                path.display(),
                kind(&data)
            );
        }
        let document = serde_json::from_value::<Self>(data)
            .map_err(|error| anyhow!("Invalid document in '{}': {}", path.display(), error))?;
        if document.entries.is_empty() {
            bail!("Expected at least one entry in '{}'", path.display());
        }
        Ok(document)
    }
}

/// Return the JSON-style type label used in input errors.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(item) if item.is_i64() || item.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
