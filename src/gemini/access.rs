//! Runtime access policy for Gemini credentials and key validation.

use anyhow::Result;
use std::sync::Arc;

use super::{GeminiClient, HttpTransport};
use crate::application::KeyValidation;
use crate::config::default_store;
use crate::runtime::locations::SystemContext;

/// Selects the documented credential precedence for one delivery surface.
#[derive(Clone, Debug)]
enum KeyLookup {
    Saved,
    Environment,
    Explicit(Arc<GeminiClient<HttpTransport>>),
    #[cfg(test)]
    Unavailable,
}

/// Opens Gemini clients using the credential policy of one workflow.
#[derive(Clone, Debug)]
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

    /// Bind one caller-owned client without reading environment or preferences.
    #[must_use]
    pub(crate) fn from_client(client: GeminiClient<HttpTransport>) -> Self {
        Self::new(KeyLookup::Explicit(Arc::new(client)))
    }

    #[cfg(test)]
    /// Build access that deterministically refuses to open a client.
    pub(crate) fn unavailable() -> Self {
        Self::new(KeyLookup::Unavailable)
    }

    /// Open a client after resolving the latest saved preferences.
    pub(crate) fn client(&self) -> Result<GeminiClient<HttpTransport>> {
        match &self.keys {
            KeyLookup::Explicit(client) => Ok(client.as_ref().clone()),
            KeyLookup::Saved => {
                let saved = default_store(&SystemContext)?.read()?.api_key;
                GeminiClient::from_saved(saved.as_deref())
            }
            KeyLookup::Environment if env_key_present() => GeminiClient::from_env_or_saved(None),
            KeyLookup::Environment => {
                let saved = default_store(&SystemContext)?.read()?.api_key;
                GeminiClient::from_env_or_saved(saved.as_deref())
            }
            #[cfg(test)]
            KeyLookup::Unavailable => anyhow::bail!("Gemini access unavailable in test"),
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
