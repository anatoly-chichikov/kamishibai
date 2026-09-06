//! Locked removal of one card's metadata and dependent cached artifacts.

use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_COST_FILE, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY,
    LEGACY_VISUAL_REVISION_FILE, META_COST_FILE, META_FILE, PICTURE_REQUESTS_FILE,
    ROOT_STAGE_LOCK_TIMEOUT, RootStage, RootStageGuard, SCENE_ATTEMPT_FILE, SCENE_COST_FILE,
    SCENE_FILE, VISUAL_LOCK_TIMEOUT, VOICE_COST_FILE, VOICE_FILE, VisualGuard,
};
use crate::generation::visual_revision;
#[cfg(test)]
use crate::session::LanguagePair;
use crate::session::{CardCell, CardDraft};

/// Remove one card's complete current artifact set under the destructive lock order.
#[cfg(test)]
pub(crate) fn invalidate_card(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
    keep_meta: bool,
    keep_meta_cost: bool,
) -> Result<()> {
    invalidate_cell(
        CardCell::new(root.to_path_buf(), pair, term, understanding),
        keep_meta,
        keep_meta_cost,
    )
}

/// Remove one reviewed draft's complete current artifact set under the lock order.
pub(crate) fn invalidate_draft(
    root: &Path,
    draft: &CardDraft,
    keep_meta: bool,
    keep_meta_cost: bool,
) -> Result<()> {
    invalidate_cell(
        CardCell::for_draft(root.to_path_buf(), draft),
        keep_meta,
        keep_meta_cost,
    )
}

fn invalidate_cell(cell: CardCell, keep_meta: bool, keep_meta_cost: bool) -> Result<()> {
    let cache = cell.cache();
    let visual = cache.visual(visual_revision())?;
    let _guards = ArtifactGuards::hold(&cache, &visual)?;
    remove_cached_files(
        &visual,
        &[
            SCENE_FILE,
            SCENE_ATTEMPT_FILE,
            SCENE_COST_FILE,
            ILLUSTRATION_FILE,
            ILLUSTRATION_COST_FILE,
            PICTURE_REQUESTS_FILE,
        ],
    )?;
    remove_attempt_journal(&visual)?;
    remove_cached_files(
        &cache,
        &[
            VOICE_FILE,
            VOICE_COST_FILE,
            SCENE_FILE,
            SCENE_COST_FILE,
            ILLUSTRATION_FILE,
            ILLUSTRATION_COST_FILE,
            PICTURE_REQUESTS_FILE,
            LEGACY_VISUAL_REVISION_FILE,
        ],
    )?;
    if !keep_meta {
        remove_cached_files(&cache, &[META_FILE])?;
    }
    if !keep_meta_cost {
        remove_cached_files(&cache, &[META_COST_FILE])?;
    }
    Ok(())
}

/// Every producer lease required for a destructive card-cache transaction.
pub(super) struct ArtifactGuards {
    _meta: RootStageGuard,
    _dependents: DependentGuards,
}

/// Voice and visual leases acquired after a metadata lease is already held.
pub(super) struct DependentGuards {
    _voice: RootStageGuard,
    _visual: VisualGuard,
}

impl ArtifactGuards {
    /// Acquire producer leases in the global metadata, voice, visual order.
    pub(super) fn hold(cache: &Cache, visual: &Cache) -> Result<Self> {
        let meta = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        let dependents = DependentGuards::hold(cache, visual)?;
        Ok(Self {
            _meta: meta,
            _dependents: dependents,
        })
    }
}

impl DependentGuards {
    /// Acquire dependent producer leases after the caller has locked metadata.
    pub(super) fn hold(cache: &Cache, visual: &Cache) -> Result<Self> {
        let voice = cache.hold_root_stage(RootStage::Voice, ROOT_STAGE_LOCK_TIMEOUT)?;
        let visual = visual.hold_visual(VISUAL_LOCK_TIMEOUT)?;
        Ok(Self {
            _voice: voice,
            _visual: visual,
        })
    }
}

/// Clear every artifact that depends on replaced metadata while retaining costs.
pub(super) fn clear_for_meta_refresh(cache: &Cache, visual: &Cache) -> Result<()> {
    remove_cached_files(
        visual,
        &[
            SCENE_FILE,
            SCENE_ATTEMPT_FILE,
            ILLUSTRATION_FILE,
            PICTURE_REQUESTS_FILE,
        ],
    )?;
    remove_attempt_journal(visual)?;
    remove_cached_files(
        cache,
        &[
            VOICE_FILE,
            SCENE_FILE,
            SCENE_ATTEMPT_FILE,
            ILLUSTRATION_FILE,
            PICTURE_REQUESTS_FILE,
            LEGACY_VISUAL_REVISION_FILE,
        ],
    )?;
    Ok(())
}

fn remove_cached_files(cache: &Cache, files: &[&str]) -> Result<()> {
    for file in files {
        let path = cache.path().join(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn remove_attempt_journal(cache: &Cache) -> Result<()> {
    let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    if attempts.exists() {
        fs::remove_dir_all(attempts)?;
    }
    Ok(())
}
