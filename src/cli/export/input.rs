//! Keep terminal reads separate from the owner that must drain cancelled archive writes.

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crossterm::event::{self, Event};

/// Read terminal events without preventing the owner from observing cancellation.
pub(super) trait Input {
    /// Return one event or allow the owner to check cancellation after the timeout.
    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// Isolate a terminal reader that may never return after its PTY disconnects.
/// Only archive workers are joined during shutdown; this reader owns no output files.
pub(super) struct TerminalInput {
    receiver: Receiver<io::Result<Event>>,
}

impl TerminalInput {
    fn new(receiver: Receiver<io::Result<Event>>) -> Self {
        Self { receiver }
    }

    /// Forward physical terminal events while the export owner remains responsive.
    #[must_use]
    pub(super) fn from_terminal() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            loop {
                let event = event::read();
                let failed = event.is_err();
                if sender.send(event).is_err() || failed {
                    break;
                }
            }
        });
        Self::new(receiver)
    }
}

impl Input for TerminalInput {
    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => event.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other(
                "export terminal reader ended without an event",
            )),
        }
    }
}
