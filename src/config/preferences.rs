use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

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
}

impl fmt::Debug for Preferences {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Preferences")
            .field("my_language", &self.my_language)
            .field("my_language_confirmed", &self.my_language_confirmed)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("sentences", &self.sentences)
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
        }
    }

    /// Return a preference bundle with a new `my_language` value.
    pub fn adopt(&self, language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
            my_language_confirmed: true,
            api_key: self.api_key.clone(),
            sentences: self.sentences.clone(),
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
        }
    }

    /// Return a preference bundle with the API key cleared.
    pub fn without_api_key(&self) -> Self {
        Self {
            my_language: self.my_language.clone(),
            my_language_confirmed: self.my_language_confirmed,
            api_key: None,
            sentences: self.sentences.clone(),
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
        }
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
