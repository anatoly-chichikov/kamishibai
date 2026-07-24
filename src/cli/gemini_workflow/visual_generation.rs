//! Owns visual leases, scene-attempt cursors, and picture recovery policy.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::{AccountingHealth, picture_request_total};
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, SCENE_ATTEMPT_FILE, SCENE_FILE, VisualGuard,
};
use crate::generation::manga::Progress as SceneProgress;
use crate::session::Artifact;

/// Acquire visual leases in stable order and deduplicate cache paths.
pub(super) fn hold_visuals(mut visuals: Vec<Cache>, timeout: Duration) -> Result<Vec<VisualGuard>> {
    visuals.sort_by_key(Cache::path);
    visuals.dedup_by(|left, right| left.path() == right.path());
    visuals
        .iter()
        .map(|visual| visual.hold_visual(timeout))
        .collect()
}

/// Discards scene progress when no surface consumes it.
pub(super) struct NoopProgress;

impl SceneProgress for NoopProgress {
    fn step(&mut self, _name: &str) {}

    fn done(&mut self, _name: &str, _label: &str, _path: Option<&Path>) {}
}

/// Fall back to the committed scene only before a recomposed image request.
pub(super) fn render_recomposition_with_fallback<T, P, R, F, E>(
    cache: &Cache,
    accounting: &AccountingHealth,
    progress: &mut P,
    recompose: R,
    fallback: F,
) -> std::result::Result<T, E>
where
    R: FnOnce(&mut P) -> std::result::Result<T, E>,
    F: FnOnce(&mut P) -> std::result::Result<T, E>,
    E: From<anyhow::Error>,
{
    let before = accounting
        .record(picture_request_total(cache))
        .map_err(E::from)?;
    let original = match recompose(progress) {
        Ok(rendered) => return Ok(rendered),
        Err(error) => error,
    };
    if accounting.failed() {
        return Err(original);
    }
    let after = accounting
        .record(picture_request_total(cache))
        .map_err(E::from)?;
    if after != before {
        return Err(original);
    }
    match fallback(progress) {
        Ok(rendered) => Ok(rendered),
        Err(error) if accounting.failed() => Err(error),
        Err(error)
            if accounting
                .record(picture_request_total(cache))
                .map_err(E::from)?
                != after =>
        {
            Err(error)
        }
        Err(_) => Err(original),
    }
}

#[derive(Clone, Debug, Default)]
/// Decides whether the next picture attempt needs a recomposed scene.
pub(super) struct PictureRecovery {
    states: Arc<Mutex<BTreeMap<PathBuf, PictureRecoveryState>>>,
}

impl PictureRecovery {
    /// Synchronize recovery state and decide whether to recompose.
    pub(super) fn prepare(&self, path: &Path, done: u8) -> Result<bool> {
        let persisted = persisted_local_rejections(path)?;
        let mut states = self
            .states
            .lock()
            .map_err(|_| anyhow!("picture recovery state lock is poisoned"))?;
        if done == 0 {
            states.insert(
                path.to_path_buf(),
                PictureRecoveryState {
                    observed_attempts: 0,
                    rejections: persisted,
                },
            );
        } else {
            let state = states
                .entry(path.to_path_buf())
                .or_insert_with(|| PictureRecoveryState {
                    observed_attempts: done,
                    rejections: persisted,
                });
            if state.observed_attempts != done {
                *state = PictureRecoveryState {
                    observed_attempts: done,
                    rejections: persisted,
                };
            } else {
                state.rejections = state.rejections.synchronized(persisted);
            }
        }
        Ok(states
            .get(path)
            .is_some_and(|state| state.rejections.recompose()))
    }

    /// Observe one completed local validation attempt.
    pub(super) fn observe(
        &self,
        path: &Path,
        done: u8,
        local_rejection: Option<LocalImageRejection>,
    ) -> Result<()> {
        let mut states = self
            .states
            .lock()
            .map_err(|_| anyhow!("picture recovery state lock is poisoned"))?;
        let state = states.entry(path.to_path_buf()).or_default();
        if state.observed_attempts != done {
            *state = PictureRecoveryState {
                observed_attempts: done,
                rejections: LocalImageRejections::default(),
            };
        }
        state.observed_attempts = done.saturating_add(1);
        if let Some(rejection) = local_rejection {
            state.rejections = state.rejections.pushed(rejection);
        }
        Ok(())
    }
}

/// Rebuild recent local rejection state from archived attempt verdicts.
pub(super) fn persisted_local_rejections(path: &Path) -> Result<LocalImageRejections> {
    let directory = path.join(IMAGE_ATTEMPTS_DIRECTORY);
    if !directory.is_dir() {
        return Ok(LocalImageRejections::default());
    }
    let mut verdicts = fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let sequence = name
                .strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<usize>()
                .ok()?;
            Some((sequence, entry.path()))
        })
        .collect::<Vec<_>>();
    verdicts.sort_by_key(|(sequence, _)| *sequence);
    verdicts.iter().try_fold(
        LocalImageRejections::default(),
        |rejections, (_, verdict)| {
            let value = fs::read(verdict)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let local_rejection = value.as_ref().and_then(LocalImageRejection::from_verdict);
            Ok(local_rejection.map_or(rejections, |rejection| rejections.pushed(rejection)))
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Classifies local image rejection signals relevant to recovery.
pub(super) enum LocalImageRejection {
    Color,
    Topology,
    RecallText,
    Border,
    LegacyGutter,
    Other,
}

impl LocalImageRejection {
    /// Map a renderer category to the recovery vocabulary.
    pub(super) fn from_category(category: &str) -> Self {
        match category {
            "color" => Self::Color,
            "topology" => Self::Topology,
            "ocr" | "recall_text" => Self::RecallText,
            "border" => Self::Border,
            "legacy_gutter" => Self::LegacyGutter,
            _ => Self::Other,
        }
    }

    /// Decode a persisted renderer verdict into the recovery vocabulary.
    pub(super) fn from_verdict(value: &serde_json::Value) -> Option<Self> {
        if value.get("status").and_then(serde_json::Value::as_str) != Some("rejected") {
            return None;
        }
        value
            .get("category")
            .and_then(serde_json::Value::as_str)
            .map(Self::from_category)
            .or_else(|| {
                matches!(
                    value.get("reason").and_then(serde_json::Value::as_str),
                    Some(
                        "Registered panel topology was not detected"
                            | "Unexpected internal gutter in one-panel layout"
                    )
                )
                .then_some(Self::Topology)
            })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Retains the two rejection signals that control the next attempt.
pub(super) struct LocalImageRejections {
    recent: [Option<LocalImageRejection>; 2],
}

impl LocalImageRejections {
    /// Append a rejection while keeping the bounded history.
    pub(super) fn pushed(self, rejection: LocalImageRejection) -> Self {
        Self {
            recent: [self.recent[1], Some(rejection)],
        }
    }

    /// Decide whether the bounded history requires scene recomposition.
    pub(super) fn recompose(&self) -> bool {
        matches!(
            self.recent,
            [Some(LocalImageRejection::Topology), Some(_)]
                | [Some(_), Some(LocalImageRejection::Topology)]
                | [
                    Some(LocalImageRejection::Border),
                    Some(LocalImageRejection::Border)
                ]
        )
    }

    fn synchronized(self, persisted: Self) -> Self {
        if persisted.len() > self.len() {
            persisted
        } else {
            self
        }
    }

    fn len(&self) -> usize {
        self.recent.iter().flatten().count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Reconciles committed, archived, and durably reserved scene attempts.
pub(super) struct SceneAttemptCursor {
    /// Attempt stored in the current scene.
    pub(super) committed: Option<u8>,
    /// Highest attempt found in the image-attempt archive.
    pub(super) archived: Option<u8>,
    /// Highest attempt already reserved by this visual revision.
    pub(super) attempted: Option<u8>,
}

impl SceneAttemptCursor {
    /// Report whether an uncommitted recomposed scene was rejected.
    pub(super) fn has_rejected_recomposition(&self) -> bool {
        self.committed
            .zip(self.attempted)
            .is_some_and(|(committed, attempted)| attempted > committed)
    }

    /// Reconcile requested recovery with persisted scene state.
    pub(super) fn recompose(&self, requested: bool) -> bool {
        let unrendered = self
            .committed
            .is_some_and(|committed| self.archived.is_none_or(|archived| committed > archived));
        self.has_rejected_recomposition() || requested && !unrendered
    }

    /// Select the current attempt without advancing the cursor.
    pub(super) fn current(&self, fallback: u8) -> u8 {
        self.committed.or(self.attempted).unwrap_or(fallback)
    }

    /// Select the next attempt while detecting overflow.
    pub(super) fn next(&self, fallback: u8) -> Result<u8> {
        self.attempted.map_or(Ok(fallback), |attempted| {
            attempted
                .checked_add(1)
                .map(|next| next.max(fallback))
                .ok_or_else(|| anyhow!("scene attempt index overflow"))
        })
    }
}

/// Rebuild the scene-attempt cursor from durable visual state.
pub(super) fn scene_attempt_cursor(cache: &Cache, fallback: u8) -> Result<SceneAttemptCursor> {
    let committed = if cache.exists(SCENE_FILE) {
        let scene = serde_json::from_slice::<serde_json::Value>(
            fs::read(cache.path().join(SCENE_FILE))?.as_slice(),
        )?;
        scene_attempt_index(&scene)?.or(Some(fallback))
    } else {
        None
    };
    let reserved = load_scene_attempt(cache)?;
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    let mut attempted = committed.into_iter().chain(reserved).max();
    let mut archived = None;
    if directory.is_dir() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if !name.starts_with("attempt-") || !name.ends_with(".scene.json") {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(scene) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if let Some(index) = scene_attempt_index(&scene)? {
                archived = archived.into_iter().chain(Some(index)).max();
            }
        }
    }
    attempted = attempted.into_iter().chain(archived).max();
    Ok(SceneAttemptCursor {
        committed,
        archived,
        attempted,
    })
}

/// Durably select the scene attempt used by one visual operation.
pub(super) fn reserve_scene_attempt(
    cache: &Cache,
    artifact: Artifact,
    fallback: u8,
    recompose: bool,
) -> Result<u8> {
    let cursor = scene_attempt_cursor(cache, fallback)?;
    let advance = match artifact {
        Artifact::Scene => !cache.exists(SCENE_FILE),
        Artifact::Picture => recompose && !cache.exists(ILLUSTRATION_FILE),
        Artifact::Meta | Artifact::Sound => {
            bail!("scene attempt reservation requires scene or picture")
        }
    };
    let selected = if advance {
        cursor.next(fallback)?
    } else {
        cursor.current(fallback)
    };
    store_scene_attempt(cache, selected)?;
    Ok(selected)
}

/// Load the durably reserved scene-attempt index.
pub(super) fn load_scene_attempt(cache: &Cache) -> Result<Option<u8>> {
    if !cache.exists(SCENE_ATTEMPT_FILE) {
        return Ok(None);
    }
    let value = serde_json::from_slice::<serde_json::Value>(
        fs::read(cache.path().join(SCENE_ATTEMPT_FILE))?.as_slice(),
    )?;
    value
        .get("scene_attempt_index")
        .and_then(serde_json::Value::as_u64)
        .map(u8::try_from)
        .transpose()
        .map_err(anyhow::Error::from)
}

fn store_scene_attempt(cache: &Cache, attempt: u8) -> Result<()> {
    if let Some(current) = load_scene_attempt(cache)? {
        if current > attempt {
            bail!("scene attempt cursor cannot move backwards from {current} to {attempt}");
        }
        if current == attempt {
            return Ok(());
        }
    }
    let staged = cache.stage(".scene-attempt.json")?;
    let result = serde_json::to_vec_pretty(&serde_json::json!({
        "scene_attempt_index": attempt
    }))
    .map_err(anyhow::Error::from)
    .and_then(|bytes| fs::write(&staged, bytes).map_err(anyhow::Error::from))
    .and_then(|()| cache.commit(&staged, SCENE_ATTEMPT_FILE));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn scene_attempt_index(scene: &serde_json::Value) -> Result<Option<u8>> {
    scene
        .pointer("/manga_panel/meta/layout_selection/scene_attempt_index")
        .and_then(serde_json::Value::as_u64)
        .map(u8::try_from)
        .transpose()
        .map_err(anyhow::Error::from)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PictureRecoveryState {
    observed_attempts: u8,
    rejections: LocalImageRejections,
}
