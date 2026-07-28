//! Runtime access policy for Gemini credentials and key validation.

use anyhow::Result;

use super::{GeminiClient, HttpTransport};
use crate::application::KeyValidation;
use crate::config::default_store;
use crate::runtime::locations::SystemContext;

/// Selects the documented credential precedence for one delivery surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyLookup {
    Saved,
    Environment,
}

/// Opens Gemini clients using the credential policy of one workflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeminiAccess {
    keys: KeyLookup,
}

impl GeminiAccess {
    /// Build access with one explicit credential policy.
    fn new(keys: KeyLookup) -> Self {
        Self { keys }
    }

    /// Build access for the TUI, which uses the validated saved key.
    #[must_use]
    pub(crate) fn interactive() -> Self {
        Self::new(KeyLookup::Saved)
    }

    /// Build access for console sessions, where the environment wins.
    #[must_use]
    pub(crate) fn console() -> Self {
        Self::new(KeyLookup::Environment)
    }

    /// Open a client after resolving the latest saved preferences.
    pub(crate) fn client(&self) -> Result<GeminiClient<HttpTransport>> {
        match self.keys {
            KeyLookup::Saved => {
                let saved = default_store(&SystemContext)?.read()?.api_key;
                GeminiClient::from_saved(saved.as_deref())
            }
            KeyLookup::Environment if env_key_present() => GeminiClient::from_env_or_saved(None),
            KeyLookup::Environment => {
                let saved = default_store(&SystemContext)?.read()?.api_key;
                GeminiClient::from_env_or_saved(saved.as_deref())
            }
        }
    }
}

fn env_key_present() -> bool {
    std::env::var("GEMINI_API_KEY")
        .ok()
        .is_some_and(|key| !key.trim().is_empty())
}

impl KeyValidation for GeminiAccess {
    fn check_key(&self, key: &str) -> Result<()> {
        GeminiClient::new(key, HttpTransport::credential()).validate_key()
    }
}
