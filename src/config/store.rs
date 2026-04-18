use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::runtime::locations::{Context, data_home};

use super::Preferences;

const DIRECTORY: &str = "kamishibai";
const FILENAME: &str = "preferences.json";

/// Persistent filesystem-backed preference store.
#[derive(Clone, Debug)]
pub struct PreferenceStore {
    path: PathBuf,
}

impl PreferenceStore {
    /// Create one preference store pinned to an explicit file path.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Return the file path used for persistence.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Read current preferences, returning defaults when the file is absent.
    pub fn read(&self) -> Result<Preferences> {
        if !self.path.exists() {
            return Ok(Preferences::default());
        }
        let data = fs::read_to_string(self.path.as_path())?;
        Ok(serde_json::from_str::<Preferences>(data.as_str())?)
    }

    /// Atomically overwrite the stored preferences.
    pub fn write(&self, preferences: &Preferences) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(preferences)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(temporary.as_path(), data)?;
        fs::rename(temporary.as_path(), self.path.as_path())?;
        Ok(())
    }
}

/// Build the default platform-appropriate preference store.
pub fn default_store<C>(context: &C) -> Result<PreferenceStore>
where
    C: Context,
{
    let root = data_home(context)?;
    Ok(PreferenceStore::at(root.join(DIRECTORY).join(FILENAME)))
}
