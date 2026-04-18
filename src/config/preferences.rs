use serde::{Deserialize, Serialize};

use super::DEFAULT_MY_LANGUAGE;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Preferences {
    pub my_language: String,
}

impl Default for Preferences {
    /// Return the first-run preference bundle with `my_language` set to `en`.
    fn default() -> Self {
        Self {
            my_language: String::from(DEFAULT_MY_LANGUAGE),
        }
    }
}

impl Preferences {
    /// Create one preference bundle from a chosen support language.
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
        }
    }

    /// Return a preference bundle with a new `my_language` value.
    pub fn adopt(&self, language: impl Into<String>) -> Self {
        Self {
            my_language: language.into(),
        }
    }
}
