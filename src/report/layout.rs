use crate::languages::{ReportLabels, UiLabels};
use crate::vocabulary::VocabularyEntry;

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
