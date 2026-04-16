use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use regex::Regex;

use super::contracts::{Console, Output, Status};
use super::uses_stdout;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    Stderr,
    Stdout,
}

const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
pub(super) const INTERVAL: Duration = Duration::from_millis(200);

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
pub(super) fn frame(index: usize) -> &'static str {
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
pub(super) fn line(frame: &str, text: &str) -> String {
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
pub(super) fn base(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|item| item.to_str())
        .map(String::from)
        .unwrap_or_else(|| path.display().to_string())
}

/// Return ANSI-rendered terminal output for one rich-markup line.
pub(super) fn rendered(text: &str) -> String {
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
