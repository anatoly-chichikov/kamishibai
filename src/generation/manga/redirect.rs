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
        let flushed = flushed();
        let restored = self.restore_streams();
        flushed.and(restored)
    }

    fn restore_streams(&mut self) -> Result<()> {
        let stdout = self.stdout.take().map(restored_stdout).transpose();
        let stderr = self.stderr.take().map(restored_stderr).transpose();
        stdout.and(stderr).map(|_| ())
    }
}

#[cfg(unix)]
impl Drop for Redirect {
    fn drop(&mut self) {
        let _ = self.restore_streams();
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

#[cfg(all(test, unix))]
mod tests {
    use super::{Redirect, locked, quiet};

    #[test]
    fn a_panicking_native_call_cannot_leave_host_streams_muted() {
        let sink = tempfile::NamedTempFile::new().expect("sink must open");
        let panicked = locked(|| {
            let outer = Redirect::new(sink.reopen()?)?;
            let result =
                std::panic::catch_unwind(|| quiet::<(), _>(|| panic!("native call failed")));
            rustix::io::write(std::io::stdout(), b"restored stdout\n")?;
            rustix::io::write(std::io::stderr(), b"restored stderr\n")?;
            outer.restore()?;
            Ok(result.is_err())
        })
        .expect("streams must restore");
        let output = std::fs::read_to_string(sink.path()).expect("sink must read");
        assert!(
            panicked
                && output.contains("restored stdout\n")
                && output.contains("restored stderr\n"),
            "unwinding a native call permanently muted host output"
        );
    }

    #[test]
    fn embedded_native_operations_cannot_swallow_host_output() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let marker = "KAMISHIBAI_EMBEDDED_OUTPUT_TEST";
        if std::env::var_os(marker).is_some() {
            crate::generation::manga::NativeOutput::Preserve
                .run(|| {
                    rustix::io::write(std::io::stdout(), b"host-out-marker\n")?;
                    rustix::io::write(std::io::stderr(), b"host-err-marker\n")?;
                    Ok(())
                })
                .expect("embedded operation must run");
            return;
        }
        let mut child = Command::new(std::env::current_exe().expect("test executable must resolve"))
            .args(["generation::manga::redirect::tests::embedded_native_operations_cannot_swallow_host_output", "--exact"])
            .env(marker, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn().expect("isolated output test must start");
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().expect("child status must read").is_none() {
            if Instant::now() >= deadline {
                child.kill().expect("stalled output test must stop");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().expect("child output must collect");
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("host-out-marker")
                && String::from_utf8_lossy(&output.stderr).contains("host-err-marker"),
            "embedded OCR policy swallowed the hosting process output"
        );
    }
}
