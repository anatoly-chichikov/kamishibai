use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow};

#[cfg(unix)]
use std::{fs::File, io::Write, os::fd::OwnedFd};

#[cfg(unix)]
pub(super) struct Redirect {
    sink: File,
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
}

#[cfg(unix)]
impl Redirect {
    /// Redirect stdout and stderr into one sink file.
    pub(super) fn new(sink: File) -> Result<Self> {
        let item = Self {
            sink,
            stdout: Some(saved_stdout()?),
            stderr: Some(saved_stderr()?),
        };
        if let Err(error) = item.mute() {
            let _ = item.restore();
            return Err(error);
        }
        Ok(item)
    }

    /// Redirect stdout and stderr into the sink file.
    fn mute(&self) -> Result<()> {
        flushed()?;
        muted(&self.sink)
    }

    /// Restore stdout and stderr after one redirect.
    pub(super) fn restore(mut self) -> Result<()> {
        flushed()?;
        restored_stdout(
            self.stdout
                .take()
                .ok_or_else(|| anyhow!("Saved stdout descriptor is missing"))?,
        )?;
        restored_stderr(
            self.stderr
                .take()
                .ok_or_else(|| anyhow!("Saved stderr descriptor is missing"))?,
        )
    }
}

/// Return the process-wide redirect gate.
fn gate() -> &'static Mutex<()> {
    static CELL: OnceLock<Mutex<()>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(()))
}

/// Run one closure while holding the process-wide redirect gate.
pub(super) fn locked<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _guard = gate()
        .lock()
        .map_err(|_| anyhow!("Redirect gate is poisoned"))?;
    action()
}

/// Run one closure while stdout and stderr are redirected to /dev/null.
pub(super) fn hush<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    locked(|| quiet(action))
}

/// Run one closure while stdout and stderr stay redirected to /dev/null.
#[cfg(unix)]
pub(super) fn quiet<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let sink = File::options().read(true).write(true).open("/dev/null")?;
    let item = Redirect::new(sink)?;
    let result = action();
    item.restore()?;
    result
}

/// Run one closure without native stream redirection on non-Unix systems.
#[cfg(not(unix))]
pub(super) fn quiet<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    action()
}

/// Drop one value while stdout and stderr stay redirected to /dev/null.
pub(super) fn discarded<T>(item: T) -> Result<()> {
    quiet(|| {
        drop(item);
        Ok(())
    })
}

/// Flush the noisy output stream before one descriptor swap.
#[cfg(unix)]
fn flushed() -> Result<()> {
    std::io::stdout().flush()?;
    std::io::stderr().flush()?;
    Ok(())
}

/// Return one duplicate of stdout.
#[cfg(unix)]
fn saved_stdout() -> Result<OwnedFd> {
    rustix::io::dup(std::io::stdout())
        .map_err(|error| anyhow!("Failed to duplicate stdout: {}", error))
}

/// Return one duplicate of stderr.
#[cfg(unix)]
fn saved_stderr() -> Result<OwnedFd> {
    rustix::io::dup(std::io::stderr())
        .map_err(|error| anyhow!("Failed to duplicate stderr: {}", error))
}

/// Redirect stdout and stderr into the sink file.
#[cfg(unix)]
fn muted(sink: &File) -> Result<()> {
    rustix::stdio::dup2_stdout(sink)
        .map_err(|error| anyhow!("Failed to redirect stdout: {}", error))?;
    rustix::stdio::dup2_stderr(sink)
        .map_err(|error| anyhow!("Failed to redirect stderr: {}", error))
}

/// Restore stdout from the saved descriptor.
#[cfg(unix)]
fn restored_stdout(saved: OwnedFd) -> Result<()> {
    rustix::stdio::dup2_stdout(&saved)
        .map_err(|error| anyhow!("Failed to restore stdout: {}", error))
}

/// Restore stderr from the saved descriptor.
#[cfg(unix)]
fn restored_stderr(saved: OwnedFd) -> Result<()> {
    rustix::stdio::dup2_stderr(&saved)
        .map_err(|error| anyhow!("Failed to restore stderr: {}", error))
}
