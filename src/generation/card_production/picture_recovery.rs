//! Chooses picture fallback and scene-recomposition recovery from durable evidence.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

use super::cost_accounting::AccountingHealth;
use super::picture_requests::picture_request_total;
use crate::generation::artifact_cache::{Cache, IMAGE_ATTEMPTS_DIRECTORY};
use crate::generation::manga::Progress as SceneProgress;

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
    ///
    /// Repeated text or fidelity rejections indict the scene itself — the
    /// composed props or per-panel demands keep producing prohibited or
    /// undrawable content — so two of them in a row stop re-rolling the
    /// same scene and recompose it instead.
    pub(super) fn recompose(&self) -> bool {
        matches!(
            self.recent,
            [Some(LocalImageRejection::Topology), Some(_)]
                | [Some(_), Some(LocalImageRejection::Topology)]
                | [
                    Some(LocalImageRejection::Border),
                    Some(LocalImageRejection::Border)
                ]
                | [
                    Some(LocalImageRejection::RecallText),
                    Some(LocalImageRejection::RecallText)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PictureRecoveryState {
    observed_attempts: u8,
    rejections: LocalImageRejections,
}
