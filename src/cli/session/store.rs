//! Persistent session state on disk: identity, phase, worker handle, result.
//!
//! Artifact readiness is deliberately NOT stored here — it is recomputed from
//! the shared cache (see `view`). This record carries only what the cache cannot:
//! the typed words, the curatable candidates, the committed plan (drafts), the
//! language pair, the output directory, the lifecycle phase, the published
//! result, and the background worker's pid. A monotonic `rev` gives saves
//! optimistic concurrency (compare-and-swap). Each session is a directory
//! `<cache>/sessions/<id>/` holding `session.json` (+ `worker.log`).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;
use time::format_description::well_known::Rfc3339;

use crate::generation::artifact_cache::Cache;
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CandidateRecord, WordCandidate};

use super::liveness;

const SESSION_FILE: &str = "session.json";
const WORKER_LOG: &str = "worker.log";
pub(super) const LOCK_FILE: &str = "lock";
const WRITE_LOCK_FILE: &str = "write.lock";
const VERSION: u32 = 2;

/// The lifecycle phase of one session, projected to JSON in lowercase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Phase {
    /// Created by `new`; words understood and curatable, generation not started.
    #[serde(alias = "draft")]
    Understood,
    /// A worker is generating (verify liveness before trusting this).
    #[serde(alias = "running")]
    Generating,
    /// A worker was recorded but its process is gone (crash/kill).
    Interrupted,
    /// Generation finished and the deck + report were written.
    Published,
    /// Generation ran out of retries or publishing failed.
    Failed,
    /// The worker was cancelled by the user.
    Cancelled,
}

/// The background worker's process handle, present only while one is recorded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct WorkerHandle {
    pub pid: i32,
    pub started: String,
}

/// One card draft's identity (the cache holds its artifacts and meta).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct DraftRecord {
    pub term: String,
    pub understanding: String,
}

/// The last artifact the worker reported working on (advisory heartbeat).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Progress {
    pub term: String,
    pub artifact: String,
}

/// The published artifacts of one session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ResultRecord {
    pub deck: String,
    pub report: String,
    pub output: String,
}

/// One persisted generation session.
///
/// `candidates` is the curatable understanding (which senses become cards);
/// `drafts` is the committed generation plan derived from the candidates when
/// generation starts. An empty `drafts` means no plan is committed yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SessionRecord {
    pub version: u32,
    /// Monotonic mutation counter for optimistic concurrency: every successful
    /// save bumps it, and a save is refused when the on-disk value has moved.
    #[serde(default)]
    pub rev: u64,
    pub id: String,
    pub created: String,
    pub from: String,
    pub to: String,
    pub out: String,
    pub senses: String,
    pub source: String,
    pub phase: Phase,
    #[serde(default)]
    pub words: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<CandidateRecord>,
    #[serde(default)]
    pub drafts: Vec<DraftRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<WorkerHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SessionRecord {
    /// Create one freshly understood session with no committed plan or worker.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn understood(
        id: String,
        created: String,
        from: String,
        to: String,
        out: String,
        senses: String,
        source: String,
        words: Vec<String>,
        candidates: Vec<CandidateRecord>,
    ) -> Self {
        Self {
            version: VERSION,
            rev: 0,
            id,
            created,
            from,
            to,
            out,
            senses,
            source,
            phase: Phase::Understood,
            words,
            candidates,
            drafts: Vec::new(),
            worker: None,
            progress: None,
            result: None,
            error: None,
        }
    }

    /// Backfill the candidate-driven shape onto a record read from an older file,
    /// synthesizing one single-sense candidate per legacy draft.
    fn backfilled(mut self) -> Self {
        if self.candidates.is_empty() && !self.drafts.is_empty() {
            self.candidates = self
                .drafts
                .iter()
                .map(|draft| {
                    CandidateRecord::from_candidate(&WordCandidate::new(
                        draft.term.as_str(),
                        draft.understanding.as_str(),
                        true,
                    ))
                })
                .collect();
        }
        if self.words.is_empty() {
            self.words = self
                .candidates
                .iter()
                .map(|candidate| candidate.term().to_string())
                .collect();
        }
        self.version = VERSION;
        self
    }
}

/// Persistent home for every session, rooted in the shared cache directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// Create one store rooted at an explicit cache directory (used by tests).
    pub(super) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create one store rooted at the live cache directory.
    pub(super) fn system() -> Result<Self> {
        Ok(Self::new(cache_root(&SystemContext)?))
    }

    /// Return the directory holding one session's files.
    pub(super) fn dir(&self, id: &str) -> PathBuf {
        self.root.join("sessions").join(id)
    }

    /// Return the path of one session's worker log.
    pub(super) fn log_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(WORKER_LOG)
    }

    /// Return the path of one session's advisory worker lock.
    pub(super) fn lock_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(LOCK_FILE)
    }

    /// Return the path of one session's save serialization lock.
    fn write_lock_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(WRITE_LOCK_FILE)
    }

    /// Return whether a session with this id already exists on disk.
    pub(super) fn exists(&self, id: &str) -> bool {
        self.dir(id).join(SESSION_FILE).is_file()
    }

    /// Read one session record, failing clearly when it is absent or corrupt.
    pub(super) fn open(&self, id: &str) -> Result<SessionRecord> {
        let path = self.dir(id).join(SESSION_FILE);
        let text = fs::read_to_string(&path).with_context(|| format!("no session '{id}'"))?;
        let record: SessionRecord = serde_json::from_str(text.as_str())
            .with_context(|| format!("session '{id}' is corrupt"))?;
        Ok(record.backfilled())
    }

    /// Return the on-disk revision of one session, or 0 when it is absent.
    pub(super) fn current_rev(&self, id: &str) -> u64 {
        let path = self.dir(id).join(SESSION_FILE);
        fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<SessionRecord>(text.as_str()).ok())
            .map(|record| record.rev)
            .unwrap_or(0)
    }

    /// Atomically write one session record under optimistic concurrency.
    ///
    /// A blocking write lock makes the read-revision/compare/write a single
    /// critical section across processes, so the compare-and-swap is real: the
    /// save is refused when the on-disk revision moved since `record` was read
    /// (a second `generate`, a `fix` racing the worker, …), and concurrent
    /// writers cannot both pass the check and have the last rename win. On
    /// success the record's `rev` is bumped to the value now on disk.
    pub(super) fn save(&self, record: &mut SessionRecord) -> Result<()> {
        fs::create_dir_all(self.dir(&record.id))?;
        let _write = liveness::lock_for_write(&self.write_lock_path(&record.id))?;
        let on_disk = self.current_rev(record.id.as_str());
        if on_disk != record.rev {
            bail!(
                "session '{}' changed concurrently (on-disk rev {on_disk}, expected {}); reload and retry",
                record.id,
                record.rev
            );
        }
        record.rev = on_disk + 1;
        let cache = Cache::new(format!("sessions/{}", record.id), self.root.clone());
        let staged = cache.stage(".json")?;
        let result = fs::write(&staged, serde_json::to_string_pretty(record)?)
            .map_err(anyhow::Error::from)
            .and_then(|()| cache.commit(&staged, SESSION_FILE));
        if result.is_err() {
            record.rev = on_disk;
            let _ = fs::remove_file(&staged);
        }
        result
    }

    /// Return every readable session, oldest first.
    pub(super) fn list(&self) -> Result<Vec<SessionRecord>> {
        let root = self.root.join("sessions");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&root)? {
            let path = entry?.path().join(SESSION_FILE);
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(record) = serde_json::from_str::<SessionRecord>(text.as_str())
            {
                out.push(record.backfilled());
            }
        }
        out.sort_by(|left, right| left.created.cmp(&right.created));
        Ok(out)
    }

    /// Delete one session's directory and everything in it.
    pub(super) fn remove(&self, id: &str) -> Result<()> {
        let dir = self.dir(id);
        if dir.is_dir() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// Return whether a user-supplied id is one safe filesystem segment.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Mint a default session id from the target language and the current time.
pub(super) fn mint_id(target: &str) -> Result<String> {
    Ok(format!("{target}-{}_{}", stamp()?, salt()))
}

/// Return the current UTC time formatted as RFC 3339.
pub(super) fn now() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn stamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc()
        .format(parse_time("[year][month][day]_[hour][minute][second]")?.as_slice())?)
}

fn salt() -> String {
    let mut rng = rand::rng();
    (0..4)
        .map(|_| char::from_digit(rng.random_range(0..36), 36).unwrap_or('0'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record(id: &str) -> SessionRecord {
        let mut record = SessionRecord::understood(
            String::from(id),
            String::from("2026-06-06T00:00:00Z"),
            String::from("en"),
            String::from("fr"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard")],
            vec![CandidateRecord::from_candidate(&WordCandidate::new(
                "canard", "a duck", true,
            ))],
        );
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
        }];
        record
    }

    #[test]
    fn a_saved_session_reads_back_identically() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut saved = record("fr-1");
        store.save(&mut saved).expect("session must save");
        assert_eq!(
            store.open("fr-1").expect("session must reopen"),
            saved,
            "a saved session no longer reads back identically"
        );
    }

    #[test]
    fn a_stale_save_is_refused_by_compare_and_swap() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut holder = record("fr-1");
        store
            .save(&mut holder)
            .expect("the first save must succeed");
        let mut stale = record("fr-1");
        stale.phase = Phase::Published;
        assert!(
            store.save(&mut stale).is_err(),
            "a save built on a superseded revision must be refused, not clobber the newer state"
        );
    }

    #[test]
    fn a_legacy_draft_session_backfills_one_candidate_per_draft() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let dir = store.dir("old-1");
        fs::create_dir_all(&dir).expect("session dir must be created");
        fs::write(
            dir.join("session.json"),
            r#"{"version":1,"id":"old-1","created":"t","from":"en","to":"fr","out":"/out","senses":"primary","source":"words","phase":"draft","drafts":[{"term":"canard","understanding":"a duck"}]}"#,
        )
        .expect("legacy session must be written");
        let record = store.open("old-1").expect("legacy session must reopen");
        assert_eq!(
            (
                record.phase,
                record.candidates.len(),
                record.candidates.first().map(CandidateRecord::term),
                record.words.clone(),
            ),
            (
                Phase::Understood,
                1,
                Some("canard"),
                vec![String::from("canard")],
            ),
            "a legacy draft session must read as understood with one backfilled candidate"
        );
    }

    #[test]
    fn the_latest_save_overwrites_the_previous_one() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut record = record("fr-1");
        store.save(&mut record).expect("first save");
        record.phase = Phase::Published;
        store.save(&mut record).expect("second save");
        assert_eq!(
            store.open("fr-1").expect("reopen").phase,
            Phase::Published,
            "the latest save no longer overwrites the previous session state"
        );
    }

    #[test]
    fn unsafe_ids_are_rejected() {
        assert!(
            !valid_id("../escape") && !valid_id("..") && !valid_id("a/b") && valid_id("fr-1_x.y"),
            "session id validation must reject path traversal and accept safe slugs"
        );
    }
}
