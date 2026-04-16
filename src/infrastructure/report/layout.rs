use crate::domain::entry::NormalizedEntry;
use crate::domain::profile::{Fonts, Labels, UiLabels};

use super::FontFamily;

/// Select one label set for one report entry.
pub trait LabelSource {
    /// Return the label set for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> UiLabels;
}

impl LabelSource for Labels {
    /// Return the label set for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> UiLabels {
        Labels::selected(self, entry)
    }
}

impl LabelSource for UiLabels {
    /// Return the label set for the entry.
    fn selected(&self, _entry: &NormalizedEntry) -> UiLabels {
        self.clone()
    }
}

/// Select one font family for one report entry.
pub trait FontSelector {
    /// Return the font family for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> FontFamily;
}

impl FontSelector for Fonts {
    /// Return the font family for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> FontFamily {
        FontFamily::new(Fonts::selected(self, entry).name())
    }
}

impl FontSelector for FontFamily {
    /// Return the font family for the entry.
    fn selected(&self, _entry: &NormalizedEntry) -> FontFamily {
        self.clone()
    }
}

/// Format one report entry into text rows.
pub trait ReportLayout {
    /// Return the text rows for one report entry.
    fn row(&self, entry: &NormalizedEntry) -> Vec<(String, f32)>;
}

/// Format one vocabulary entry into the frozen report row layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocabularyLayout<L> {
    labels: L,
}

impl Default for VocabularyLayout<Labels> {
    /// Return the default vocabulary layout.
    fn default() -> Self {
        Self {
            labels: Labels::default(),
        }
    }
}

impl<L> VocabularyLayout<L> {
    /// Create one vocabulary layout.
    pub fn new(labels: L) -> Self {
        Self { labels }
    }
}

impl<L> ReportLayout for VocabularyLayout<L>
where
    L: LabelSource,
{
    /// Return the text rows for one report entry.
    fn row(&self, entry: &NormalizedEntry) -> Vec<(String, f32)> {
        let labels = self.labels.selected(entry);
        let mut header = entry.word.clone();
        if !entry.pronunciation.is_empty() {
            header.push_str(format!(" /{}/", entry.pronunciation.trim_matches('/')).as_str());
        }
        header.push_str(format!(" — {}", entry.translation).as_str());
        let mut lines = vec![(header, 11.0)];
        if !entry.example.is_empty() {
            lines.push((entry.example.clone(), 9.0));
        }
        if !entry.sentence.is_empty() {
            lines.push((
                format!("{}: {}", labels.sentence.as_str(), entry.sentence),
                9.0,
            ));
        }
        if !entry.context.is_empty() {
            lines.push((
                format!("{}: {}", labels.context.as_str(), entry.context),
                8.0,
            ));
        }
        if !entry.hint.is_empty() {
            lines.push((format!("{}: {}", labels.hint.as_str(), entry.hint), 8.0));
        }
        if !entry.importance.is_empty() {
            lines.push((
                format!("{}: {}/10", labels.importance.as_str(), entry.importance),
                8.0,
            ));
        }
        lines
    }
}
