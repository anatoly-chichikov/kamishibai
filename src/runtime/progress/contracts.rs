use std::path::Path;

use crate::generation::{BuildProgress, SkippedCard};

/// Print one terminal line with an optional highlight hint.
pub trait Console {
    /// Print one terminal line.
    fn print(&mut self, text: &str, highlight: bool);
}

/// Start, stop, and update one status indicator.
pub trait Status {
    /// Update the visible status text.
    fn update(&mut self, text: &str);
    /// Start the status indicator.
    fn start(&mut self);
    /// Stop the status indicator.
    fn stop(&mut self);
}

/// Report output artifacts and summary lines after the batch.
pub trait AppProgress: BuildProgress {
    /// Report one final output artifact.
    fn result(&mut self, label: &str, path: &Path);
    /// Report one final batch summary.
    fn finish(&mut self, successful: usize, total: usize, failures: &[SkippedCard]);
}
