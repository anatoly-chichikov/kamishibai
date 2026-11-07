//! Scoped catchable signals and cooperative cancellation for local export I/O.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use signal_hook::SigId;
use signal_hook::consts::{SIGINT, SIGTERM};

use crate::anki::history::{HistoryError, SaveGate};

/// Share cancellation between signal handlers, the screen and its archive worker.
#[derive(Clone)]
pub(super) struct Cancellation {
    requested: Arc<AtomicBool>,
}

impl Cancellation {
    /// Start an export that has not been cancelled.
    pub(super) fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Ask outstanding I/O to refuse publication and remove its private part.
    pub(super) fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }
}

impl SaveGate for Cancellation {
    fn verify(&self) -> Result<(), HistoryError> {
        if self.requested.load(Ordering::Acquire) {
            return Err(HistoryError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "export cancelled",
            )));
        }
        Ok(())
    }
}

/// Keep cancellation hooks registered until workers and terminal restoration finish.
pub(super) struct Signals {
    registrations: Vec<SigId>,
}

impl Signals {
    fn new(registrations: Vec<SigId>) -> Self {
        Self { registrations }
    }

    /// Subscribe only this export to ordinary termination and terminal hangup.
    pub(super) fn from_cancellation(cancelled: &Cancellation) -> io::Result<Self> {
        let mut signals = Self::new(Vec::new());
        for signal in [SIGINT, SIGTERM] {
            signals.registrations.push(signal_hook::flag::register(
                signal,
                Arc::clone(&cancelled.requested),
            )?);
        }
        #[cfg(unix)]
        signals.registrations.push(signal_hook::flag::register(
            signal_hook::consts::SIGHUP,
            Arc::clone(&cancelled.requested),
        )?);
        Ok(signals)
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}
