//! Detect disconnected input independently of the reader while the owner drains writes.

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crossterm::event::{self, Event};

/// Read terminal events without preventing the owner from observing cancellation.
pub(super) trait Input {
    /// Return one event, an input failure or time for the owner to check cancellation.
    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// Forward decoded keys while the owner independently observes terminal hangup.
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
                let event = read();
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
        #[cfg(unix)]
        connected()?;
        let event = match self.receiver.recv_timeout(timeout) {
            Ok(event) => event.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other(
                "export terminal reader ended without an event",
            )),
        };
        #[cfg(unix)]
        connected()?;
        event
    }
}

/// Observe terminal hangup even when Crossterm remains inside its read loop.
#[cfg(unix)]
fn connected() -> io::Result<()> {
    wait(Duration::ZERO)
}

/// Wait on stdin without entering Crossterm's EOF-blind blocking reader.
#[cfg(unix)]
fn read() -> io::Result<Event> {
    loop {
        connected()?;
        if event::poll(Duration::ZERO)? {
            return event::read();
        }
        wait(Duration::from_millis(80))?;
    }
}

#[cfg(not(unix))]
fn read() -> io::Result<Event> {
    event::read()
}

/// Sleep until input, timeout or hangup, leaving all key decoding to Crossterm.
#[cfg(unix)]
fn wait(timeout: Duration) -> io::Result<()> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut descriptors = [PollFd::from_borrowed_fd(
        rustix::stdio::stdin(),
        PollFlags::IN,
    )];
    let timeout = Timespec::try_from(timeout).map_err(io::Error::other)?;
    if let Err(error) = poll(&mut descriptors, Some(&timeout))
        && error != rustix::io::Errno::INTR
    {
        return Err(error.into());
    }
    if descriptors[0]
        .revents()
        .intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL)
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "terminal input closed, export stopped",
        ));
    }
    Ok(())
}
