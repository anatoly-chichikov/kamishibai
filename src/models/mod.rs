//! Provider-neutral model boundary for the canonical card workflow.

mod look;
mod workflow;

use anyhow::Result;
use serde_json::Value;

/// Identifies one provider operation without exposing delivery-specific controls.
/// Experimental embedding API; compatibility is not guaranteed.
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

use crate::session::CostRecord;
pub use look::{Look, MangaInk, PictureStyle, Resend, SoftWorld};
pub use workflow::{Parts, cache_root, workflow};

/// Execute one model request without owning card prompts, validation, or retries.
///
/// Implementations own authentication, model selection, request authorization,
/// network deadlines, and provider wire formats. Authorize every outbound call,
/// including retries, and return observed usage even when its price is unknown.
/// The workflow calls this boundary for all generation and inspection stages.
/// Experimental embedding API; compatibility is not guaranteed.
pub trait ModelProvider: Send + Sync {
    /// Perform one blocking request and return its typed output and measured usage.
    fn generate(&self, request: ModelRequest) -> Result<ModelReply>;

    /// Perform a visual request, distinguishing a definite pre-send pause from provider failure.
    fn generate_visual(
        &self,
        request: ModelRequest,
    ) -> crate::generation::VisualResult<ModelReply> {
        self.generate(request).map_err(Into::into)
    }
}

/// A complete semantic request prepared by the canonical workflow.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub enum ModelRequest {
    /// Generate or inspect text, optionally using images as visual evidence.
    Text {
        /// Canonical operation used for provider-owned model selection.
        stage: GenerationStage,
        /// Fully rendered canonical prompt.
        prompt: String,
        /// Original images or enlarged crops required for this inspection.
        images: Vec<ModelImage>,
        /// Structured output and reasoning requirements.
        options: TextOptions,
    },
    /// Render a completed scene with the requested page proportions.
    Image {
        /// Fully compiled illustration prompt.
        prompt: String,
        /// Requested page ratio, currently `1:1`.
        aspect_ratio: String,
    },
    /// Produce signed 16-bit little-endian mono PCM at 24,000 samples per second.
    Speech {
        /// Canonical pronunciation directions, including its language context.
        prompt: String,
        /// Exact learner-facing text to pronounce, separate from instructions.
        text: String,
    },
}

/// Binary visual evidence with its declared MIME type.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub struct ModelImage {
    mime: String,
    bytes: Vec<u8>,
}

impl ModelImage {
    /// Bind original image bytes to their MIME type.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn new(mime: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            mime: mime.into(),
            bytes,
        }
    }

    /// Consume evidence into its MIME type and original bytes.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn into_parts(self) -> (String, Vec<u8>) {
        (self.mime, self.bytes)
    }
}

/// Requested output shape; canonical decoders still validate the returned text.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub enum TextFormat {
    /// Free text, which can contain a prompt-specified JSON document.
    Plain,
    /// One valid JSON document without provider-enforced schema.
    Json,
    /// One JSON document following the supplied JSON Schema.
    Schema(Value),
}

/// Relative reasoning budget requested for one canonical stage.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    /// Minimal deliberation for bounded extraction or rendering choices.
    Low,
    /// Intermediate deliberation for validation.
    Medium,
    /// Thorough deliberation for rich metadata composition.
    High,
}

/// Text output constraints independent of a provider's wire representation.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub struct TextOptions {
    format: TextFormat,
    reasoning: Option<ReasoningEffort>,
    ceiling: Option<u32>,
    deterministic: bool,
}

impl TextOptions {
    /// Bind output shape, optional reasoning and token ceiling, and determinism.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn new(
        format: TextFormat,
        reasoning: Option<ReasoningEffort>,
        ceiling: Option<u32>,
        deterministic: bool,
    ) -> Self {
        Self {
            format,
            reasoning,
            ceiling,
            deterministic,
        }
    }

    /// Consume settings into output shape, reasoning, token ceiling, and determinism.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn into_parts(self) -> (TextFormat, Option<ReasoningEffort>, Option<u32>, bool) {
        (
            self.format,
            self.reasoning,
            self.ceiling,
            self.deterministic,
        )
    }
}

/// Provider output before canonical schema, media, and semantic validation.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub enum ModelOutput {
    /// Provider moderation refused this completed, accounted request.
    Refused,
    /// Model text with an explicit token-ceiling termination signal.
    Text {
        /// Original model text, including any rejected payload for diagnostics.
        text: String,
        /// Whether generation stopped because it reached its output ceiling.
        truncated: bool,
    },
    /// Original encoded image bytes, decoded and inspected by the core.
    Image(Vec<u8>),
    /// Signed 16-bit little-endian mono PCM at 24,000 samples per second.
    Speech(Vec<u8>),
}

/// One completed model response with provider-owned usage and pricing.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Debug)]
pub struct ModelReply {
    output: ModelOutput,
    usage: CostRecord,
}

impl ModelReply {
    /// Bind returned output to its observed request usage.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn new(output: ModelOutput, usage: CostRecord) -> Self {
        Self { output, usage }
    }

    /// Consume this reply into output and its request usage record.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn into_parts(self) -> (ModelOutput, CostRecord) {
        (self.output, self.usage)
    }
}
