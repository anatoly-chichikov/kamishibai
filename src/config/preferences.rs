use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;

use crate::languages::catalog;
use crate::session::SentenceBatchSettings;

use super::DEFAULT_MY_LANGUAGE;

/// Persisted setup choices loaded before the TUI starts.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    /// User-facing support language code last confirmed by the user.
    pub my_language: String,
    /// Whether `my_language` came from an explicit user choice.
    pub my_language_confirmed: bool,
    /// Saved Gemini API key, when the user chose local persistence.
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sentences: BTreeMap<String, SentenceBatchSettings>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_learning_languages"
    )]
    learning_languages: Vec<LearningLanguage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LearningLanguage {
    language: String,
    last_selected_at: i64,
}

fn deserialize_learning_languages<'de, D>(
    deserializer: D,
) -> Result<Vec<LearningLanguage>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(ordered_learning_languages(Vec::deserialize(deserializer)?))
}

fn ordered_learning_languages(mut languages: Vec<LearningLanguage>) -> Vec<LearningLanguage> {
    languages.sort_by_key(|language| Reverse(language.last_selected_at));
    for language in &mut languages {
        language.language = language.language.trim().to_ascii_uppercase();
    }
    let mut unique = BTreeSet::new();
    languages.retain(|language| unique.insert(language.language.clone()));
    languages
}

impl fmt::Debug for Preferences {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Preferences")
            .field("my_language", &self.my_language)
            .field("my_language_confirmed", &self.my_language_confirmed)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("sentences", &self.sentences)
            .field("learning_languages", &self.learning_languages)
            .finish()
    }
}

impl Default for Preferences {
    /// Return the first-run preference bundle with an unconfirmed language
    /// choice and no stored Gemini API key.
    fn default() -> Self {
        Self {
            my_language: String::from(DEFAULT_MY_LANGUAGE),
            my_language_confirmed: false,
            api_key: None,
            sentences: BTreeMap::new(),
            learning_languages: Vec::new(),
        }
    }
}

impl Preferences {
    /// Create one preference bundle from a chosen support language. The
    /// Gemini API key is left empty until the user pastes one through the
    /// Welcome screen.
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
            my_language_confirmed: true,
            api_key: None,
            sentences: BTreeMap::new(),
            learning_languages: Vec::new(),
        }
    }

    /// Return a preference bundle with a new `my_language` value.
    pub fn adopt(&self, language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
            my_language_confirmed: true,
            api_key: self.api_key.clone(),
            sentences: self.sentences.clone(),
            learning_languages: self.learning_languages.clone(),
        }
    }

    /// Return a preference bundle with the API key field set. An empty input
    /// is normalised to `None` so the persisted JSON stays clean.
    pub fn with_api_key(&self, key: impl Into<String>) -> Self {
        let key: String = key.into();
        Self {
            my_language: self.my_language.clone(),
            my_language_confirmed: self.my_language_confirmed,
            api_key: if key.is_empty() { None } else { Some(key) },
            sentences: self.sentences.clone(),
            learning_languages: self.learning_languages.clone(),
        }
    }

    /// Return a preference bundle with the API key cleared.
    pub fn without_api_key(&self) -> Self {
        Self {
            my_language: self.my_language.clone(),
            my_language_confirmed: self.my_language_confirmed,
            api_key: None,
            sentences: self.sentences.clone(),
            learning_languages: self.learning_languages.clone(),
        }
    }

    /// Return the generation guidance saved for one learning language, or the
    /// unconstrained best-fit policy when that language has no override.
    #[must_use]
    pub fn guidance(&self, learning: &str) -> SentenceBatchSettings {
        self.saved_guidance(learning).unwrap_or_default()
    }

    /// Return the explicit generation-guidance override for one learning
    /// language, preserving genuine absence for session migration decisions.
    #[must_use]
    pub(crate) fn saved_guidance(&self, learning: &str) -> Option<SentenceBatchSettings> {
        let key = learning.trim().to_ascii_uppercase();
        self.sentences.get(key.as_str()).copied().or_else(|| {
            self.sentences.iter().find_map(|(code, settings)| {
                code.eq_ignore_ascii_case(key.as_str()).then_some(*settings)
            })
        })
    }

    /// Return preferences remembering one learning language's generation
    /// guidance. Restoring both axes to best fit removes the override.
    #[must_use]
    pub fn remember(&self, learning: &str, settings: SentenceBatchSettings) -> Self {
        let key = learning.trim().to_ascii_uppercase();
        assert!(
            !key.is_empty(),
            "invariant: generation guidance requires a learning language"
        );
        let mut sentences = self.sentences.clone();
        sentences.retain(|code, _| !code.eq_ignore_ascii_case(key.as_str()));
        if settings != SentenceBatchSettings::default() {
            sentences.insert(key, settings);
        }
        Self {
            my_language: self.my_language.clone(),
            my_language_confirmed: self.my_language_confirmed,
            api_key: self.api_key.clone(),
            sentences,
            learning_languages: self.learning_languages.clone(),
        }
    }

    /// Remember an explicit learning-language selection with its Unix timestamp
    /// in seconds, retaining only its latest choice and sorting newest first.
    #[must_use]
    pub fn with_learning_language(&self, language: &str, timestamp: OffsetDateTime) -> Self {
        let language = catalog()
            .resolve(language.trim())
            .expect("invariant: learning history requires a supported language")
            .to_string();
        let mut learning_languages = vec![LearningLanguage {
            language: language.clone(),
            last_selected_at: timestamp.unix_timestamp(),
        }];
        learning_languages.extend(
            self.learning_languages
                .iter()
                .filter(|entry| entry.language != language)
                .cloned(),
        );
        Self {
            my_language: self.my_language.clone(),
            my_language_confirmed: self.my_language_confirmed,
            api_key: self.api_key.clone(),
            sentences: self.sentences.clone(),
            learning_languages: ordered_learning_languages(learning_languages),
        }
    }

    /// List previously selected supported learning languages newest first;
    /// unsupported saved codes remain stored but cannot appear in a picker.
    #[must_use]
    pub fn recent_learning(&self) -> Vec<String> {
        self.learning_languages
            .iter()
            .filter_map(|entry| catalog().resolve(entry.language.as_str()).ok())
            .map(|language| language.to_string())
            .collect()
    }

    /// Return whether startup still needs an explicit language confirmation.
    #[must_use]
    pub fn requires_language_choice(&self) -> bool {
        !self.my_language_confirmed
    }

    /// Return the support language startup may trust before showing the TUI.
    #[must_use]
    pub fn startup_language(&self) -> &str {
        if self.requires_language_choice() {
            DEFAULT_MY_LANGUAGE
        } else {
            self.my_language.as_str()
        }
    }
}
