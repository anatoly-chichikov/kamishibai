use super::{LanguageEntry, UiLabels, language};

/// Select user-facing labels from the source language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportLabels {
    default: UiLabels,
}

impl Default for ReportLabels {
    /// Return the default label selector.
    fn default() -> Self {
        Self {
            default: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        }
    }
}

impl ReportLabels {
    /// Return the selected labels for one entry.
    pub fn selected<T>(&self, entry: &T) -> UiLabels
    where
        T: LanguageEntry,
    {
        let Some(code) = entry.source() else {
            return self.default.clone();
        };
        match language(code) {
            Ok(item) => item.labels,
            Err(_) => self.default.clone(),
        }
    }
}
