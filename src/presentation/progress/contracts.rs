use std::path::Path;

use crate::application::media::{Failure, PipelineProgress};

/// Print one plain output line.
pub trait Output {
    /// Print one output line.
    fn print(&mut self, text: &str);
}

/// Print one terminal line with an optional highlight hint.
pub trait Console {
    /// Print one terminal line.
    fn print(&mut self, text: &str, highlight: bool);
}

/// Start and stop one live terminal region.
pub trait Live {
    /// Start the live region.
    fn start(&mut self);
    /// Stop the live region.
    fn stop(&mut self);
}

/// Update one spinner label.
pub trait Spinner {
    /// Update the spinner text.
    fn update(&mut self, text: &str);
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
pub trait AppProgress: PipelineProgress {
    /// Report one final output artifact.
    fn result(&mut self, label: &str, path: &Path);
    /// Report one final batch summary.
    fn finish(&mut self, successful: usize, total: usize, failures: &[Failure]);
}

/// Align one spinner with a separate live region controller.
#[derive(Clone, Debug)]
pub struct AlignedStatus<L, S> {
    live: L,
    spinner: S,
}

impl<L, S> AlignedStatus<L, S> {
    /// Create one aligned status wrapper.
    pub fn new(live: L, spinner: S) -> Self {
        Self { live, spinner }
    }
}

impl<L, S> Status for AlignedStatus<L, S>
where
    L: Live,
    S: Spinner,
{
    /// Update the visible status text.
    fn update(&mut self, text: &str) {
        self.spinner.update(text);
    }

    /// Start the status indicator.
    fn start(&mut self) {
        self.live.start();
    }

    /// Stop the status indicator.
    fn stop(&mut self) {
        self.live.stop();
    }
}

impl<L, S> AlignedStatus<L, S>
where
    L: Live,
    S: Spinner,
{
    /// Update the visible status text.
    pub fn update(&mut self, text: &str) {
        Status::update(self, text);
    }

    /// Start the status indicator.
    pub fn start(&mut self) {
        Status::start(self);
    }

    /// Stop the status indicator.
    pub fn stop(&mut self) {
        Status::stop(self);
    }
}
