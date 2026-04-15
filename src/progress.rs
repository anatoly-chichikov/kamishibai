//! Plain and terminal progress output for the pipeline and CLI.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use regex::Regex;

use crate::media::{Failure, PipelineProgress};
use crate::scene::Progress as SceneProgress;

/// Return whether terminal progress should render on stdout.
pub fn uses_stdout() -> bool {
    cfg!(any(target_os = "macos", target_os = "ios"))
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    Stderr,
    Stdout,
}

const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
const INTERVAL: Duration = Duration::from_millis(200);

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
        if uses_stdout() {
            println!("{}", rendered(text));
            return;
        }
        eprintln!("{}", rendered(text));
    }
}

pub struct TerminalStatus {
    item: Option<LiveStatus>,
    stream: Stream,
    text: Arc<Mutex<String>>,
}

impl TerminalStatus {
    /// Create one terminal spinner status.
    pub fn new() -> Self {
        Self {
            item: None,
            stream: target(),
            text: Arc::new(Mutex::new(String::new())),
        }
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
        *self.text.lock().expect("status text must stay available") = String::from(text);
    }

    /// Start the status indicator.
    fn start(&mut self) {
        self.stop();
        self.item = Some(LiveStatus::new(self.stream, self.text.clone(), INTERVAL));
    }

    /// Stop the status indicator.
    fn stop(&mut self) {
        if let Some(item) = self.item.take() {
            item.stop();
        }
    }
}

/// Return the active terminal draw target for progress updates.
fn target() -> Stream {
    if uses_stdout() {
        return Stream::Stdout;
    }
    Stream::Stderr
}

#[derive(Debug)]
struct LiveStatus {
    join: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    stream: Stream,
}

impl LiveStatus {
    /// Create one live terminal spinner session.
    fn new(stream: Stream, text: Arc<Mutex<String>>, span: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || run(stream, text, flag, span));
        Self {
            join: Some(join),
            stop,
            stream,
        }
    }

    /// Stop one live terminal spinner session.
    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(item) = self.join.take() {
            let _ = item.join();
        }
        clear(self.stream);
    }
}

/// Render one live terminal spinner loop until the stop flag is set.
fn run(stream: Stream, text: Arc<Mutex<String>>, stop: Arc<AtomicBool>, span: Duration) {
    let mut item = 0usize;
    while !stop.load(Ordering::Relaxed) {
        let line = text
            .lock()
            .expect("status text must stay available")
            .clone();
        draw(stream, frame(item), line.as_str());
        item += 1;
        thread::sleep(span);
    }
}

/// Return one spinner frame for the requested tick number.
fn frame(index: usize) -> &'static str {
    FRAMES[index % FRAMES.len()]
}

/// Draw one spinner frame with its text on the target stream.
fn draw(stream: Stream, frame: &str, text: &str) {
    write(stream, line(frame, text).as_str());
}

/// Clear the active spinner line on the target stream.
fn clear(stream: Stream) {
    write(stream, "\r\x1b[2K");
}

/// Return one formatted live spinner line.
fn line(frame: &str, text: &str) -> String {
    format!("\r\x1b[2K  {frame} {text}")
}

/// Write one live terminal control sequence to the target stream.
fn write(stream: Stream, text: &str) {
    match stream {
        Stream::Stdout => {
            use std::io::Write;
            let mut item = io::stdout();
            let _ = item.write_all(text.as_bytes());
            let _ = item.flush();
        }
        Stream::Stderr => {
            use std::io::Write;
            let mut item = io::stderr();
            let _ = item.write_all(text.as_bytes());
            let _ = item.flush();
        }
    }
}

/// Return the basename for one filesystem path.
fn base(path: &Path) -> String {
    path.file_name()
        .and_then(|item| item.to_str())
        .map(String::from)
        .unwrap_or_else(|| path.display().to_string())
}

/// Return ANSI-rendered terminal output for one rich-markup line.
fn rendered(text: &str) -> String {
    let mut value = links(text);
    for (mark, code) in [
        ("[bold]", "\u{1b}[1m"),
        ("[/bold]", "\u{1b}[0m"),
        ("[green]", "\u{1b}[32m"),
        ("[/green]", "\u{1b}[0m"),
        ("[yellow]", "\u{1b}[33m"),
        ("[/yellow]", "\u{1b}[0m"),
        ("[red]", "\u{1b}[31m"),
        ("[/red]", "\u{1b}[0m"),
        ("[dim]", "\u{1b}[2m"),
        ("[/dim]", "\u{1b}[0m"),
    ] {
        value = value.replace(mark, code);
    }
    value
}

/// Return OSC8 hyperlinks for every rich link tag in one line.
fn links(text: &str) -> String {
    let regex = Regex::new(r"\[link=([^\]]+)\]([^\[]+)\[/link\]").expect("link regex must compile");
    regex
        .replace_all(text, "\u{1b}]8;;$1\u{1b}\\$2\u{1b}]8;;\u{1b}\\")
        .into_owned()
}

impl Output for io::Stdout {
    /// Print one output line.
    fn print(&mut self, text: &str) {
        println!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{INTERVAL, frame, line, rendered, uses_stdout};

    /// Terminal rendering converts rich progress markup into ANSI and OSC8 sequences.
    #[test]
    fn terminal_rendering_converts_rich_progress_markup_into_ansi_and_osc8_sequences() {
        assert_eq!(
            rendered("[bold]whims[/bold]  [green]✔[/green] ([link=file:///tmp/x.wav]x.wav[/link])"),
            "\u{1b}[1mwhims\u{1b}[0m  \u{1b}[32m✔\u{1b}[0m (\u{1b}]8;;file:///tmp/x.wav\u{1b}\\x.wav\u{1b}]8;;\u{1b}\\)",
            "terminal rendering no longer converts rich progress markup into ansi and osc8 sequences"
        );
    }

    /// Terminal spinner frames keep the circular half-step sequence.
    #[test]
    fn terminal_spinner_frames_keep_the_circular_half_step_sequence() {
        assert_eq!(
            (frame(0), frame(1), frame(2), frame(3), frame(4)),
            ("◐", "◓", "◑", "◒", "◐"),
            "terminal spinner frames no longer keep the circular half step sequence"
        );
    }

    /// Terminal spinner lines keep the same left indent as completed steps.
    #[test]
    fn terminal_spinner_lines_keep_the_same_left_indent_as_completed_steps() {
        assert_eq!(
            line("◐", "Rendering manga..."),
            String::from("\r\x1b[2K  ◐ Rendering manga..."),
            "terminal spinner lines no longer keep the same left indent as completed steps"
        );
    }

    /// Terminal spinner waits two hundred milliseconds between frames.
    #[test]
    fn terminal_spinner_waits_two_hundred_milliseconds_between_frames() {
        assert_eq!(
            INTERVAL,
            Duration::from_millis(200),
            "terminal spinner no longer waits two hundred milliseconds between frames"
        );
    }

    /// Terminal progress keeps stdout on Apple to avoid OCR stderr suppression.
    #[test]
    fn terminal_progress_keeps_stdout_on_apple_to_avoid_ocr_stderr_suppression() {
        assert_eq!(
            uses_stdout(),
            cfg!(any(target_os = "macos", target_os = "ios")),
            "terminal progress no longer keeps stdout on apple to avoid ocr stderr suppression"
        );
    }
}
