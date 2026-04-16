use std::path::Path;

use crate::generation::manga::Progress as SceneProgress;
use crate::generation::{BuildProgress, SkippedCard};

use super::contracts::{AppProgress, Console, Status};
use super::terminal::base;

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

impl<C, S> BuildProgress for RichProgress<C, S>
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
    fn finish(&mut self, successful: usize, total: usize, failures: &[SkippedCard]) {
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
