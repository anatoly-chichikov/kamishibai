use std::path::Path;

use crate::application::media::{Failure, PipelineProgress};
use crate::infrastructure::scene::Progress as SceneProgress;

use super::contracts::{AppProgress, Output};
use super::terminal::base;

/// Print plain non-interactive progress output.
#[derive(Clone, Debug)]
pub struct PlainProgress<O> {
    output: O,
}

impl<O> PlainProgress<O> {
    /// Create one plain progress printer.
    pub fn new(output: O) -> Self {
        Self { output }
    }
}

impl<O> SceneProgress for PlainProgress<O>
where
    O: Output,
{
    /// Signal the start of one step.
    fn step(&mut self, _name: &str) {}

    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>) {
        let suffix = match path {
            Some(item) => format!(" ({})", base(item)),
            None => String::new(),
        };
        self.output
            .print(format!("  {name}: {label}{suffix}").as_str());
    }

    /// Signal one retry within rendering.
    fn retry(&mut self, _name: &str, attempt: usize, reason: &str) {
        self.output
            .print(format!("  {reason} (attempt {attempt}), retrying...").as_str());
    }
}

impl<O> PipelineProgress for PlainProgress<O>
where
    O: Output,
{
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str) {
        self.output
            .print(format!("Processing card {index}/{total}: {word}").as_str());
    }

    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str) {
        self.output
            .print(format!("  Skipping {word} - {reason}").as_str());
    }
}

impl<O> AppProgress for PlainProgress<O>
where
    O: Output,
{
    /// Report one final output artifact.
    fn result(&mut self, label: &str, path: &Path) {
        self.output
            .print(format!("  {label}: {} ({})", base(path), path.display()).as_str());
    }

    /// Report one final batch summary.
    fn finish(&mut self, successful: usize, total: usize, failures: &[Failure]) {
        self.output
            .print(format!("\nProcessed {successful}/{total} cards").as_str());
        if failures.is_empty() {
            return;
        }
        self.output
            .print(format!("Skipped {} card(s):", failures.len()).as_str());
        for item in failures {
            self.output
                .print(format!("  - {}: {}", item.word, item.reason).as_str());
        }
    }
}
