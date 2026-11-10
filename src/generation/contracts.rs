use crate::models::GenerationStage;
use serde_json::Value;

/// Scene world chosen only after a provider moderation refusal.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneMode {
    /// Preserve the original request bytes.
    Stock,
    /// Append the plugged gentle world to both scene prompts.
    Soft,
}

/// Keep moderation and pre-send pauses typed until the visual transaction records its outcome.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Debug, thiserror::Error)]
pub enum VisualFailure {
    /// Delivery refused this request before sending it, preserving retry budgets.
    #[error("visual request paused before sending")]
    NotSent,
    /// A completed provider request refused the indicated visual stage.
    #[error("moderation refused at {stage:?}")]
    Refused {
        /// Canonical stage that produced the refusal.
        stage: GenerationStage,
    },
    /// A transport, validation, accounting, or persistence failure.
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

impl VisualFailure {
    /// Restore the original operational error at legacy anyhow boundaries.
    #[must_use]
    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Failed(error) => error,
            failure @ (Self::Refused { .. } | Self::NotSent) => anyhow::Error::new(failure),
        }
    }

    /// Attach operational context without erasing moderation or a pre-send pause.
    #[must_use]
    pub(crate) fn context(self, context: &str) -> Self {
        match self {
            Self::Refused { .. } | Self::NotSent => self,
            Self::Failed(error) => Self::Failed(error.context(context.to_owned())),
        }
    }
}

impl From<std::io::Error> for VisualFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.into())
    }
}

impl From<serde_json::Error> for VisualFailure {
    fn from(error: serde_json::Error) -> Self {
        Self::Failed(error.into())
    }
}

impl From<image::ImageError> for VisualFailure {
    fn from(error: image::ImageError) -> Self {
        Self::Failed(error.into())
    }
}

/// Result of a visual operation before moderation is committed to its cache.
/// Experimental embedding API; compatibility is not guaranteed.
pub type VisualResult<T> = std::result::Result<T, VisualFailure>;

/// Translate scenes with one prompt language bound to one client.
/// Experimental embedding API; compatibility is not guaranteed.
pub trait SceneSource {
    /// Return one translated scene JSON document.
    fn scene(
        &self,
        language: &str,
        term: &str,
        sentence: &str,
        target: &str,
        attempt: u8,
        mode: SceneMode,
    ) -> VisualResult<Value>;
}
