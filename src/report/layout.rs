use crate::languages::{ReportLabels, UiLabels, language};
use crate::vocabulary::VocabularyEntry;

use super::FontFamily;

const DEFAULT_FONT: &str = "DejaVu Sans";

/// Select one label set for one report entry.
pub trait LabelSource {
    /// Return the label set for the entry.
    fn selected(&self, entry: &VocabularyEntry) -> UiLabels;
}

impl LabelSource for ReportLabels {
    /// Return the label set for the entry.
    fn selected(&self, entry: &VocabularyEntry) -> UiLabels {
        ReportLabels::selected(self, entry)
    }
}

impl LabelSource for UiLabels {
    /// Return the label set for the entry.
    fn selected(&self, _entry: &VocabularyEntry) -> UiLabels {
        self.clone()
    }
}

/// Select report fonts from the language profiles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportFonts {
    default: String,
}

impl Default for ReportFonts {
    /// Return the default font selector.
    fn default() -> Self {
        Self {
            default: String::from(DEFAULT_FONT),
        }
    }
}

impl ReportFonts {
    /// Return the selected font family for one entry.
    pub fn selected(&self, entry: &VocabularyEntry) -> FontFamily {
        let names = [entry.source.lang.as_str(), entry.target.lang.as_str()]
            .into_iter()
            .filter_map(|code| language(code).ok().map(|item| item.report_font))
            .collect::<Vec<_>>();
        if let Some(item) = names
            .iter()
            .find(|name| name.as_str() != self.default.as_str())
        {
            return FontFamily::new(item.clone());
        }
        if let Some(item) = names.first() {
            return FontFamily::new(item.clone());
        }
        FontFamily::new(self.default.clone())
    }
}

/// Select one font family for one report entry.
pub trait FontSelector {
    /// Return the font family for the entry.
    fn selected(&self, entry: &VocabularyEntry) -> FontFamily;
}

impl FontSelector for ReportFonts {
    /// Return the font family for the entry.
    fn selected(&self, entry: &VocabularyEntry) -> FontFamily {
        ReportFonts::selected(self, entry)
    }
}

impl FontSelector for FontFamily {
    /// Return the font family for the entry.
    fn selected(&self, _entry: &VocabularyEntry) -> FontFamily {
        self.clone()
    }
}

/// Format one report entry into text rows.
pub trait ReportLayout {
    /// Return the text rows for one report entry.
    fn row(&self, entry: &VocabularyEntry) -> Vec<(String, f32)>;
}

/// Format one vocabulary entry into the frozen report row layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocabularyLayout<L> {
    labels: L,
}

impl Default for VocabularyLayout<ReportLabels> {
    /// Return the default vocabulary layout.
    fn default() -> Self {
        Self {
            labels: ReportLabels::default(),
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
    fn row(&self, entry: &VocabularyEntry) -> Vec<(String, f32)> {
        let labels = self.labels.selected(entry);
        let mut header = String::from(entry.term.as_str());
        header.push_str(format!(" /{}/", entry.pronunciation.as_str().trim_matches('/')).as_str());
        header.push_str(format!(" — {}", entry.meaning.as_str()).as_str());
        let mut lines = vec![(header, 11.0)];
        lines.push((String::from(entry.target.sentence.as_str()), 9.0));
        lines.push((
            format!("{}: {}", labels.sentence.as_str(), entry.source.sentence),
            9.0,
        ));
        lines.push((
            format!("{}: {}", labels.context.as_str(), entry.source.context),
            8.0,
        ));
        lines.push((
            format!("{}: {}", labels.hint.as_str(), entry.source.hint),
            8.0,
        ));
        lines.push((
            format!("{}: {}/10", labels.importance.as_str(), entry.importance),
            8.0,
        ));
        lines
    }
}
