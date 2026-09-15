//! Immutable model and prompt policy for one explicitly configured Gemini client.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::sync::Arc;

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const PROFILE_VERSION: &str = "kamishibai-gemini-profile-v1";

/// Identifies one provider operation without exposing delivery-specific controls.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GenerationStage {
    /// Understand a fresh vocabulary batch.
    Intake,
    /// Add or correct reviewed senses.
    Senses,
    /// Produce initial card metadata.
    Metadata,
    /// Rewrite existing card metadata.
    Correction,
    /// Check pronunciation and transcription.
    Phonetics,
    /// Extract features used to choose a scene layout.
    Features,
    /// Compose a scene inside its selected layout.
    Scene,
    /// Render the scene as an image.
    Picture,
    /// Judge whether an image reveals the answer.
    Recall,
    /// Judge whether an image preserves its scene requirements.
    Fidelity,
    /// Inspect enlarged crops for literal writing.
    Zoom,
    /// Judge visible text for languages without the OCR route.
    Text,
    /// Synthesize spoken pronunciation.
    Speech,
}

impl GenerationStage {
    fn model(self) -> &'static str {
        match self {
            Self::Picture => "gemini-3.1-flash-image",
            Self::Speech => "gemini-3.1-flash-tts-preview",
            _ => "gemini-3.8-flash",
        }
    }
}

/// Selects models per operation while retaining built-in choices for omitted stages.
///
/// Start with `StageModels::default()` and call `with_model` for each override;
/// every other stage retains its built-in model. Every response records its
/// request and usage even when the model price is unknown. Mixed estimates
/// retain the known subtotal and explicitly mark the total as incomplete.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StageModels {
    overrides: BTreeMap<GenerationStage, String>,
}

impl StageModels {
    /// Return a selection with one stage changed to a Gemini-compatible model identifier.
    pub fn with_model(mut self, stage: GenerationStage, model: impl Into<String>) -> Result<Self> {
        let model = model.into();
        if model.is_empty()
            || !model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            bail!("Gemini model must be a nonempty identifier without path or query characters");
        }
        if model == stage.model() {
            self.overrides.remove(&stage);
        } else {
            self.overrides.insert(stage, model);
        }
        Ok(self)
    }

    pub(super) fn resolve(&self, stage: GenerationStage) -> &str {
        self.overrides
            .get(&stage)
            .map(String::as_str)
            .unwrap_or_else(|| stage.model())
    }
}

/// Adapts a fully rendered prompt while preserving the request's schema and media.
///
/// Implementations must be deterministic and immutable for the profile revision.
/// Change that revision whenever prompt behavior changes. The resulting prompt
/// must preserve the stage's output contract; existing decoders still validate it.
pub trait PromptPolicy: Send + Sync {
    /// Return the prompt to send for one stage, or fail before a provider request.
    fn render(&self, stage: GenerationStage, prompt: &str) -> Result<String>;
}

/// Keeps the built-in rendered prompts unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmbeddedPrompts;

impl PromptPolicy for EmbeddedPrompts {
    fn render(&self, _stage: GenerationStage, prompt: &str) -> Result<String> {
        Ok(String::from(prompt))
    }
}

/// Binds one endpoint, stage model selection, and versioned prompt policy.
///
/// This profile never reads the environment or local preferences. Its identity
/// separates caches across configurations and excludes API credentials.
#[derive(Clone)]
pub struct GeminiProfile {
    endpoint: String,
    models: StageModels,
    prompts: Arc<dyn PromptPolicy>,
    identity: String,
}

impl GeminiProfile {
    /// Create an explicit profile with a nonempty semantic prompt-policy revision.
    pub fn new(
        endpoint: impl Into<String>,
        models: StageModels,
        revision: impl AsRef<str>,
        prompts: Arc<dyn PromptPolicy>,
    ) -> Result<Self> {
        let endpoint = endpoint.into();
        let parsed = reqwest::Url::parse(endpoint.as_str())?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            bail!(
                "Gemini endpoint must be an HTTP models URL without credentials, query, or fragment"
            );
        }
        if revision.as_ref().trim().is_empty() {
            bail!("Gemini prompt policy revision must not be empty");
        }
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let identity = identity(&endpoint, &models, revision.as_ref());
        Ok(Self {
            endpoint,
            models,
            prompts,
            identity,
        })
    }

    /// Create an explicit profile using built-in prompts and caller-selected models.
    pub fn from_models(endpoint: impl Into<String>, models: StageModels) -> Result<Self> {
        Self::new(endpoint, models, "embedded", Arc::new(EmbeddedPrompts))
    }

    /// Return the stable non-secret namespace for this complete generation policy.
    #[must_use]
    pub fn identity(&self) -> &str {
        self.identity.as_str()
    }

    pub(super) fn legacy(endpoint: String) -> Self {
        let models = StageModels::default();
        let identity = identity(&endpoint, &models, "embedded");
        Self {
            endpoint,
            models,
            prompts: Arc::new(EmbeddedPrompts),
            identity,
        }
    }

    pub(super) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    pub(super) fn model(&self, stage: GenerationStage) -> &str {
        self.models.resolve(stage)
    }

    pub(super) fn render(&self, stage: GenerationStage, prompt: &str) -> Result<String> {
        let rendered = self.prompts.render(stage, prompt)?;
        if rendered.trim().is_empty() {
            bail!("Gemini prompt policy returned an empty prompt for {stage:?}");
        }
        Ok(rendered)
    }
}

impl Default for GeminiProfile {
    fn default() -> Self {
        Self::legacy(String::from(DEFAULT_ENDPOINT))
    }
}

impl fmt::Debug for GeminiProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiProfile")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

fn identity(endpoint: &str, models: &StageModels, revision: &str) -> String {
    let mut digest = Sha256::new();
    for value in [
        PROFILE_VERSION,
        env!("CARGO_PKG_VERSION"),
        endpoint,
        revision,
        crate::generation::visual_revision(),
        include_str!("../../assets/gemini_intake_prompt.txt"),
        include_str!("../../assets/gemini_sense_prompt.txt"),
        include_str!("../../assets/gemini_card_meta_prompt.txt"),
        include_str!("../../assets/gemini_card_prompt.txt"),
        include_str!("../../assets/gemini_phonetics_prompt.txt"),
        include_str!("../../assets/learner_explanations_prompt.txt"),
        include_str!("../../assets/prompt_examples.json"),
        crate::generation::audio_prompt(),
    ] {
        digest.update(
            u64::try_from(value.len())
                .expect("invariant: prompt length must fit in u64")
                .to_le_bytes(),
        );
        digest.update(value.as_bytes());
    }
    let catalog = crate::languages::catalog();
    for code in catalog.codes() {
        let profile = catalog
            .borrowed(code)
            .expect("invariant: a declared language must have a profile");
        for value in [profile.code, profile.prompt.as_str()] {
            digest.update(
                u64::try_from(value.len())
                    .expect("invariant: language label length must fit in u64")
                    .to_le_bytes(),
            );
            digest.update(value.as_bytes());
        }
    }
    for stage in [
        GenerationStage::Intake,
        GenerationStage::Senses,
        GenerationStage::Metadata,
        GenerationStage::Correction,
        GenerationStage::Phonetics,
        GenerationStage::Features,
        GenerationStage::Scene,
        GenerationStage::Picture,
        GenerationStage::Recall,
        GenerationStage::Fidelity,
        GenerationStage::Zoom,
        GenerationStage::Text,
        GenerationStage::Speech,
    ] {
        let model = models.resolve(stage);
        digest.update(
            u64::try_from(model.len())
                .expect("invariant: model identifier length must fit in u64")
                .to_le_bytes(),
        );
        digest.update(model.as_bytes());
    }
    digest
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(&mut output, "{byte:02x}")
                .expect("invariant: writing hexadecimal bytes to a string cannot fail");
            output
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_identity_separates_endpoint_model_and_prompt_revision() {
        let baseline = GeminiProfile::default();
        let models = StageModels::default()
            .with_model(GenerationStage::Metadata, "alternate-text")
            .expect("alternate model must validate");
        let changed = [
            GeminiProfile::from_models("https://example.com/models", StageModels::default()),
            GeminiProfile::from_models(DEFAULT_ENDPOINT, models),
            GeminiProfile::new(
                DEFAULT_ENDPOINT,
                StageModels::default(),
                "new-prompt-v7",
                Arc::new(EmbeddedPrompts),
            ),
        ];
        assert!(
            changed.into_iter().all(|profile| {
                profile.expect("custom profile must validate").identity() != baseline.identity()
            }),
            "changed generation settings retained the same artifact namespace"
        );
    }

    #[test]
    fn model_override_order_cannot_change_cache_identity() {
        let forward = StageModels::default()
            .with_model(GenerationStage::Metadata, "alternate-text")
            .and_then(|models| models.with_model(GenerationStage::Picture, "alternate-image"))
            .expect("forward models must validate");
        let reverse = StageModels::default()
            .with_model(GenerationStage::Picture, "alternate-image")
            .and_then(|models| models.with_model(GenerationStage::Metadata, "alternate-text"))
            .expect("reverse models must validate");
        assert_eq!(
            GeminiProfile::from_models(DEFAULT_ENDPOINT, forward)
                .expect("forward profile must validate")
                .identity(),
            GeminiProfile::from_models(DEFAULT_ENDPOINT, reverse)
                .expect("reverse profile must validate")
                .identity(),
            "equivalent model selections produced different cache identities"
        );
    }

    #[test]
    fn custom_prompt_policy_cannot_omit_its_revision() {
        assert!(
            GeminiProfile::new(
                DEFAULT_ENDPOINT,
                StageModels::default(),
                " \n ",
                Arc::new(EmbeddedPrompts),
            )
            .is_err(),
            "an unversioned prompt policy escaped cache identity validation"
        );
    }

    #[test]
    fn model_identifiers_cannot_change_request_paths() {
        assert!(
            ["", "../model", "model?key=other", "model/other", " model"]
                .into_iter()
                .all(|model| {
                    StageModels::default()
                        .with_model(GenerationStage::Intake, model)
                        .is_err()
                }),
            "a malformed model identifier escaped URL validation"
        );
    }
}
