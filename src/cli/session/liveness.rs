//! Worker liveness and termination for the managed generation worker.
//!
//! Liveness is decided by an advisory lock the worker holds for its whole run
//! ([`hold`]/[`is_held`], `flock` via `rustix::fs`): the OS releases the lock when
//! the process dies — even on a crash — so a stale or reused pid can never fake a
//! live worker. The pid is kept only as the kill target for `cancel`
//! ([`terminate`]). Unix uses `rustix` so the crate stays free of `unsafe`; other
//! platforms report "not held / not alive", steering callers toward `--wait`.

use std::fs::File;
use std::path::Path;

use anyhow::Result;

#[cfg(unix)]
use std::thread::sleep;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process, test_kill_process};

/// Take the session's advisory lock, returning the guard `File` while this
/// process should be treated as the live worker. `None` means another live
/// worker already holds it. The lock is released when the returned `File` drops
/// (or when the process exits, by the OS).
#[cfg(unix)]
pub(super) fn hold(path: &Path) -> Result<Option<File>> {
    use rustix::fs::{FlockOperation, flock};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(file)),
        Err(error) if would_block(error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Acquire the per-session write lock, blocking until it is free, so the
/// read-modify-write of a save is atomic across processes. This is a different
/// file from the long-held liveness lock, so a worker holding the latter can
/// still take this one for each of its own saves. Released when the guard drops.
#[cfg(unix)]
pub(super) fn lock_for_write(path: &Path) -> Result<File> {
    use rustix::fs::{FlockOperation, flock};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    flock(&file, FlockOperation::LockExclusive)?;
    Ok(file)
}

/// Return whether some live process is currently holding the session's lock.
#[cfg(unix)]
pub(super) fn is_held(path: &Path) -> bool {
    use rustix::fs::{FlockOperation, flock};
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    else {
        return false;
    };
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => false,
        Err(error) => would_block(error),
    }
}

#[cfg(unix)]
fn would_block(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::AGAIN
}

/// Return whether a process with this pid is currently alive.
#[cfg(unix)]
pub(super) fn is_alive(pid: i32) -> bool {
    Pid::from_raw(pid)
        .map(|target| test_kill_process(target).is_ok())
        .unwrap_or(false)
}

/// Ask one process to terminate, escalating to a hard kill if it lingers.
#[cfg(unix)]
pub(super) fn terminate(pid: i32) {
    let Some(target) = Pid::from_raw(pid) else {
        return;
    };
    let _ = kill_process(target, Signal::TERM);
    for _ in 0..20 {
        if !is_alive(pid) {
            return;
        }
        sleep(Duration::from_millis(50));
    }
    let _ = kill_process(target, Signal::KILL);
}

/// Advisory locking is unsupported on this platform; open the file without a real
/// lock so the worker can still proceed (no cross-process mutual exclusion).
#[cfg(not(unix))]
pub(super) fn hold(path: &Path) -> Result<Option<File>> {
    Ok(Some(File::create(path)?))
}

/// Advisory locking is unsupported on this platform; open the file without a real
/// lock (saves are not serialized across processes here).
#[cfg(not(unix))]
pub(super) fn lock_for_write(path: &Path) -> Result<File> {
    Ok(File::create(path)?)
}

/// Lock state is unknown on this platform; report not held.
#[cfg(not(unix))]
pub(super) fn is_held(_path: &Path) -> bool {
    false
}

/// Liveness is unknown on this platform; report not alive.
#[cfg(not(unix))]
pub(super) fn is_alive(_pid: i32) -> bool {
    false
}

/// Termination is unsupported on this platform.
#[cfg(not(unix))]
pub(super) fn terminate(_pid: i32) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn an_absent_lock_reads_free_and_a_fresh_hold_acquires_it() {
        // Cross-process contention (the real worker-vs-status path) is covered by
        // the offline session e2e; flock between two fds of the same process is
        // unreliable on BSD/macOS, and the product never probes its own held lock.
        let home = TempDir::new().expect("tempdir must be created");
        let path = home.path().join("lock");
        let absent_free = !is_held(&path);
        let acquired = hold(&path)
            .expect("holding a free lock must not error")
            .is_some();
        assert!(
            absent_free && acquired,
            "an absent lock must read free and a fresh hold must acquire it"
        );
    }

    #[test]
    fn the_running_process_reports_alive() {
        let pid = i32::try_from(std::process::id()).expect("pid fits in i32");
        assert!(
            is_alive(pid),
            "the currently running process must report as alive"
        );
    }

    #[test]
    fn a_reaped_child_reports_not_alive() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawning `true` must succeed");
        let pid = i32::try_from(child.id()).expect("child pid fits in i32");
        child.wait().expect("reaping the child must succeed");
        assert!(
            !is_alive(pid),
            "a reaped child process must not report as alive"
        );
    }
}
