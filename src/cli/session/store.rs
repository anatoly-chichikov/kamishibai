//! Persistent session state on disk: identity, phase, worker handle, result.
//!
//! Artifact readiness is deliberately NOT stored here — it is recomputed from
//! the shared cache (see `view`). This record carries only what the cache cannot:
//! the typed words, the curatable candidates, the committed plan (drafts), the
//! language pair, session-scoped provider spend, the output directory, the
//! lifecycle phase, the published result, and the background worker's pid. Each
//! session is a directory `<cache>/sessions/<id>/` holding `session.json` (+
//! `worker.log`).
//!
//! Concurrency is two locks. The long-held liveness flock (`lock`, see
//! `liveness`) decides who may generate: the OS releases it when the worker dies,
//! so a stale pid can never fake a live worker — `interrupted` is derived from
//! a recorded worker whose lock is free, while `cancelled` is stored by `cancel`.
//! The short write flock (`write.lock`) makes every change to `session.json` a
//! serialized read-modify-write ([`SessionStore::update`]): concurrent commands
//! all apply, in some order, instead of clobbering or spuriously failing. The
//! worker additionally writes only while the record still names it (see
//! `worker`), which is how cancel and a finishing worker resolve their race.
//! On non-Unix platforms advisory flocks are unavailable and both locks degrade
//! to best-effort (see `liveness`): single-process use stays correct, but
//! concurrent processes are not serialized there.

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
use crate::session::{
    ArtifactCosts, CandidateRecord, CardRewrite, SentenceBatchSettings, SentenceLabelSelection,
};

use super::cost_journal::SessionCostJournal;
use super::liveness;

const SESSION_FILE: &str = "session.json";
const WORKER_LOG: &str = "worker.log";
pub(super) const LOCK_FILE: &str = "lock";
const WRITE_LOCK_FILE: &str = "write.lock";
const VERSION: u32 = 2;

/// The lifecycle phase of one session, projected to JSON in lowercase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::cli) enum Phase {
    /// Created by `new`; words understood and curatable, generation not started.
    Understood,
    /// A worker is generating (verify liveness before trusting this).
    Generating,
    /// A worker was recorded but its process is gone (crash/kill).
    Interrupted,
    /// Generation finished and the deck + report were written.
    Published,
    /// The deck was published but some cards failed; the rest are in it.
    Partial,
    /// Generation ran out of retries or publishing failed.
    Failed,
    /// The worker was cancelled by the user.
    Cancelled,
}

/// The background worker's process handle, present only while one is recorded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::cli) struct WorkerHandle {
    pub pid: i32,
    pub started: String,
}

/// One card draft's identity and this session's provider spend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::cli) struct DraftRecord {
    pub term: String,
    pub understanding: String,
    #[serde(default, skip_serializing_if = "ArtifactCosts::is_empty")]
    pub costs: ArtifactCosts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite: Option<CardRewrite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_request: Option<SentenceLabelSelection>,
}

/// The last artifact the worker reported working on (advisory heartbeat).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::cli) struct Progress {
    pub term: String,
    pub artifact: String,
}

/// The published artifacts of one session, with how many cards made the deck and
/// how many failed (`failed > 0` marks a `Partial` publish).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::cli) struct ResultRecord {
    pub deck: String,
    pub report: String,
    pub output: String,
    #[serde(default)]
    pub cards: usize,
    #[serde(default)]
    pub failed: usize,
}

/// One persisted generation session.
///
/// `candidates` is the curatable understanding (which senses become cards);
/// `drafts` is the committed generation plan derived from the candidates when
/// generation starts. An empty `drafts` means no plan is committed yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::cli) struct SessionRecord {
    pub version: u32,
    pub id: String,
    pub created: String,
    pub known: String,
    pub learning: String,
    pub out: String,
    pub senses: String,
    #[serde(default)]
    pub sentences: SentenceBatchSettings,
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
    pub(in crate::cli) fn understood(
        id: String,
        created: String,
        known: String,
        learning: String,
        out: String,
        senses: String,
        source: String,
        words: Vec<String>,
        candidates: Vec<CandidateRecord>,
    ) -> Self {
        Self {
            version: VERSION,
            id,
            created,
            known,
            learning,
            out,
            senses,
            sentences: SentenceBatchSettings::default(),
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

    /// Return the record carrying this batch's sentence-generation settings.
    #[must_use]
    pub(in crate::cli) fn with_sentences(mut self, sentences: SentenceBatchSettings) -> Self {
        self.sentences = sentences;
        self
    }
}

/// Persistent home for every session, rooted in the shared cache directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::cli) struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// Create one store rooted at an explicit cache directory (used by tests).
    pub(in crate::cli) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create one store rooted at the live cache directory.
    pub(in crate::cli) fn system() -> Result<Self> {
        Ok(Self::new(cache_root(&SystemContext)?))
    }

    /// Address the cost journal belonging only to this record's immutable run identity.
    pub(in crate::cli) fn cost_journal(&self, record: &SessionRecord) -> SessionCostJournal {
        self.cost_journal_for(record.id.as_str(), record.created.as_str())
    }

    /// Address one cost journal before a fresh TUI has written its first record.
    pub(in crate::cli) fn cost_journal_for(&self, id: &str, created: &str) -> SessionCostJournal {
        SessionCostJournal::new(self.root.as_path(), id, created)
    }

    /// Return the directory holding one session's files.
    fn dir(&self, id: &str) -> PathBuf {
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

    /// Take one session's long-held liveness lock, creating its directory on the
    /// way; `None` means a live worker already holds it. The OS releases the lock
    /// when the holder dies, so callers keep the returned guard for the whole run.
    pub(in crate::cli) fn hold(&self, id: &str) -> Result<Option<fs::File>> {
        fs::create_dir_all(self.dir(id))?;
        liveness::hold(&self.lock_path(id))
    }

    /// Read one session record, failing clearly when it is absent, corrupt, or
    /// written by an incompatible build.
    pub(super) fn open(&self, id: &str) -> Result<SessionRecord> {
        let path = self.dir(id).join(SESSION_FILE);
        let text = fs::read_to_string(&path).with_context(|| format!("no session '{id}'"))?;
        let record: SessionRecord = serde_json::from_str(text.as_str())
            .with_context(|| format!("session '{id}' is corrupt"))?;
        if record.version != VERSION {
            bail!(
                "session '{id}' was written by an older build (version {}); remove it with kamishibai rm",
                record.version
            );
        }
        Ok(record)
    }

    /// Atomically replace one session's file (stage + rename). Callers hold the
    /// write lock; this never locks by itself.
    fn write(&self, record: &SessionRecord) -> Result<()> {
        let cache = Cache::new(format!("sessions/{}", record.id), self.root.clone());
        let staged = cache.stage(".json")?;
        let result = fs::write(&staged, serde_json::to_string_pretty(record)?)
            .map_err(anyhow::Error::from)
            .and_then(|()| cache.commit(&staged, SESSION_FILE));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result
    }

    /// Persist one freshly created session, refusing to overwrite an existing one.
    pub(in crate::cli) fn create(&self, record: &SessionRecord) -> Result<()> {
        fs::create_dir_all(self.dir(&record.id))?;
        let _write = liveness::lock_for_write(&self.write_lock_path(&record.id))?;
        if self.exists(record.id.as_str()) {
            bail!(
                "session '{}' already exists; pick another --id or remove it first",
                record.id
            );
        }
        self.write(record)
    }

    /// Apply one mutation to a session as a serialized read-modify-write.
    ///
    /// The blocking write lock makes read → apply → write one critical section
    /// across processes, so concurrent mutations all land, in some order, and
    /// none clobbers another's. A failed closure writes nothing and propagates
    /// its error. Returns the record as written.
    pub(in crate::cli) fn update(
        &self,
        id: &str,
        apply: impl FnOnce(&mut SessionRecord) -> Result<()>,
    ) -> Result<SessionRecord> {
        if !self.exists(id) {
            bail!("no session '{id}'");
        }
        let _write = liveness::lock_for_write(&self.write_lock_path(id))?;
        let mut record = self.open(id)?;
        apply(&mut record)?;
        self.write(&record)?;
        Ok(record)
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
                && record.version == VERSION
            {
                out.push(record);
            }
        }
        out.sort_by(|left, right| left.created.cmp(&right.created));
        Ok(out)
    }

    /// Delete one session's directory and everything in it, after any in-flight
    /// update finishes (the write lock serializes the two).
    pub(super) fn remove(&self, id: &str) -> Result<()> {
        let dir = self.dir(id);
        if !dir.is_dir() {
            return Ok(());
        }
        let _write = liveness::lock_for_write(&self.write_lock_path(id))?;
        fs::remove_dir_all(&dir)?;
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
pub(in crate::cli) fn mint_id(target: &str) -> Result<String> {
    Ok(format!("{target}-{}_{}", stamp()?, salt()))
}

/// Return the current UTC time formatted as RFC 3339. The sub-second precision
/// is kept: it is the tiebreaker that keeps `ls`/ambiguous ordering stable for
/// sessions created within the same second.
pub(in crate::cli) fn now() -> Result<String> {
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
    use crate::session::{SentenceLevel, SentenceTypeMix, WordCandidate};
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
            costs: ArtifactCosts::default(),
            rewrite: None,
            meta_request: None,
        }];
        record
    }

    #[test]
    fn a_created_session_reads_back_identically() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let saved = record("fr-1");
        store.create(&saved).expect("session must save");
        assert_eq!(
            store.open("fr-1").expect("session must reopen"),
            saved,
            "a created session no longer reads back identically"
        );
    }

    #[test]
    fn nondefault_sentence_settings_read_back_identically() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let saved = record("fr-settings").with_sentences(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Varied,
        ));
        store.create(&saved).expect("session must save");
        assert_eq!(
            store
                .open("fr-settings")
                .expect("session must reopen")
                .sentences,
            saved.sentences,
            "nondefault sentence settings no longer survive a session round trip"
        );
    }

    #[test]
    fn creating_over_an_existing_session_is_refused() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        store.create(&record("fr-1")).expect("first create");
        assert!(
            store.create(&record("fr-1")).is_err(),
            "creating over an existing session must be refused, not overwrite it"
        );
    }

    #[test]
    fn an_update_applies_the_closure_to_the_freshly_read_record() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        store.create(&record("fr-1")).expect("create");
        store
            .update("fr-1", |record| {
                record.phase = Phase::Published;
                Ok(())
            })
            .expect("update must apply");
        assert_eq!(
            store.open("fr-1").expect("reopen").phase,
            Phase::Published,
            "an update must persist the closure's mutation of the freshly read record"
        );
    }

    #[test]
    fn a_failed_closure_leaves_the_record_unwritten() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        store.create(&record("fr-1")).expect("create");
        let _ = store.update("fr-1", |record| {
            record.phase = Phase::Published;
            anyhow::bail!("refused")
        });
        assert_eq!(
            store.open("fr-1").expect("reopen").phase,
            Phase::Understood,
            "a failed update closure must leave the on-disk record untouched"
        );
    }

    #[test]
    fn a_record_from_an_older_build_is_refused_with_a_clear_error() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let dir = store.dir("old-1");
        fs::create_dir_all(&dir).expect("session dir must be created");
        fs::write(
            dir.join("session.json"),
            r#"{"version":1,"id":"old-1","created":"t","known":"en","learning":"fr","out":"/out","senses":"primary","source":"words","phase":"understood","drafts":[]}"#,
        )
        .expect("old session must be written");
        assert!(
            store
                .open("old-1")
                .expect_err("an old version must not open")
                .to_string()
                .contains("older build"),
            "a session from an older build must be refused with a remove-it hint"
        );
    }

    #[test]
    fn a_version_two_record_defaults_new_batch_and_draft_fields() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let dir = store.dir("legacy-2");
        fs::create_dir_all(&dir).expect("session dir must be created");
        fs::write(
            dir.join("session.json"),
            r#"{"version":2,"id":"legacy-2","created":"t","known":"EN","learning":"FR","out":"/out","senses":"primary","source":"words","phase":"understood","drafts":[{"term":"canard","understanding":"a duck"}]}"#,
        )
        .expect("version two session must be written");
        let opened = store
            .open("legacy-2")
            .expect("version two session must open");
        assert_eq!(
            (opened.sentences, opened.drafts[0].meta_request.as_ref()),
            (SentenceBatchSettings::default(), None),
            "a compatible session did not default its new batch or draft fields"
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
