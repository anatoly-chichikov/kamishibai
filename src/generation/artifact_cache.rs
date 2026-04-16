//! Persistent filesystem cache helpers for media artifacts.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use tempfile::Builder;

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
}
