//! Startup and validation error output for plain and terminal modes.

use std::io::{self, Write};
use std::path::Path;

use crate::runtime::progress::Console;

/// Display one user-facing error.
pub trait Display {
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>);
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
    O: Console,
{
    /// Show one error message and an optional file path.
    fn show(&mut self, message: &str, path: Option<&Path>) {
        self.output
            .print(format!("Error: {message}").as_str(), false);
        if let Some(item) = path {
            self.output
                .print(format!("  File: {}", item.display()).as_str(), false);
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
        let mut plain = vec![String::from(message)];
        let mut rich = vec![String::from(message)];
        if let Some(item) = path {
            plain.push(String::new());
            plain.push(format!("File: {}", item.display()));
            rich.push(String::new());
            rich.push(format!(
                "{} {}",
                dim("File:"),
                link(
                    format!("file://{}", item.display()).as_str(),
                    item.display().to_string().as_str()
                )
            ));
        }
        self.console.print(
            panel("Error", plain.as_slice(), rich.as_slice()).as_str(),
            true,
        );
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

impl Console for StderrOutput {
    /// Print one output line.
    fn print(&mut self, text: &str, _highlight: bool) {
        let _ = writeln!(io::stderr(), "{text}");
    }
}

pub struct TerminalConsole;

impl Console for TerminalConsole {
    /// Print one terminal renderable.
    fn print(&mut self, text: &str, _highlight: bool) {
        eprintln!("{text}");
    }
}

/// Return one dimmed ANSI span.
fn dim(text: &str) -> String {
    format!("\u{1b}[2m{text}\u{1b}[0m")
}

/// Return one OSC8 hyperlink span.
fn link(url: &str, label: &str) -> String {
    format!("\u{1b}]8;;{url}\u{1b}\\{label}\u{1b}]8;;\u{1b}\\")
}

/// Return one boxed diagnosis panel with visible-width padding.
fn panel(title: &str, plain: &[String], rich: &[String]) -> String {
    let width = plain
        .iter()
        .map(|item| item.chars().count())
        .max()
        .unwrap_or(0);
    let inner = width + 2;
    let gap = inner.saturating_sub(title.chars().count() + 2);
    let left = gap / 2;
    let right = gap - left;
    let mut lines = vec![format!(
        "╭{} {} {}╮",
        "─".repeat(left),
        title,
        "─".repeat(right)
    )];
    for (plain, rich) in plain.iter().zip(rich.iter()) {
        lines.push(format!(
            "│ {}{} │",
            rich,
            " ".repeat(width.saturating_sub(plain.chars().count()))
        ));
    }
    lines.push(format!("╰{}╯", "─".repeat(inner)));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{dim, link, panel};

    /// Rich diagnosis panel keeps the box drawing and clickable file link contract.
    #[test]
    fn rich_diagnosis_panel_keeps_the_box_drawing_and_clickable_file_link_contract() {
        assert_eq!(
            panel(
                "Error",
                &[
                    String::from("problem"),
                    String::new(),
                    String::from("File: /tmp/broken.json")
                ],
                &[
                    String::from("problem"),
                    String::new(),
                    format!(
                        "{} {}",
                        dim("File:"),
                        link("file:///tmp/broken.json", "/tmp/broken.json")
                    )
                ]
            ),
            format!(
                "╭──────── Error ─────────╮\n│ problem                │\n│                        │\n│ {} {} │\n╰────────────────────────╯",
                dim("File:"),
                link("file:///tmp/broken.json", "/tmp/broken.json")
            ),
            "rich diagnosis panel no longer keeps the box drawing and clickable file link contract"
        );
    }
}
