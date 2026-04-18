use crate::languages::{LanguageCatalog, LanguageProfile};
use anyhow::Result;

/// One explicit language direction for a batch: what is being learned, and what is explained in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguagePair {
    target: String,
    support: String,
}

impl LanguagePair {
    /// Create one language pair from target and support codes.
    pub fn new(target: impl Into<String>, support: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            support: support.into(),
        }
    }

    /// Return the target (learned) language code.
    pub fn target(&self) -> &str {
        self.target.as_str()
    }

    /// Return the support (my) language code.
    pub fn support(&self) -> &str {
        self.support.as_str()
    }

    /// Return the resolved target profile from the supplied catalog.
    pub fn target_profile(&self, catalog: &LanguageCatalog) -> Result<LanguageProfile> {
        catalog.item(self.target.as_str())
    }

    /// Return the resolved support profile from the supplied catalog.
    pub fn support_profile(&self, catalog: &LanguageCatalog) -> Result<LanguageProfile> {
        catalog.item(self.support.as_str())
    }

    /// Return a compact "target → support" label for header placement.
    pub fn label(&self) -> String {
        format!(
            "{} → {}",
            self.target.to_uppercase(),
            self.support.to_uppercase()
        )
    }
}
