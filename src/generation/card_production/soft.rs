//! Persists moderation and the remaining shared visual budget under its lease.

use std::fs;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::picture_requests::{load_picture_request_counter, picture_request_total};
use crate::generation::artifact_cache::{Cache, IMAGE_ATTEMPTS_DIRECTORY, SCENE_FILE, SOFT_FILE};
use crate::generation::{SceneMode, VisualFailure, VisualResult};
use crate::models::GenerationStage;
use crate::session::{ARTIFACT_ATTEMPT_CEILING, StateFailure};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Moderation {
    stage: String,
    attempt: u8,
    terminal: bool,
    budget: SoftBudget,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SoftBudget {
    remaining: u8,
    pictures: u32,
    failures: u8,
}

/// Resolve the durable world and enforce the shared four-attempt ceiling.
pub(super) fn mode(cache: &Cache) -> VisualResult<SceneMode> {
    restore(cache).map_err(StateFailure::new)?;
    let Some(state) = load(cache).map_err(StateFailure::new)? else {
        return Ok(SceneMode::Stock);
    };
    let stage = match state.stage.as_str() {
        "features" => GenerationStage::Features,
        "scene" => GenerationStage::Scene,
        "picture" => GenerationStage::Picture,
        _ => return Err(anyhow::anyhow!("moderation cache has an invalid stage").into()),
    };
    if state.terminal {
        return Err(VisualFailure::Refused { stage });
    }
    let pictures = picture_request_total(cache)?
        .checked_sub(state.budget.pictures)
        .ok_or_else(|| anyhow::anyhow!("moderation picture counter moved backwards"))?;
    if pictures + u32::from(state.budget.failures) >= u32::from(state.budget.remaining) {
        return Err(anyhow::anyhow!("visual generation exhausted its four-attempt ceiling").into());
    }
    Ok(SceneMode::Soft)
}

/// Commit a moderation transition under the caller-held visual lease.
pub(super) fn refuse(
    cache: &Cache,
    stage: GenerationStage,
    attempt: u8,
    mode: SceneMode,
    spent: u8,
) -> Result<()> {
    let stage = match stage {
        GenerationStage::Features => "features",
        GenerationStage::Scene => "scene",
        GenerationStage::Picture => "picture",
        _ => bail!("moderation transition requires a scene or picture stage"),
    };
    let budget = match load(cache)? {
        Some(state) => state.budget,
        None => SoftBudget {
            remaining: ARTIFACT_ATTEMPT_CEILING.saturating_sub(spent.saturating_add(1)),
            pictures: picture_request_total(cache)?,
            failures: 0,
        },
    };
    let state = Moderation {
        stage: stage.into(),
        attempt,
        terminal: mode == SceneMode::Soft,
        budget,
    };
    if stage != "picture" {
        let directory = cache.filepath(IMAGE_ATTEMPTS_DIRECTORY)?;
        fs::create_dir_all(&directory)?;
        let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
        serde_json::to_writer_pretty(
            staged.as_file_mut(),
            &serde_json::json!({
                "category": "moderation", "soft_state": state
            }),
        )?;
        staged.as_file().sync_all()?;
        staged.persist(directory.join(format!("moderation-{attempt:04}.json")))?;
    }
    store(cache, &state)
}

/// Count a failed soft pass only when it did not already reserve an image.
pub(super) fn reject(cache: &Cache, pictures: u32) -> Result<()> {
    let mut state =
        load(cache)?.ok_or_else(|| anyhow::anyhow!("soft rejection has no moderation state"))?;
    if picture_request_total(cache)? == pictures {
        state.budget.failures = state
            .budget
            .failures
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("soft rejection counter overflow"))?;
        store(cache, &state)?;
    }
    Ok(())
}

/// Require a soft replacement while the committed scene remains stock.
pub(super) fn recompose(cache: &Cache) -> Result<bool> {
    if !cache.exists(SCENE_FILE) {
        let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
        if !directory.is_dir() {
            return Ok(false);
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !(name.starts_with("attempt-") || name.starts_with("moderation-"))
                || !name.ends_with(".json")
                || name.ends_with(".scene.json")
            {
                continue;
            }
            let value: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            if value["retry_requested"] == true && value["recompose_requested"] == true {
                return Ok(true);
            }
        }
        return Ok(cache.exists(SOFT_FILE));
    }
    if !cache.exists(SOFT_FILE) {
        return Ok(false);
    }
    let scene: serde_json::Value =
        serde_json::from_slice(&fs::read(cache.path().join(SCENE_FILE))?)?;
    Ok(scene
        .pointer("/manga_panel/meta/layout_selection/scene_mode")
        .and_then(serde_json::Value::as_str)
        != Some("soft"))
}

/// Rebuild incomplete moderation transitions from archived refusal evidence.
pub(super) fn restore(cache: &Cache) -> Result<()> {
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    if !directory.is_dir() {
        return Ok(());
    }
    let mut pictures = Vec::new();
    let mut evidence = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(sequence) = name
            .strip_prefix("attempt-")
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|name| name.parse::<u32>().ok())
        {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            pictures.push((sequence, value));
        } else if name.starts_with("moderation-") && name.ends_with(".json") {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            if value["retry_requested"] != true {
                evidence.push(serde_json::from_value::<Moderation>(
                    value["soft_state"].clone(),
                )?);
            }
        }
    }
    let counter = load_picture_request_counter(cache)?;
    for (sequence, value) in &pictures {
        if value["category"] != "moderation" || value["retry_requested"] == true {
            continue;
        }
        let name = value["scene"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("moderation verdict has no scene"))?;
        if std::path::Path::new(name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(name)
        {
            bail!("moderation verdict scene is not a filename");
        }
        let scene: serde_json::Value = serde_json::from_slice(&fs::read(directory.join(name))?)?;
        let after = u32::try_from(
            pictures
                .iter()
                .filter(|(later, value)| later > sequence && value["status"] != "not_sent")
                .count(),
        )?;
        let pictures = counter
            .requests
            .checked_sub(after)
            .ok_or_else(|| anyhow::anyhow!("moderation archive exceeds its picture counter"))?;
        let spent = counter
            .series_requests
            .unwrap_or(counter.requests)
            .saturating_sub(after);
        evidence.push(Moderation {
            stage: "picture".into(),
            attempt: u8::try_from(
                scene
                    .pointer("/manga_panel/meta/layout_selection/scene_attempt_index")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("moderated scene has no attempt index"))?,
            )?,
            terminal: scene
                .pointer("/manga_panel/meta/layout_selection/scene_mode")
                .and_then(serde_json::Value::as_str)
                == Some("soft"),
            budget: SoftBudget {
                remaining: ARTIFACT_ATTEMPT_CEILING.saturating_sub(u8::try_from(spent)?),
                pictures,
                failures: 0,
            },
        });
    }
    evidence.sort_by_key(|state| state.attempt);
    let mut state = load(cache)?;
    let mut changed = false;
    for refused in evidence {
        match &mut state {
            None => {
                state = Some(refused);
                changed = true;
            }
            Some(state) if refused.attempt > state.attempt => {
                state.stage = refused.stage;
                state.attempt = refused.attempt;
                state.terminal = true;
                changed = true;
            }
            Some(state) if refused.terminal && !state.terminal => {
                state.terminal = true;
                changed = true;
            }
            Some(_) => {}
        }
    }
    if changed && let Some(state) = state {
        store(cache, &state)?;
    }
    Ok(())
}

/// Retire explicit scene-retry intent once a replacement scene is committed.
pub(super) fn settle(cache: &Cache) -> Result<()> {
    if !cache.exists(SCENE_FILE) {
        return Ok(());
    }
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with("attempt-") || name.starts_with("moderation-"))
            || !name.ends_with(".json")
            || name.ends_with(".scene.json")
        {
            continue;
        }
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
        if value["recompose_requested"] == true {
            value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("scene retry verdict is not an object"))?
                .remove("recompose_requested");
            let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
            serde_json::to_writer_pretty(staged.as_file_mut(), &value)?;
            staged.as_file().sync_all()?;
            staged.persist(entry.path())?;
        }
    }
    Ok(())
}

fn load(cache: &Cache) -> Result<Option<Moderation>> {
    if !cache.exists(SOFT_FILE) {
        return Ok(None);
    }
    let state: Moderation = serde_json::from_slice(&fs::read(cache.path().join(SOFT_FILE))?)?;
    if state.budget.remaining >= ARTIFACT_ATTEMPT_CEILING
        || state.budget.failures > state.budget.remaining
    {
        bail!("moderation cache has an invalid remaining budget");
    }
    Ok(Some(state))
}

fn store(cache: &Cache, state: &Moderation) -> Result<()> {
    let staged = cache.stage(".soft.json")?;
    let result = fs::write(&staged, serde_json::to_vec_pretty(state)?)
        .map_err(anyhow::Error::from)
        .and_then(|()| cache.commit(&staged, SOFT_FILE));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}
