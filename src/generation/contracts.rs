use anyhow::Result;
use serde_json::Value;

/// Translate scenes with one prompt language bound to one client.
pub trait SceneSource {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, term: &str, sentence: &str, target: &str) -> Result<Value>;
}
