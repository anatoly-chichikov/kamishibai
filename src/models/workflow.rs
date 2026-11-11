//! Compose an external provider and plugged parts with the shared card and publication pipeline.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};

use super::{Look, ModelProvider};
use crate::application::{CardUseCases, GenerationCostLedger};
use crate::gemini::{EmbeddedPrompts, GeminiClient, GeminiProfile, PromptPolicy};

/// Everything an application plugs into the canonical workflow besides its model.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone)]
pub struct Parts {
    look: Look,
    prompts: Arc<dyn PromptPolicy>,
}

impl Parts {
    /// Bind the picture parts and the prompt policy an application supplies.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn new(look: Look, prompts: Arc<dyn PromptPolicy>) -> Self {
        Self { look, prompts }
    }
}

impl Default for Parts {
    fn default() -> Self {
        Self::new(Look::default(), Arc::new(EmbeddedPrompts))
    }
}

/// Resolve the exact cache namespace used by an external-provider workflow.
///
/// Identity must describe every provider endpoint, model, generation setting,
/// plugged part, and prompt-policy revision that can affect output, excluding
/// credentials. The core combines it with its own canonical prompt and schema identity.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn cache_root(cache: &Path, identity: &str) -> Result<PathBuf> {
    Ok(cache
        .join("profiles")
        .join(profile(identity, Arc::new(EmbeddedPrompts))?.identity()))
}

/// Reuse canonical prompts, scenes, retries, media checks, and study publication.
///
/// Call from a blocking worker with paths isolated per tenant and mutable job.
/// Provider identity must change with model configuration, plugged parts, or prompt
/// behavior. Every provider response reaches the ledger before output decoding,
/// including invalid responses and requests whose price is unknown. The provider
/// owns authorization immediately before each outbound request.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn workflow(
    provider: Arc<dyn ModelProvider>,
    parts: Parts,
    identity: &str,
    cache: PathBuf,
    output: PathBuf,
    ledger: Option<Arc<dyn GenerationCostLedger>>,
) -> Result<impl CardUseCases + Clone> {
    let client =
        GeminiClient::from_provider(provider, profile(identity, parts.prompts)?, parts.look);
    Ok(crate::gemini::workflow(client, cache, output, ledger))
}

fn profile(identity: &str, prompts: Arc<dyn PromptPolicy>) -> Result<GeminiProfile> {
    if identity.trim().is_empty() {
        bail!("external provider identity must not be empty");
    }
    Ok(GeminiProfile::external(identity, prompts))
}
