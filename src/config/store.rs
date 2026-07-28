use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result as AnyResult;
use serde::{Deserialize, Serialize};
use tempfile::Builder;
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::process::Command;

use crate::runtime::locations::{Context, data_home};

use super::Preferences;

const DIRECTORY: &str = "kamishibai";
const FILENAME: &str = "preferences.json";
const LOCK_SUFFIX: &str = "lock";
#[cfg(windows)]
const WINDOWS_ACL_VERSION: u8 = 1;
type StoreResult<T> = std::result::Result<T, PreferenceStoreError>;

#[cfg(windows)]
const WINDOWS_ACL_SCRIPT: &str = include_str!("secure_windows_acl.ps1");

#[derive(Deserialize, Serialize)]
struct StoredPreferences {
    #[serde(flatten)]
    preferences: Preferences,
    #[serde(default, skip_serializing_if = "is_zero")]
    windows_acl_version: u8,
}

impl StoredPreferences {
    fn current(preferences: &Preferences) -> Self {
        Self {
            preferences: preferences.clone(),
            windows_acl_version: current_windows_acl_version(),
        }
    }
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

#[cfg(windows)]
fn current_windows_acl_version() -> u8 {
    WINDOWS_ACL_VERSION
}

#[cfg(not(windows))]
fn current_windows_acl_version() -> u8 {
    0
}

/// One actionable failure to read, secure, lock, or replace user preferences.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PreferenceStoreError {
    message: String,
    hint: String,
}

impl PreferenceStoreError {
    /// Return the recovery action suitable for plain and JSON CLI errors.
    #[must_use]
    pub fn hint(&self) -> &str {
        self.hint.as_str()
    }
}

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

    /// Read current preferences, returning defaults only when the file is absent.
    pub fn read(&self) -> std::result::Result<Preferences, PreferenceStoreError> {
        self.read_current()
    }

    /// Atomically overwrite stored preferences while holding the write lock.
    pub fn write(
        &self,
        preferences: &Preferences,
    ) -> std::result::Result<(), PreferenceStoreError> {
        let data = self.serialize(preferences)?;
        let _lock = self.lock()?;
        self.replace(data.as_slice())
    }

    /// Apply one serialized read-modify-write without losing concurrent updates.
    pub fn update(
        &self,
        apply: impl FnOnce(Preferences) -> Preferences,
    ) -> std::result::Result<Preferences, PreferenceStoreError> {
        let _lock = self.lock()?;
        let preferences = apply(self.read_for_update()?);
        let data = self.serialize(&preferences)?;
        self.replace(data.as_slice())?;
        Ok(preferences)
    }

    fn read_current(&self) -> StoreResult<Preferences> {
        let stored = self.read_stored(cfg!(unix))?;
        #[cfg(windows)]
        if stored.windows_acl_version < WINDOWS_ACL_VERSION {
            return self.migrate_windows_acl();
        }
        Ok(stored.preferences)
    }

    fn read_for_update(&self) -> StoreResult<Preferences> {
        Ok(self.read_stored(cfg!(unix))?.preferences)
    }

    fn read_stored(&self, secure: bool) -> StoreResult<StoredPreferences> {
        match fs::metadata(self.path.as_path()) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoredPreferences::current(&Preferences::default()));
            }
            Err(error) => return Err(self.failure("inspect", error)),
        }
        if secure {
            self.secure_existing()?;
        }
        let data =
            fs::read_to_string(self.path.as_path()).map_err(|error| self.failure("read", error))?;
        serde_json::from_str::<StoredPreferences>(data.as_str())
            .map_err(|error| self.failure("parse", error))
    }

    #[cfg(windows)]
    fn migrate_windows_acl(&self) -> StoreResult<Preferences> {
        let _lock = self.lock()?;
        let stored = self.read_stored(false)?;
        if stored.windows_acl_version >= WINDOWS_ACL_VERSION {
            return Ok(stored.preferences);
        }
        let preferences = stored.preferences;
        let data = self.serialize(&preferences)?;
        self.replace(data.as_slice())?;
        Ok(preferences)
    }

    fn serialize(&self, preferences: &Preferences) -> StoreResult<Vec<u8>> {
        serde_json::to_vec_pretty(&StoredPreferences::current(preferences))
            .map_err(|error| self.failure("serialize", error))
    }

    fn lock(&self) -> StoreResult<File> {
        let directory = self.prepare_directory()?;
        let path = self.path.with_extension(LOCK_SUFFIX);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(path.as_path())
            .map_err(|error| self.failure("open the write lock for", error))?;
        secure_file(path.as_path())
            .map_err(|error| self.failure("secure the write lock for", error))?;
        file.lock().map_err(|error| self.failure("lock", error))?;
        secure_directory(directory)
            .map_err(|error| self.failure("secure the directory for", error))?;
        Ok(file)
    }

    fn replace(&self, data: &[u8]) -> StoreResult<()> {
        let directory = self.prepare_directory()?;
        self.secure_existing_if_present()?;
        let mut staged = Builder::new()
            .prefix(".preferences-")
            .tempfile_in(directory)
            .map_err(|error| self.failure("create a temporary file for", error))?;
        secure_file(staged.path())
            .map_err(|error| self.failure("secure a temporary file for", error))?;
        staged
            .as_file_mut()
            .write_all(data)
            .map_err(|error| self.failure("write", error))?;
        staged
            .as_file_mut()
            .write_all(b"\n")
            .map_err(|error| self.failure("write", error))?;
        staged
            .as_file()
            .sync_all()
            .map_err(|error| self.failure("sync", error))?;
        staged
            .persist(self.path.as_path())
            .map_err(|error| self.failure("replace", error.error))?;
        secure_file(self.path.as_path()).map_err(|error| self.failure("secure", error))?;
        sync_directory(directory).map_err(|error| self.failure("sync the directory for", error))?;
        Ok(())
    }

    fn secure_existing(&self) -> StoreResult<()> {
        let directory = self.parent();
        secure_directory(directory)
            .map_err(|error| self.failure("secure the directory for", error))?;
        secure_file(self.path.as_path()).map_err(|error| self.failure("secure", error))?;
        Ok(())
    }

    fn secure_existing_if_present(&self) -> StoreResult<()> {
        match fs::metadata(self.path.as_path()) {
            Ok(_) => self.secure_existing(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(self.failure("inspect", error)),
        }
    }

    fn prepare_directory(&self) -> StoreResult<&Path> {
        let directory = self.parent();
        fs::create_dir_all(directory)
            .map_err(|error| self.failure("create the directory for", error))?;
        secure_directory(directory)
            .map_err(|error| self.failure("secure the directory for", error))?;
        Ok(directory)
    }

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn failure(&self, action: &str, error: impl std::fmt::Display) -> PreferenceStoreError {
        PreferenceStoreError {
            message: format!(
                "could not {action} preferences at '{}': {error}",
                self.path.display()
            ),
            hint: format!(
                "Fix ownership or permissions, or move the damaged file aside and retry: {}",
                self.path.display()
            ),
        }
    }
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn secure_directory(path: &Path) -> std::io::Result<()> {
    secure_windows_acl(path, "directory")
}

#[cfg(unix)]
fn secure_file(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(windows)]
fn secure_file(path: &Path) -> std::io::Result<()> {
    secure_windows_acl(path, "file")
}

#[cfg(windows)]
fn secure_windows_acl(path: &Path, kind: &str) -> std::io::Result<()> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            WINDOWS_ACL_SCRIPT,
        ])
        .env("KAMISHIBAI_ACL_TARGET", path)
        .env("KAMISHIBAI_ACL_KIND", kind)
        .env("PSModulePath", "")
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(output.stderr.as_slice());
    let detail = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().trim_end_matches('.'))
        .unwrap_or("PowerShell did not report a cause");
    Err(std::io::Error::other(format!(
        "Windows ACL enforcement failed with {}: {detail}",
        output.status,
    )))
}

#[cfg(not(any(unix, windows)))]
fn secure_directory(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::unsupported(
        "private directory permissions are unavailable",
    ))
}

#[cfg(not(any(unix, windows)))]
fn secure_file(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::unsupported(
        "private file permissions are unavailable",
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    let directory = File::open(path)?;
    match directory.sync_all() {
        Ok(()) => Ok(()),
        Err(error) if sync_unsupported(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_unsupported(error: &std::io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        let errno = rustix::io::Errno::from_raw_os_error(code);
        errno == rustix::io::Errno::INVAL
            || errno == rustix::io::Errno::NOTSUP
            || errno == rustix::io::Errno::OPNOTSUPP
    })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Build the default platform-appropriate preference store.
pub fn default_store<C>(context: &C) -> AnyResult<PreferenceStore>
where
    C: Context,
{
    let root = data_home(context)?;
    Ok(PreferenceStore::at(root.join(DIRECTORY).join(FILENAME)))
}
