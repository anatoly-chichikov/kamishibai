//! Startup and validation error output for plain and terminal modes.

use std::io::{self, Write};
use std::path::Path;

/// Display one user-facing error.
pub trait Display {
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>);
}

/// Print one plain diagnosis line.
pub trait Output {
    /// Print one output line.
    fn print(&mut self, text: &str);
}

/// Print one terminal diagnosis renderable.
pub trait Console {
    /// Print one terminal renderable.
    fn print(&mut self, text: &str);
}

/// Print plain diagnosis output for non-interactive stderr.
#[derive(Clone, Debug)]
pub struct PlainDiagnosis<O> {
    output: O,
}

impl<O> PlainDiagnosis<O> {
    /// Create one plain diagnosis printer.
    pub fn new(output: O) -> Self {
        Self { output }
    }
}

impl<O> Display for PlainDiagnosis<O>
where
    O: Output,
{
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>) {
        self.output.print(format!("Error: {message}").as_str());
        if let Some(item) = path {
            self.output
                .print(format!("  File: {}", item.display()).as_str());
        }
    }
}

/// Print richer terminal diagnosis output.
#[derive(Clone, Debug)]
pub struct RichDiagnosis<C> {
    console: C,
}

impl<C> RichDiagnosis<C> {
    /// Create one rich diagnosis printer.
    pub fn new(console: C) -> Self {
        Self { console }
    }
}

impl<C> Display for RichDiagnosis<C>
where
    C: Console,
{
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>) {
        let mut body = String::from(message);
        if let Some(item) = path {
            body.push_str(
                format!(
                    "\n\n[dim]File:[/dim] [link=file://{0}]{0}[/link]",
                    item.display()
                )
                .as_str(),
            );
        }
        self.console
            .print(format!("[red][bold]Error[/bold][/red]\n{body}").as_str());
    }
}

/// Select the terminal-aware diagnosis implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosisSelector {
    terminal: bool,
}

impl DiagnosisSelector {
    /// Create one diagnosis selector.
    pub fn new(terminal: bool) -> Self {
        Self { terminal }
    }

    /// Return the selected diagnosis implementation.
    pub fn selected(self) -> SelectedDiagnosis {
        if self.terminal {
            return SelectedDiagnosis::Rich(RichDiagnosis::new(TerminalConsole));
        }
        SelectedDiagnosis::Plain(PlainDiagnosis::new(StderrOutput))
    }
}

/// Hold the selected diagnosis implementation.
pub enum SelectedDiagnosis {
    Plain(PlainDiagnosis<StderrOutput>),
    Rich(RichDiagnosis<TerminalConsole>),
}

impl Display for SelectedDiagnosis {
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>) {
        match self {
            Self::Plain(item) => item.show(message, path),
            Self::Rich(item) => item.show(message, path),
        }
    }
}

#[derive(Default)]
pub struct StderrOutput;

impl Output for StderrOutput {
    /// Print one output line.
    fn print(&mut self, text: &str) {
        let _ = writeln!(io::stderr(), "{text}");
    }
}

pub struct TerminalConsole;

impl Console for TerminalConsole {
    /// Print one terminal renderable.
    fn print(&mut self, text: &str) {
        eprintln!("{text}");
    }
}
