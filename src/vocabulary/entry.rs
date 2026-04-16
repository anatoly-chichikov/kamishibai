use std::fmt::{Display, Formatter, Result as FmtResult};

use anyhow::{Result, bail};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// One strict vocabulary JSON document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyDocument {
    pub entries: Vec<VocabularyEntry>,
}

/// One strict vocabulary entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyEntry {
    pub term: NonEmptyText,
    pub meaning: NonEmptyText,
    pub pronunciation: NonEmptyText,
    pub transcription: NonEmptyText,
    pub importance: Importance,
    pub source: VocabularySource,
    pub target: VocabularyTarget,
}

/// One strict source-side sentence payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularySource {
    pub sentence: NonEmptyText,
    pub lang: LanguageCode,
    pub highlight: NonEmptyText,
    pub hint: NonEmptyText,
    pub context: NonEmptyText,
}

/// One strict target-side sentence payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyTarget {
    pub sentence: NonEmptyText,
    pub lang: LanguageCode,
}

/// One non-empty text value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NonEmptyText(String);

/// One non-empty language code.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LanguageCode(String);

/// One bounded importance value.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Importance(u8);

impl NonEmptyText {
    /// Create one validated non-empty text value.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            bail!("Expected a non-empty string");
        }
        Ok(Self(value))
    }

    /// Return the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl LanguageCode {
    /// Create one validated language code.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        Ok(Self(NonEmptyText::new(value)?.0))
    }

    /// Return the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Importance {
    /// Create one validated importance value.
    pub fn new(value: u8) -> Result<Self> {
        if !(1..=10).contains(&value) {
            bail!("Expected an integer from 1 to 10");
        }
        Ok(Self(value))
    }

    /// Return the numeric score.
    pub fn value(&self) -> u8 {
        self.0
    }
}

impl Display for NonEmptyText {
    /// Write the contained string value.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.0.as_str())
    }
}

impl Display for LanguageCode {
    /// Write the contained language code.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.0.as_str())
    }
}

impl Display for Importance {
    /// Write the contained score.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        write!(formatter, "{}", self.0)
    }
}

impl<'de> Deserialize<'de> for NonEmptyText {
    /// Return one validated non-empty string from JSON.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.trim().is_empty() {
            return Err(de::Error::custom("expected a non-empty string"));
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for LanguageCode {
    /// Return one validated language code from JSON.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(NonEmptyText::deserialize(deserializer)?.0))
    }
}

impl<'de> Deserialize<'de> for Importance {
    /// Return one validated importance score from JSON.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        if !(1..=10).contains(&value) {
            return Err(de::Error::custom("expected an integer from 1 to 10"));
        }
        Ok(Self(value))
    }
}
