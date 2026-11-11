//! Identifies failures of the core's own state independently of provider errors.

/// A storage failure that cannot be settled as a cooperative provider pause.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct StateFailure {
    source: anyhow::Error,
}

impl StateFailure {
    /// Preserve a state operation's original error and its diagnostic chain.
    pub(crate) fn new(source: impl Into<anyhow::Error>) -> Self {
        Self {
            source: source.into(),
        }
    }
}
