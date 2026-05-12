use serde::{Deserialize, Serialize};

use super::DEFAULT_MY_LANGUAGE;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub my_language: String,
    pub api_key: Option<String>,
}

impl Default for Preferences {
    /// Return the first-run preference bundle with `my_language` set to `en`
    /// and no stored Gemini API key.
    fn default() -> Self {
        Self {
            my_language: String::from(DEFAULT_MY_LANGUAGE),
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
            api_key: None,
        }
    }

    /// Return a preference bundle with a new `my_language` value.
    pub fn adopt(&self, language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
            api_key: self.api_key.clone(),
        }
    }

    /// Return a preference bundle with the API key field set. An empty input
    /// is normalised to `None` so the persisted JSON stays clean.
    pub fn with_api_key(&self, key: impl Into<String>) -> Self {
        let key: String = key.into();
        Self {
            my_language: self.my_language.clone(),
            api_key: if key.is_empty() { None } else { Some(key) },
        }
    }
}
