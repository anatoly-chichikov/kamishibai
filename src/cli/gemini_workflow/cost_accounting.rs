//! Records provider spend in artifact and session journals.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use anyhow::Result;

use crate::cli::session::SessionCostScope;
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_COST_FILE, META_COST_FILE, ROOT_STAGE_LOCK_TIMEOUT, RootStage,
    SCENE_COST_FILE, VOICE_COST_FILE,
};
use crate::session::{Artifact, ArtifactAttempt, CostRecord, GenerationCost};

#[derive(Clone, Debug)]
/// Attributes provider cost to one stable card slot in a session.
pub(super) struct SessionCostAttribution {
    scope: SessionCostScope,
    slot: usize,
}

impl SessionCostAttribution {
    /// Bind a session journal to one stable card slot.
    pub(super) fn new(scope: SessionCostScope, slot: usize) -> Self {
        Self { scope, slot }
    }

    fn charge(&self, artifact: Artifact, delta: GenerationCost) -> Result<()> {
        self.scope.charge(self.slot, artifact, delta).map(|_| ())
    }
}

#[derive(Clone, Debug)]
/// Shares fail-fast accounting health across one provider operation.
pub(super) struct AccountingHealth {
    failed: Rc<std::cell::Cell<bool>>,
}

impl AccountingHealth {
    fn new(failed: Rc<std::cell::Cell<bool>>) -> Self {
        Self { failed }
    }

    /// Remember accounting failure while preserving the original result.
    pub(super) fn record<T>(&self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.failed.set(true);
        }
        result
    }

    /// Report whether accounting failed in this operation.
    pub(super) fn failed(&self) -> bool {
        self.failed.get()
    }
}

impl Default for AccountingHealth {
    fn default() -> Self {
        Self::new(Rc::new(std::cell::Cell::new(false)))
    }
}

#[derive(Clone, Debug)]
struct CostState {
    observed: Rc<RefCell<Option<CostRecord>>>,
    accounting: AccountingHealth,
}

impl CostState {
    fn new(accounting: AccountingHealth) -> Self {
        Self {
            observed: Rc::new(RefCell::new(None)),
            accounting,
        }
    }
}

#[derive(Clone, Debug)]
/// Persists provider usage and optionally journals it to a session slot.
pub(super) struct CostRecorder {
    cache: Cache,
    artifact: Artifact,
    state: CostState,
    session: Option<SessionCostAttribution>,
}

impl CostRecorder {
    #[cfg(test)]
    /// Build an unattributed recorder for an isolated test cache.
    pub(super) fn new(cache: Cache, artifact: Artifact) -> Self {
        Self::attributed(cache, artifact, None)
    }

    #[cfg(test)]
    /// Build a recorder with optional session attribution for tests.
    pub(super) fn attributed(
        cache: Cache,
        artifact: Artifact,
        session: Option<SessionCostAttribution>,
    ) -> Self {
        Self::guarded(cache, artifact, session, AccountingHealth::default())
    }

    /// Build a recorder sharing accounting health with its provider boundary.
    pub(super) fn guarded(
        cache: Cache,
        artifact: Artifact,
        session: Option<SessionCostAttribution>,
        accounting: AccountingHealth,
    ) -> Self {
        Self {
            cache,
            artifact,
            state: CostState::new(accounting),
            session,
        }
    }

    /// Persist usage observed for a generated artifact.
    pub(super) fn push(&self, record: CostRecord) -> Result<()> {
        if record.requests() == 0 {
            return Ok(());
        }
        let result = self
            .observe(&record)
            .and_then(|()| store_cost(&self.cache, self.artifact, &record).map(|_| ()));
        self.state.accounting.record(result)
    }

    /// Persist usage observed for a metadata correction.
    pub(super) fn push_correction(&self, record: CostRecord) -> Result<()> {
        if record.requests() == 0 {
            return Ok(());
        }
        let result = self
            .observe(&record)
            .and_then(|()| persist_correction_cost(&self.cache, &record));
        self.state.accounting.record(result)
    }

    fn observe(&self, record: &CostRecord) -> Result<()> {
        if let Some(session) = self.session.as_ref() {
            session.charge(self.artifact, record.cost())?;
        }
        let aggregate = self
            .state
            .observed
            .borrow()
            .as_ref()
            .map(|current| current.merged(record))
            .unwrap_or_else(|| record.clone());
        *self.state.observed.borrow_mut() = Some(aggregate);
        Ok(())
    }

    /// Return usage from this operation rather than historical usage.
    pub(super) fn current(&self, cached: bool) -> Result<Option<GenerationCost>> {
        if cached {
            return Ok(None);
        }
        Ok(self.state.observed.borrow().as_ref().map(CostRecord::cost))
    }

    /// Return all usage observed during this operation.
    pub(super) fn cumulative(&self, cached: bool) -> Result<Option<GenerationCost>> {
        self.current(cached)
    }
}

#[cfg(test)]
/// Load a persisted cost value for an assertion.
pub(super) fn load_cost(cache: &Cache, artifact: Artifact) -> Result<Option<GenerationCost>> {
    Ok(load_cost_record(cache, artifact)?.map(|record| record.cost()))
}

/// Load the persisted usage record for an artifact.
pub(super) fn load_cost_record(cache: &Cache, artifact: Artifact) -> Result<Option<CostRecord>> {
    let filename = cost_filename(artifact);
    if !cache.exists(filename) {
        return Ok(None);
    }
    let path = cache.filepath(filename)?;
    let text = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str::<CostRecord>(text.as_str())?))
}

/// Merge and atomically persist one provider usage record.
pub(super) fn store_cost(
    cache: &Cache,
    artifact: Artifact,
    record: &CostRecord,
) -> Result<CostRecord> {
    if record.requests() == 0 {
        return Ok(load_cost_record(cache, artifact)?.unwrap_or_else(|| record.clone()));
    }
    let merged = load_cost_record(cache, artifact)?
        .map(|existing| existing.merged(record))
        .unwrap_or_else(|| record.clone());
    let staged = cache.stage(".cost.json")?;
    let result = serde_json::to_string_pretty(&merged)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&staged, json).map_err(anyhow::Error::from))
        .and_then(|()| cache.commit(&staged, cost_filename(artifact)));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result?;
    Ok(merged)
}

/// Persist correction usage while holding the metadata lease.
pub(super) fn persist_correction_cost(cache: &Cache, record: &CostRecord) -> Result<()> {
    let _guard = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
    store_cost(cache, Artifact::Meta, record)?;
    Ok(())
}

/// Settle direct and related costs for a visual operation.
pub(super) fn visual_costs(
    artifact: Artifact,
    cached: bool,
    scene: &CostRecorder,
    picture: &CostRecorder,
) -> Result<(Option<GenerationCost>, Option<GenerationCost>)> {
    match artifact {
        Artifact::Scene => Ok((scene.cumulative(cached)?, None)),
        Artifact::Picture => Ok((picture.cumulative(cached)?, scene.current(cached)?)),
        Artifact::Meta | Artifact::Sound => {
            panic!("visual cost settlement requires scene or picture")
        }
    }
}

/// Attach recomposed-scene cost to a picture attempt.
pub(super) fn attach_scene_cost<T>(
    attempt: ArtifactAttempt<T>,
    artifact: Artifact,
    scene: Option<GenerationCost>,
) -> ArtifactAttempt<T> {
    match (artifact, scene) {
        (Artifact::Picture, Some(cost)) => attempt.with_related_cost(Artifact::Scene, cost),
        (_, _) => attempt,
    }
}

fn cost_filename(artifact: Artifact) -> &'static str {
    match artifact {
        Artifact::Meta => META_COST_FILE,
        Artifact::Scene => SCENE_COST_FILE,
        Artifact::Picture => ILLUSTRATION_COST_FILE,
        Artifact::Sound => VOICE_COST_FILE,
    }
}
