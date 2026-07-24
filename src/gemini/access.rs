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
        let saved_key = default_store(&SystemContext)
            .ok()
            .and_then(|store| store.read().ok())
            .and_then(|preferences| preferences.api_key);
        match self.keys {
            KeyLookup::Saved => GeminiClient::from_saved(saved_key.as_deref()),
            KeyLookup::Environment => GeminiClient::from_env_or_saved(saved_key.as_deref()),
        }
    }
}

impl KeyValidation for GeminiAccess {
    fn check_key(&self, key: &str) -> Result<()> {
        GeminiClient::new(key, HttpTransport::new()).validate_key()
    }
}
