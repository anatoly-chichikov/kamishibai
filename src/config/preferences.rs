use serde::{Deserialize, Serialize};

use super::DEFAULT_MY_LANGUAGE;

/// Persisted setup choices loaded before the TUI starts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    /// User-facing support language code last confirmed by the user.
    pub my_language: String,
    /// Whether `my_language` came from an explicit user choice.
    pub my_language_confirmed: bool,
    /// Saved Gemini API key, when the user chose local persistence.
    pub api_key: Option<String>,
}

impl Default for Preferences {
    /// Return the first-run preference bundle with an unconfirmed language
    /// choice and no stored Gemini API key.
    fn default() -> Self {
        Self {
            my_language: String::from(DEFAULT_MY_LANGUAGE),
            my_language_confirmed: false,
            api_key: None,
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
        }
    }

    /// Return a preference bundle with a new `my_language` value.
    pub fn adopt(&self, language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
            my_language_confirmed: true,
            api_key: self.api_key.clone(),
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
