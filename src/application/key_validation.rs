//! Application port for validating a newly entered provider key.

use anyhow::Result;

/// Confirm that a key is accepted before preferences are changed.
pub(crate) trait KeyValidation: Clone + Send + 'static {
    /// Validate the supplied key without persisting it.
    fn check_key(&self, key: &str) -> Result<()>;
}
