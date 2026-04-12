//! Persistent filesystem cache helpers for media artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tempfile::Builder;

/// Read and write cached media files.
pub trait FileCache {
    /// Return the root cache directory.
    fn root(&self) -> PathBuf;
    /// Return the named cache directory.
    fn path(&self) -> PathBuf;
    /// Return whether one cached filename already exists.
    fn exists(&self, filename: &str) -> bool;
    /// Return the absolute path for one cached filename.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
    /// Return one staged temporary file path.
    fn stage(&self, suffix: &str) -> Result<PathBuf>;
    /// Atomically replace the final filename with the staged file.
    fn commit(&self, staged: &Path, filename: &str) -> Result<()>;
}

/// Persistent cache rooted in one directory and named subdirectory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cache {
    root: PathBuf,
    path: PathBuf,
}

impl Cache {
    /// Create one persistent cache directory handle.
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            path: root.join(name.into()),
            root,
        }
    }
}

impl FileCache for Cache {
    /// Return the root cache directory.
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    /// Return the named cache directory.
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Return whether one cached filename already exists.
    fn exists(&self, filename: &str) -> bool {
        self.path.join(filename).exists()
    }

    /// Return the absolute path for one cached filename.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.path)?;
        Ok(self.path.join(filename))
    }

    /// Return one staged temporary file path.
    fn stage(&self, suffix: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.path)?;
        let file = Builder::new().suffix(suffix).tempfile_in(&self.path)?;
        let (_handle, path) = file.keep()?;
        Ok(path)
    }

    /// Atomically replace the final filename with the staged file.
    fn commit(&self, staged: &Path, filename: &str) -> Result<()> {
        fs::create_dir_all(&self.path)?;
        fs::rename(staged, self.path.join(filename))?;
        Ok(())
    }
}
