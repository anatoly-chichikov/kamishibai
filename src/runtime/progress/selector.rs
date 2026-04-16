use std::path::Path;

use crate::generation::manga::Progress as SceneProgress;
use crate::generation::{BuildProgress, SkippedCard};

use super::contracts::AppProgress;
use super::{PlainProgress, RichProgress, StdoutOutput, TerminalConsole, TerminalStatus};

/// Select the terminal-aware progress implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressSelector {
    terminal: bool,
}

impl ProgressSelector {
    /// Create one progress selector.
    pub fn new(terminal: bool) -> Self {
        Self { terminal }
    }

    /// Return the selected progress implementation.
    pub fn selected(self) -> SelectedProgress {
        if self.terminal {
            return SelectedProgress::Rich(RichProgress::new(
                TerminalConsole,
                TerminalStatus::new(),
            ));
        }
        SelectedProgress::Plain(PlainProgress::new(StdoutOutput))
    }
}

/// Hold the selected progress implementation.
pub enum SelectedProgress {
    Plain(PlainProgress<StdoutOutput>),
    Rich(RichProgress<TerminalConsole, TerminalStatus>),
}

impl SceneProgress for SelectedProgress {
    /// Signal the start of one step.
    fn step(&mut self, name: &str) {
        match self {
            Self::Plain(item) => item.step(name),
            Self::Rich(item) => item.step(name),
        }
    }

    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>) {
        match self {
            Self::Plain(item) => item.done(name, label, path),
            Self::Rich(item) => item.done(name, label, path),
        }
    }

    /// Signal one retry within rendering.
    fn retry(&mut self, name: &str, attempt: usize, reason: &str) {
        match self {
            Self::Plain(item) => item.retry(name, attempt, reason),
            Self::Rich(item) => item.retry(name, attempt, reason),
        }
    }
}

impl BuildProgress for SelectedProgress {
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str) {
        match self {
            Self::Plain(item) => item.card(index, total, word),
            Self::Rich(item) => item.card(index, total, word),
        }
    }

    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str) {
        match self {
            Self::Plain(item) => item.skip(word, reason),
            Self::Rich(item) => item.skip(word, reason),
        }
    }
}

impl AppProgress for SelectedProgress {
    /// Report one final output artifact.
    fn result(&mut self, label: &str, path: &Path) {
        match self {
            Self::Plain(item) => item.result(label, path),
            Self::Rich(item) => item.result(label, path),
        }
    }

    /// Report one final batch summary.
    fn finish(&mut self, successful: usize, total: usize, failures: &[SkippedCard]) {
        match self {
            Self::Plain(item) => item.finish(successful, total, failures),
            Self::Rich(item) => item.finish(successful, total, failures),
        }
    }
}
