//! Plain and terminal progress output for the pipeline and CLI.

use std::io;
use std::path::Path;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget};

use crate::media::{Failure, PipelineProgress};
use crate::scene::Progress as SceneProgress;

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

/// Print richer terminal progress output with spinner events.
#[derive(Clone, Debug)]
pub struct RichProgress<C, S> {
    console: C,
    spinner: S,
}

impl<C, S> RichProgress<C, S> {
    /// Create one rich progress printer.
    pub fn new(console: C, spinner: S) -> Self {
        Self { console, spinner }
    }
}

impl<C, S> SceneProgress for RichProgress<C, S>
where
    C: Console,
    S: Status,
{
    /// Signal the start of one step.
    fn step(&mut self, name: &str) {
        self.spinner.update(format!("{name}...").as_str());
        self.spinner.start();
    }

    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>) {
        self.spinner.stop();
        let suffix = match path {
            Some(item) => format!(" ([link=file://{}]{}[/link])", item.display(), base(item)),
            None => String::new(),
        };
        self.console.print(
            format!("  [green]✔[/green] {name}: {label}{suffix}").as_str(),
            false,
        );
    }

    /// Signal one retry within rendering.
    fn retry(&mut self, _name: &str, attempt: usize, reason: &str) {
        self.spinner.stop();
        self.console.print(
            format!("  [yellow]↻[/yellow] {reason} (attempt {attempt})").as_str(),
            true,
        );
        self.spinner.start();
    }
}

impl<C, S> PipelineProgress for RichProgress<C, S>
where
    C: Console,
    S: Status,
{
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str) {
        self.console.print(
            format!("[bold]{word}[/bold] ({index}/{total})").as_str(),
            true,
        );
    }

    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str) {
        self.spinner.stop();
        self.console.print(
            format!("  [red]✘[/red] Skipping {word} - {reason}").as_str(),
            true,
        );
    }
}

impl<C, S> AppProgress for RichProgress<C, S>
where
    C: Console,
    S: Status,
{
    /// Report one final output artifact.
    fn result(&mut self, label: &str, path: &Path) {
        self.console.print(
            format!(
                "  [green]✔[/green] {label}: [link=file://{}]{}[/link]",
                path.display(),
                base(path)
            )
            .as_str(),
            false,
        );
    }

    /// Report one final batch summary.
    fn finish(&mut self, successful: usize, total: usize, failures: &[Failure]) {
        self.console.print(
            format!("\n[bold]Processed {successful}/{total} cards[/bold]").as_str(),
            true,
        );
        if failures.is_empty() {
            return;
        }
        self.console.print(
            format!("[yellow]Skipped {} card(s):[/yellow]", failures.len()).as_str(),
            true,
        );
        for item in failures {
            self.console
                .print(format!("  - {}: {}", item.word, item.reason).as_str(), true);
        }
    }
}

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

impl PipelineProgress for SelectedProgress {
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
    fn finish(&mut self, successful: usize, total: usize, failures: &[Failure]) {
        match self {
            Self::Plain(item) => item.finish(successful, total, failures),
            Self::Rich(item) => item.finish(successful, total, failures),
        }
    }
}

#[derive(Default)]
pub struct StdoutOutput;

impl Output for StdoutOutput {
    /// Print one output line.
    fn print(&mut self, text: &str) {
        println!("{text}");
    }
}

pub struct TerminalConsole;

impl Console for TerminalConsole {
    /// Print one terminal line.
    fn print(&mut self, text: &str, _highlight: bool) {
        println!("{text}");
    }
}

pub struct TerminalStatus {
    bar: ProgressBar,
}

impl TerminalStatus {
    /// Create one terminal spinner status.
    pub fn new() -> Self {
        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stdout());
        Self { bar }
    }
}

impl Default for TerminalStatus {
    /// Return the default terminal spinner status.
    fn default() -> Self {
        Self::new()
    }
}

impl Status for TerminalStatus {
    /// Update the visible status text.
    fn update(&mut self, text: &str) {
        self.bar.set_message(String::from(text));
    }

    /// Start the status indicator.
    fn start(&mut self) {
        self.bar.enable_steady_tick(Duration::from_millis(80));
    }

    /// Stop the status indicator.
    fn stop(&mut self) {
        self.bar.finish_and_clear();
    }
}

/// Return the basename for one filesystem path.
fn base(path: &Path) -> String {
    path.file_name()
        .and_then(|item| item.to_str())
        .map(String::from)
        .unwrap_or_else(|| path.display().to_string())
}

impl Output for io::Stdout {
    /// Print one output line.
    fn print(&mut self, text: &str) {
        println!("{text}");
    }
}
