//! Terminal ownership and asynchronous local I/O for the separate export screen.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Result, anyhow};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use time::OffsetDateTime;

use crate::anki::history::{
    self, AnkiProcess, DeckSummary, HistoryError, SaveGate, Snapshot, SystemAnkiProcess,
};
use crate::tui::latin_key;

use super::input::{Input, TerminalInput};
use super::interrupt::Cancellation;
use super::screen::{Intent, Screen};
use super::{ExportArgs, profiles, refusal};

struct TerminalLease;

struct ScreenWriter<W> {
    output: W,
    cancelled: Cancellation,
}

impl<W> ScreenWriter<W> {
    fn new(output: W, cancelled: Cancellation) -> Self {
        Self { output, cancelled }
    }
}

impl<W: Write> Write for ScreenWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.output.write(bytes) {
            Err(_) if self.cancelled.verify().is_err() => Ok(bytes.len()),
            result => result,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.output.flush() {
            Err(_) if self.cancelled.verify().is_err() => Ok(()),
            result => result,
        }
    }
}

impl Drop for TerminalLease {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

struct Prepared {
    snapshot: Snapshot,
}

struct Job {
    receiver: Receiver<Result<Reply, Failure>>,
    worker: Option<JoinHandle<()>>,
    cancelled: Cancellation,
}

impl Job {
    fn new(
        receiver: Receiver<Result<Reply, Failure>>,
        worker: Option<JoinHandle<()>>,
        cancelled: Cancellation,
    ) -> Self {
        Self {
            receiver,
            worker,
            cancelled,
        }
    }

    fn poll(&mut self) -> Result<Result<Reply, Failure>, TryRecvError> {
        let result = self.receiver.try_recv()?;
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| TryRecvError::Disconnected)?;
        }
        Ok(result)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancelled.cancel();
            let _ = worker.join();
        }
    }
}

enum Reply {
    Checked(Vec<PathBuf>),
    Inspected(Vec<DeckSummary>),
    Reviewed(Box<Prepared>),
    Saved(PathBuf),
}

enum Failure {
    History(HistoryError),
    Delivery(anyhow::Error),
}

impl From<HistoryError> for Failure {
    fn from(error: HistoryError) -> Self {
        Self::History(error)
    }
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self::Delivery(error)
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::History(HistoryError::Io(error))
    }
}

/// Own the alternate terminal until success, cancellation or an unrecoverable I/O error.
pub(super) fn run(args: &ExportArgs, directory: PathBuf, cancelled: Cancellation) -> Result<()> {
    enable_raw_mode()?;
    let _lease = TerminalLease;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let output = ScreenWriter::new(io::stdout(), cancelled.clone());
    let mut terminal = Terminal::new(CrosstermBackend::new(output))?;
    let input = TerminalInput::from_terminal();
    let mut screen = Screen::from_directory(directory.clone());
    let mut prepared = None;
    let mut job = Some(start(
        Intent::Check,
        args.clone(),
        None,
        &directory,
        None,
        cancelled.clone(),
    ));
    let mut tick = 0;
    loop {
        if cancelled.verify().is_err() {
            return Ok(());
        }
        terminal.draw(|frame| screen.draw(frame, tick))?;
        if let Some(worker) = &mut job {
            match worker.poll() {
                Ok(result) => {
                    job = None;
                    match result {
                        Ok(Reply::Checked(profiles)) => screen.checked(profiles),
                        Ok(Reply::Inspected(decks)) => screen.inspected(decks),
                        Ok(Reply::Reviewed(snapshot)) => {
                            screen.reviewed(snapshot.snapshot.preview().map_err(refusal)?);
                            prepared = Some(snapshot);
                        }
                        Ok(Reply::Saved(path)) => {
                            screen.saved(path);
                        }
                        Err(_) if cancelled.verify().is_err() => return Ok(()),
                        Err(error) => {
                            let open = matches!(
                                error,
                                Failure::History(HistoryError::Open | HistoryError::Changed)
                            );
                            let error = match error {
                                Failure::History(error) => refusal(error),
                                Failure::Delivery(error) => error,
                            };
                            let hint = crate::cli::error::hint_of(&error)
                                .map(|hint| format!("\n\n{hint}"))
                                .unwrap_or_default();
                            screen.failed(format!("{error:#}{hint}"), open);
                        }
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    return Err(anyhow!("export worker ended without a result"));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(Event::Key(key)) = input.poll(Duration::from_millis(80))?
            && key.kind != KeyEventKind::Release
        {
            let key = if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char(c) if latin_key(c) == 'c')
            {
                KeyCode::Esc
            } else {
                key.code
            };
            let intent = screen.key(key);
            match intent {
                Intent::Exit => return Ok(()),
                Intent::Cancel => cancelled.cancel(),
                Intent::Wait => {}
                _ if job.is_none() => {
                    let selection = if intent == Intent::Check {
                        None
                    } else {
                        Some(screen.select())
                    };
                    job = Some(start(
                        intent,
                        args.clone(),
                        selection,
                        &directory,
                        prepared.take(),
                        cancelled.clone(),
                    ));
                }
                _ => {}
            }
        }
        tick = tick.wrapping_add(1);
    }
}

fn start(
    intent: Intent,
    args: ExportArgs,
    selection: Option<(PathBuf, Vec<String>)>,
    directory: &Path,
    prepared: Option<Box<Prepared>>,
    cancelled: Cancellation,
) -> Job {
    let directory = directory.to_path_buf();
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancellation = cancelled.clone();
    let worker = std::thread::spawn(move || {
        let result = perform(intent, &args, selection, &directory, prepared, cancellation);
        let _ = sender.send(result);
    });
    Job::new(receiver, Some(worker), cancelled)
}

fn perform(
    intent: Intent,
    args: &ExportArgs,
    selection: Option<(PathBuf, Vec<String>)>,
    directory: &Path,
    prepared: Option<Box<Prepared>>,
    cancelled: Cancellation,
) -> Result<Reply, Failure> {
    if intent == Intent::Check {
        let paths = profiles(args)?;
        if paths.len() == 1 {
            history::closed(&paths[0], &SystemAnkiProcess)?;
        } else if SystemAnkiProcess.running(&paths[0])? {
            return Err(HistoryError::Open.into());
        }
        return Ok(Reply::Checked(paths));
    }
    let (path, names) = selection.expect("invariant: read/save steps follow profile selection");
    match intent {
        Intent::Inspect => Ok(Reply::Inspected(history::inspect(
            &path,
            &args.decks,
            &SystemAnkiProcess,
            &history::application_version(),
        )?)),
        Intent::Review => {
            let path = path.canonicalize()?;
            history::closed(&path, &SystemAnkiProcess)?;
            let snapshot = history::read(
                &path,
                &names,
                &SystemAnkiProcess,
                &history::application_version(),
            )?;
            cancelled.verify()?;
            Ok(Reply::Reviewed(Box::new(Prepared { snapshot })))
        }
        Intent::Save => {
            let prepared = prepared.expect("invariant: saving follows a reviewed snapshot");
            let path = prepared.snapshot.save_guarded(
                directory,
                OffsetDateTime::now_utc(),
                env!("CARGO_PKG_VERSION"),
                &cancelled,
            )?;
            Ok(Reply::Saved(path))
        }
        _ => unreachable!("invariant: only export effects reach the worker"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ClosedDisplay;

    impl Write for ClosedDisplay {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "display closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "display closed"))
        }
    }

    #[test]
    fn an_active_display_cannot_hide_a_write_failure() {
        let result = ScreenWriter::new(ClosedDisplay, Cancellation::new()).write(b"frame 947");
        assert!(
            matches!(result, Err(error) if error.kind() == io::ErrorKind::BrokenPipe),
            "an active display hid its write failure"
        );
    }

    #[test]
    fn an_active_display_cannot_hide_a_flush_failure() {
        let result = ScreenWriter::new(ClosedDisplay, Cancellation::new()).flush();
        assert!(
            matches!(result, Err(error) if error.kind() == io::ErrorKind::BrokenPipe),
            "an active display hid its flush failure"
        );
    }

    #[test]
    fn a_cancelled_display_cannot_fail_during_cursor_restoration() {
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let result = ScreenWriter::new(ClosedDisplay, cancelled).write_all(b"cursor 593");
        assert!(
            result.is_ok(),
            "a cancelled display propagated its cursor restoration failure"
        );
    }

    #[test]
    fn a_cancelled_display_cannot_fail_during_the_final_flush() {
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let result = ScreenWriter::new(ClosedDisplay, cancelled).flush();
        assert!(
            result.is_ok(),
            "a cancelled display propagated its final flush failure"
        );
    }
}
