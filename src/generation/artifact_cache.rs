//! Persistent filesystem cache helpers for media artifacts.

use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use tempfile::Builder;

/// Canonical filename of the card-meta JSON inside one card's cache folder.
pub const META_FILE: &str = "meta.json";
/// Canonical filename of the manga scene JSON inside one card's cache folder.
pub const SCENE_FILE: &str = "scene.json";
/// Canonical filename of the native-speaker WAV inside one card's cache folder.
pub const VOICE_FILE: &str = "audio.wav";
/// Canonical filename of the manga picture JPEG inside one card's cache folder.
pub const ILLUSTRATION_FILE: &str = "picture.jpg";
/// Canonical filename of the card-meta request cost sidecar.
pub const META_COST_FILE: &str = "meta.cost.json";
/// Canonical filename of the native-speaker request cost sidecar.
pub const VOICE_COST_FILE: &str = "audio.cost.json";
/// Canonical filename of the manga scene request cost sidecar.
pub const SCENE_COST_FILE: &str = "scene.cost.json";
/// Canonical filename of the manga picture request cost sidecar.
pub const ILLUSTRATION_COST_FILE: &str = "picture.cost.json";
/// Legacy visual revision marker removed when explicitly dropping artifacts.
pub const LEGACY_VISUAL_REVISION_FILE: &str = "visual.revision";
/// Directory that groups immutable visual-policy cache revisions.
pub const VISUAL_DIRECTORY: &str = "visual";
/// Advisory lock filename inside one visual-policy revision directory.
pub const VISUAL_LOCK_FILE: &str = "visual.lock";
/// Maximum time one artifact attempt waits for another visual producer.
pub const VISUAL_LOCK_TIMEOUT: Duration = Duration::from_secs(330);

const VISUAL_LOCK_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommitPlan {
    Failing {
        count: Rc<RefCell<usize>>,
        index: usize,
    },
    Normal,
}

/// Persistent cache rooted in one directory and named subdirectory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cache {
    root: PathBuf,
    path: PathBuf,
    plan: CommitPlan,
}

/// Held exclusive lease for one visual-policy revision cache.
#[derive(Debug)]
pub struct VisualGuard {
    _file: File,
}

impl Cache {
    /// Create one persistent cache directory handle.
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            path: root.join(name.into()),
            root,
            plan: CommitPlan::Normal,
        }
    }

    /// Create one cache handle that fails one selected commit call.
    pub fn failing(name: impl Into<String>, root: impl Into<PathBuf>, index: usize) -> Self {
        let root = root.into();
        Self {
            path: root.join(name.into()),
            root,
            plan: CommitPlan::Failing {
                count: Rc::new(RefCell::new(0)),
                index,
            },
        }
    }

    /// Return the root cache directory.
    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    /// Return the named cache directory.
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Resolve the content-addressed visual cache for one SHA-256 policy revision.
    pub fn visual(&self, revision: &str) -> Result<Self> {
        if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("visual revision must be exactly 64 hexadecimal characters");
        }
        Ok(Self {
            root: self.root.clone(),
            path: self.path.join(VISUAL_DIRECTORY).join(revision),
            plan: self.plan.clone(),
        })
    }

    /// Return whether one cached filename already exists.
    pub fn exists(&self, filename: &str) -> bool {
        self.path.join(filename).exists()
    }

    /// Return the absolute path for one cached filename.
    pub fn filepath(&self, filename: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.path)?;
        Ok(self.path.join(filename))
    }

    /// Return one staged temporary file path.
    pub fn stage(&self, suffix: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.path)?;
        let file = Builder::new().suffix(suffix).tempfile_in(&self.path)?;
        let (_handle, path) = file.keep()?;
        Ok(path)
    }

    /// Atomically replace the final filename with the staged file.
    pub fn commit(&self, staged: &Path, filename: &str) -> Result<()> {
        fs::create_dir_all(&self.path)?;
        if let CommitPlan::Failing { count, index } = &self.plan {
            let current = *count.borrow();
            *count.borrow_mut() += 1;
            if current == *index {
                return Err(anyhow!("commit failed"));
            }
        }
        fs::rename(staged, self.path.join(filename))?;
        Ok(())
    }

    /// Hold this visual revision's exclusive producer lock until the guard drops.
    pub fn hold_visual(&self, timeout: Duration) -> Result<VisualGuard> {
        fs::create_dir_all(&self.path)?;
        let path = self.path.join(VISUAL_LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let started = Instant::now();
        loop {
            if try_visual_lock(&file)? {
                return Ok(VisualGuard { _file: file });
            }
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                bail!(
                    "visual cache remained locked for {} ms at '{}'",
                    timeout.as_millis(),
                    path.display()
                );
            }
            sleep(VISUAL_LOCK_POLL.min(timeout.saturating_sub(elapsed)));
        }
    }
}

#[cfg(unix)]
fn try_visual_lock(file: &File) -> Result<bool> {
    use rustix::fs::{FlockOperation, flock};
    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error)
            if error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::AGAIN =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
fn try_visual_lock(file: &File) -> Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error.into()),
    }
}
