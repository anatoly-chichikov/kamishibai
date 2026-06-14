use crate::languages::{LanguageCatalog, LanguageProfile};
use anyhow::Result;

/// One explicit language direction for a batch: what is being learned, and what is explained in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguagePair {
    learning: String,
    known: String,
}

impl LanguagePair {
    /// Create one language pair from the learning and known codes.
    pub fn new(learning: impl Into<String>, known: impl Into<String>) -> Self {
        Self {
            learning: learning.into(),
            known: known.into(),
        }
    }

    /// Return the learning (studied) language code.
    pub fn learning(&self) -> &str {
        self.learning.as_str()
    }

    /// Return the known (native) language code.
    pub fn known(&self) -> &str {
        self.known.as_str()
    }

    /// Return the resolved learning profile from the supplied catalog.
    pub fn learning_profile(&self, catalog: &LanguageCatalog) -> Result<LanguageProfile> {
        catalog.item(self.learning.as_str())
    }

    /// Return the resolved known profile from the supplied catalog.
    pub fn known_profile(&self, catalog: &LanguageCatalog) -> Result<LanguageProfile> {
        catalog.item(self.known.as_str())
    }

    /// Return a compact "known → learning" label (native first, studied second).
    ///
    /// Order matches the on-screen chip — the user reads it as "from my
    /// language into the language being learned".
    pub fn label(&self) -> String {
        format!(
            "{} → {}",
            self.known.to_uppercase(),
            self.learning.to_uppercase()
        )
    }
}
