//! Reconciles and reserves durable scene-attempt indices for visual generation.

use std::fs;

use anyhow::{Result, anyhow, bail};

use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, SCENE_ATTEMPT_FILE, SCENE_FILE,
};
use crate::session::Artifact;

/// Reconciles committed, archived, and durably reserved scene attempts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    if !advance
        && cursor
            .attempted
            .is_some_and(|attempted| attempted > selected)
    {
        return Ok(selected);
    }
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
