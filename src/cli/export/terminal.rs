//! Terminal ownership and asynchronous local I/O for the separate export screen.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::anki::history::{
    self, AnkiProcess, DeckSummary, HistoryError, SaveGate, Snapshot, SystemAnkiProcess,
};
use crate::tui::latin_key;

use super::screen::{Intent, Screen};
use super::{ExportArgs, profiles, refusal};

struct TerminalLease;

impl Drop for TerminalLease {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

struct Prepared {
    snapshot: Snapshot,
    guard: SourceGate,
}

struct SourceGate {
    collection: PathBuf,
    digest: Vec<u8>,
    cancelled: Arc<AtomicBool>,
}

impl SaveGate for SourceGate {
    fn verify(&self) -> Result<(), HistoryError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(HistoryError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "export cancelled",
            )));
        }
        history::closed(&self.collection, &SystemAnkiProcess)?;
        if fingerprint(&self.collection)? != self.digest {
            return Err(HistoryError::Changed);
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(HistoryError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "export cancelled",
            )));
        }
        Ok(())
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
pub(super) fn run(args: &ExportArgs, directory: PathBuf) -> Result<()> {
    enable_raw_mode()?;
    let _lease = TerminalLease;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut screen = Screen::from_directory(directory.clone());
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut prepared = None;
    let mut job = Some(start(
        Intent::Check,
        args.clone(),
        None,
        &directory,
        None,
        Arc::clone(&cancelled),
    ));
    let mut tick = 0;
    loop {
        terminal.draw(|frame| screen.draw(frame, tick))?;
        if let Some(receiver) = &job {
            match receiver.try_recv() {
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
                            cancelled.store(false, Ordering::Release);
                            screen.saved(path);
                        }
                        Err(_) if cancelled.load(Ordering::Acquire) => return Ok(()),
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
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(key) = event::read()?
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
                Intent::Cancel => cancelled.store(true, Ordering::Release),
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
                        Arc::clone(&cancelled),
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
    cancelled: Arc<AtomicBool>,
) -> Receiver<Result<Reply, Failure>> {
    let directory = directory.to_path_buf();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = perform(intent, &args, selection, &directory, prepared, cancelled);
        let _ = sender.send(result);
    });
    receiver
}

fn perform(
    intent: Intent,
    args: &ExportArgs,
    selection: Option<(PathBuf, Vec<String>)>,
    directory: &Path,
    prepared: Option<Box<Prepared>>,
    cancelled: Arc<AtomicBool>,
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
            let guard = SourceGate {
                digest: fingerprint(&path)?,
                collection: path.clone(),
                cancelled,
            };
            let snapshot = history::read(
                &path,
                &names,
                &SystemAnkiProcess,
                &history::application_version(),
            )?;
            guard.verify()?;
            Ok(Reply::Reviewed(Box::new(Prepared { snapshot, guard })))
        }
        Intent::Save => {
            let prepared = prepared.expect("invariant: saving follows a reviewed snapshot");
            let path = prepared.snapshot.save_guarded(
                directory,
                OffsetDateTime::now_utc(),
                env!("CARGO_PKG_VERSION"),
                &prepared.guard,
            )?;
            Ok(Reply::Saved(path))
        }
        _ => unreachable!("invariant: only export effects reach the worker"),
    }
}

fn fingerprint(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 65_536];
    loop {
        let length = file.read(&mut bytes)?;
        if length == 0 {
            return Ok(hash.finalize().to_vec());
        }
        hash.update(&bytes[..length]);
    }
}
