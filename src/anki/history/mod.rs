//! Read closed Anki collections and prepare versioned history files for the web importer.
//!
//! Call [`discover`] to list collection paths, then [`read`] with exact deck names
//! (an empty list selects Kamishibai decks). Inspect [`Snapshot::preview`] before
//! explicitly calling [`Snapshot::save`]. No provider or Anki write is involved.

mod archive;
mod manifest;
mod media;
mod source;
mod summary;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use manifest::Manifest;
use time::OffsetDateTime;

pub use archive::validate;
pub use source::{
    AnkiProcess, SystemAnkiProcess, application_version, closed, discover, inspect, read, roots,
};
pub use summary::{DeckSummary, Preview};

/// A typed import failure; delivery surfaces decide how to present recovery.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// Anki or its WAL still owns the collection; sync and quit before retrying.
    #[error("Anki is open or has crashed")]
    Open,
    /// Collection bytes changed during the read; repeat after quitting Anki.
    #[error("the Anki collection changed during the export")]
    Changed,
    /// The wire reader supports schema 18 only; update the application.
    #[error("kamishibai can't read Anki format {0} yet")]
    Schema(i64),
    /// A collection path cannot be read as selected.
    #[error("{0}")]
    Collection(String),
    /// A deck selection needs to be made explicitly.
    #[error("{0}")]
    Selection(String),
    /// Saving must leave the source profile and its Anki root untouched.
    #[error("cannot save inside the Anki folder")]
    Destination,
    /// The snapshot exceeds the web API limit; select fewer decks.
    #[error("too much to export at once")]
    Oversized,
    /// An archive violated the versioned file contract.
    #[error("the file failed its check: {0}")]
    Invalid(String),
    /// Filesystem or process inspection failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// SQLite could not read the collection.
    #[error("cannot read the Anki collection: {0}")]
    Sql(#[from] rusqlite::Error),
    /// JSON did not match the frozen snapshot shape.
    #[error("invalid file data: {0}")]
    Json(#[from] serde_json::Error),
    /// ZIP reading or writing failed.
    #[error("cannot read or write the file: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// A publication gate checked before creating output and immediately before publishing.
pub trait SaveGate {
    /// Refuse publication when the caller's source or cancellation condition changed.
    fn verify(&self) -> Result<(), HistoryError>;
}

struct Publication<'a> {
    source: &'a dyn SaveGate,
    caller: &'a dyn SaveGate,
}

impl SaveGate for Publication<'_> {
    fn verify(&self) -> Result<(), HistoryError> {
        self.source.verify()?;
        self.caller.verify()
    }
}

/// Selected raw rows and content-addressed media; source paths never serialize.
pub struct Snapshot {
    manifest: Manifest,
    source: ArchiveSource,
    decks: Vec<summary::DeckSummary>,
    missing: Vec<String>,
}

struct ArchiveSource {
    profile: PathBuf,
    files: BTreeMap<String, PathBuf>,
    gate: source::SourceGate,
}

impl ArchiveSource {
    fn new(profile: PathBuf, files: BTreeMap<String, PathBuf>, gate: source::SourceGate) -> Self {
        Self {
            profile,
            files,
            gate,
        }
    }
}

impl Snapshot {
    /// Check a proposed output folder without creating directories or staging files.
    pub fn check_destination(&self, directory: &Path) -> Result<(), HistoryError> {
        archive::destination(self, directory).map(|_| ())
    }
    fn new(
        manifest: Manifest,
        source: ArchiveSource,
        decks: Vec<summary::DeckSummary>,
        missing: Vec<String>,
    ) -> Self {
        Self {
            manifest,
            source,
            decks,
            missing,
        }
    }

    /// Calculate the preview from the selected snapshot, enforcing its size limit.
    pub fn preview(&self) -> Result<Preview, HistoryError> {
        let bytes = archive::encode(&self.manifest)?;
        Ok(Preview::new(
            &self.manifest,
            self.decks.clone(),
            self.missing.clone(),
            bytes.len(),
        ))
    }

    /// Write and verify a private, uncompressed archive without replacing any file.
    /// Pass UTC time and the writer version explicitly for reproducible exports.
    /// Refuse destinations inside the source profile or its Anki2 root before writing.
    /// Recheck that Anki is closed and the reviewed collection is unchanged at both gates.
    pub fn save(
        &self,
        directory: &Path,
        written: OffsetDateTime,
        writer: &str,
    ) -> Result<PathBuf, HistoryError> {
        archive::save(self, directory, written, writer, &self.source.gate)
    }

    /// Recheck the reviewed collection and a caller-owned gate at both publication checks.
    pub fn save_guarded(
        &self,
        directory: &Path,
        written: OffsetDateTime,
        writer: &str,
        gate: &dyn SaveGate,
    ) -> Result<PathBuf, HistoryError> {
        archive::save(
            self,
            directory,
            written,
            writer,
            &Publication {
                source: &self.source.gate,
                caller: gate,
            },
        )
    }
}
