//! Session-scoped provider spend persisted independently from worker progress.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::liveness;
use crate::generation::artifact_cache::Cache;
use crate::session::{Artifact, ArtifactCosts, GenerationCost};

const JOURNAL_SCHEMA: &str = "kamishibai.session-cost-journal";
const JOURNAL_VERSION: u8 = 1;
const WRITE_LOCK_FILE: &str = "write.lock";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RunIdentity {
    id: String,
    created: String,
}

impl RunIdentity {
    fn new(id: &str, created: &str) -> Self {
        Self {
            id: String::from(id),
            created: String::from(created),
        }
    }

    fn filename(&self) -> String {
        let identity = format!("{}\0{}", self.id, self.created);
        format!("costs-{:x}.json", md5::compute(identity.as_bytes()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct JournalDocument {
    schema: String,
    version: u8,
    run: RunIdentity,
    slots: Vec<ArtifactCosts>,
}

impl JournalDocument {
    fn new(run: RunIdentity, slots: Vec<ArtifactCosts>) -> Self {
        Self {
            schema: String::from(JOURNAL_SCHEMA),
            version: JOURNAL_VERSION,
            run,
            slots,
        }
    }

    fn validated(self, expected: &RunIdentity) -> Result<Self> {
        if self.schema != JOURNAL_SCHEMA || self.version != JOURNAL_VERSION {
            bail!("session cost journal has an unsupported schema");
        }
        if self.run != *expected {
            bail!("session cost journal belongs to a different run");
        }
        Ok(self)
    }
}

/// Durable absolute provider spend for one immutable session run identity.
#[derive(Clone, Debug)]
pub(in crate::cli) struct SessionCostJournal {
    directory: PathBuf,
    run: RunIdentity,
}

/// Late-bindable journal handle shared by a TUI session and cloned generators.
#[derive(Clone, Debug, Default)]
pub(in crate::cli) struct SessionCostScope {
    journal: Arc<Mutex<Option<SessionCostJournal>>>,
}

impl SessionCostScope {
    /// Build a bound scope directly from one session identity.
    #[cfg(test)]
    pub(in crate::cli) fn for_run(root: &Path, id: &str, created: &str) -> Self {
        Self::bound(SessionCostJournal::new(root, id, created))
    }

    /// Build a scope already bound to one console session run.
    pub(in crate::cli) fn bound(journal: SessionCostJournal) -> Self {
        Self {
            journal: Arc::new(Mutex::new(Some(journal))),
        }
    }

    /// Bind a fresh TUI scope once its session identity has been minted.
    pub(in crate::cli) fn bind(&self, journal: SessionCostJournal) -> Result<()> {
        let mut current = self
            .journal
            .lock()
            .map_err(|_| anyhow::anyhow!("session cost scope lock is poisoned"))?;
        if let Some(existing) = current.as_ref()
            && existing.address() != journal.address()
        {
            bail!("session cost scope cannot change run identity");
        }
        *current = Some(journal);
        Ok(())
    }

    /// Overlay the journal's absolute totals over a record or app snapshot.
    pub(in crate::cli) fn overlay(&self, fallback: &[ArtifactCosts]) -> Result<Vec<ArtifactCosts>> {
        self.journal()?.overlay(fallback)
    }

    /// Overlay when bound, or preserve the snapshot before a fresh TUI is claimed.
    pub(in crate::cli) fn overlay_if_bound(
        &self,
        fallback: &[ArtifactCosts],
    ) -> Result<Vec<ArtifactCosts>> {
        match self.optional_journal()? {
            Some(journal) => journal.overlay_existing(fallback),
            None => Ok(fallback.to_vec()),
        }
    }

    /// Persist one provider delta before downstream artifact settlement.
    pub(in crate::cli) fn charge(
        &self,
        slot: usize,
        artifact: Artifact,
        delta: GenerationCost,
    ) -> Result<ArtifactCosts> {
        self.journal()?.charge(slot, artifact, delta)
    }

    /// Return one slot's authoritative total after reconciling a live snapshot.
    pub(in crate::cli) fn absolute(
        &self,
        slot: usize,
        fallback: ArtifactCosts,
    ) -> Result<ArtifactCosts> {
        let length = slot
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("session cost slot overflow"))?;
        let mut slots = vec![ArtifactCosts::default(); length];
        slots[slot] = fallback;
        self.overlay(slots.as_slice())?
            .get(slot)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("session cost journal omitted card slot {slot}"))
    }

    fn journal(&self) -> Result<SessionCostJournal> {
        self.optional_journal()?
            .ok_or_else(|| anyhow::anyhow!("session cost scope is not bound to a run"))
    }

    fn optional_journal(&self) -> Result<Option<SessionCostJournal>> {
        let current = self
            .journal
            .lock()
            .map_err(|_| anyhow::anyhow!("session cost scope lock is poisoned"))?;
        Ok(current.clone())
    }
}

impl SessionCostJournal {
    /// Address one journal by cache root plus the stable session id and creation stamp.
    pub(in crate::cli) fn new(root: &Path, id: &str, created: &str) -> Self {
        Self {
            directory: root.join("sessions").join(id),
            run: RunIdentity::new(id, created),
        }
    }

    /// Create a missing journal from the session record without replacing existing authority.
    pub(in crate::cli) fn seed(&self, fallback: &[ArtifactCosts]) -> Result<()> {
        self.overlay(fallback).map(|_| ())
    }

    /// Return absolute per-slot totals, using record costs only for previously unseen slots.
    pub(in crate::cli) fn overlay(&self, fallback: &[ArtifactCosts]) -> Result<Vec<ArtifactCosts>> {
        fs::create_dir_all(&self.directory)?;
        let _guard = liveness::lock_for_write(&self.directory.join(WRITE_LOCK_FILE))?;
        let mut document = self
            .load()?
            .unwrap_or_else(|| JournalDocument::new(self.run.clone(), fallback.to_vec()));
        if document.slots.len() < fallback.len() {
            document
                .slots
                .extend_from_slice(&fallback[document.slots.len()..]);
        }
        self.write(&document)?;
        Ok(document.slots)
    }

    /// Read existing authority without creating a journal before a session claim succeeds.
    pub(in crate::cli) fn overlay_existing(
        &self,
        fallback: &[ArtifactCosts],
    ) -> Result<Vec<ArtifactCosts>> {
        if !self.directory.exists() {
            return Ok(fallback.to_vec());
        }
        let _guard = liveness::lock_for_write(&self.directory.join(WRITE_LOCK_FILE))?;
        let Some(mut document) = self.load()? else {
            return Ok(fallback.to_vec());
        };
        if document.slots.len() < fallback.len() {
            document
                .slots
                .extend_from_slice(&fallback[document.slots.len()..]);
        }
        Ok(document.slots)
    }

    /// Add one observed provider delta and return that slot's new absolute totals.
    pub(in crate::cli) fn charge(
        &self,
        slot: usize,
        artifact: Artifact,
        delta: GenerationCost,
    ) -> Result<ArtifactCosts> {
        fs::create_dir_all(&self.directory)?;
        let _guard = liveness::lock_for_write(&self.directory.join(WRITE_LOCK_FILE))?;
        let mut document = self
            .load()?
            .unwrap_or_else(|| JournalDocument::new(self.run.clone(), Vec::new()));
        if document.slots.len() <= slot {
            let length = slot
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("session cost slot overflow"))?;
            document.slots.resize(length, ArtifactCosts::default());
        }
        document.slots[slot] = document.slots[slot].charged(artifact, delta);
        let absolute = document.slots[slot];
        self.write(&document)?;
        Ok(absolute)
    }

    fn address(&self) -> (PathBuf, String) {
        (self.directory.clone(), self.run.filename())
    }

    fn load(&self) -> Result<Option<JournalDocument>> {
        let filename = self.run.filename();
        let cache = self.cache();
        if !cache.exists(filename.as_str()) {
            return Ok(None);
        }
        let path = cache.filepath(filename.as_str())?;
        let document = serde_json::from_slice::<JournalDocument>(fs::read(path)?.as_slice())?;
        Ok(Some(document.validated(&self.run)?))
    }

    fn write(&self, document: &JournalDocument) -> Result<()> {
        let filename = self.run.filename();
        let cache = self.cache();
        let staged = cache.stage(".costs.json")?;
        let result = serde_json::to_vec_pretty(document)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| fs::write(&staged, bytes).map_err(anyhow::Error::from))
            .and_then(|()| cache.commit(&staged, filename.as_str()));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result
    }

    fn cache(&self) -> Cache {
        Cache::new("", self.directory.clone())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::session::{Artifact, ArtifactCosts, GenerationCost};

    #[test]
    fn a_seeded_journal_overlays_a_stale_draft_with_its_absolute_total() {
        let home = TempDir::new().expect("tempdir must be created");
        let stale =
            ArtifactCosts::default().charged(Artifact::Picture, GenerationCost::from_nanos(100));
        let journal = SessionCostJournal::new(home.path(), "same-id", "created-a");
        journal.seed(&[stale]).expect("journal must be seeded");
        journal
            .charge(0, Artifact::Picture, GenerationCost::from_nanos(250))
            .expect("provider spend must be journaled");
        assert_eq!(
            journal.overlay(&[stale]).expect("stale draft must hydrate")[0].cost(Artifact::Picture),
            Some(GenerationCost::from_nanos(350)),
            "a restarted worker inherited stale session JSON instead of the journal's absolute total"
        );
    }

    #[test]
    fn recreating_a_session_id_cannot_inherit_the_previous_runs_journal() {
        let home = TempDir::new().expect("tempdir must be created");
        let first = SessionCostJournal::new(home.path(), "same-id", "created-a");
        first.seed(&[]).expect("first journal must seed");
        first
            .charge(0, Artifact::Meta, GenerationCost::from_nanos(500))
            .expect("first run spend must persist");
        let recreated = SessionCostJournal::new(home.path(), "same-id", "created-b");
        assert_eq!(
            recreated.overlay(&[]).expect("new journal must open"),
            Vec::<ArtifactCosts>::new(),
            "a recreated session id inherited provider spend from an older run"
        );
    }

    #[test]
    fn a_read_only_overlay_cannot_seed_authority_before_a_session_claim() {
        let home = TempDir::new().expect("tempdir must be created");
        let fallback =
            ArtifactCosts::default().charged(Artifact::Meta, GenerationCost::from_nanos(700));
        let journal = SessionCostJournal::new(home.path(), "same-id", "created-a");
        let overlay = journal
            .overlay_existing(&[fallback])
            .expect("read-only overlay must succeed");
        assert_eq!(
            (overlay, journal.directory.exists()),
            (vec![fallback], false),
            "a read-only preclaim overlay created journal authority before the session commit"
        );
    }
}
