//! The lifecycle-maintenance verbs: `cancel` (stop a running worker), `rm`
//! (delete a session, optionally its cached cards), and `cache-path`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tempfile::Builder;

use crate::generation::artifact_cache::{Cache, VISUAL_DIRECTORY, VISUAL_LOCK_TIMEOUT};
use crate::generation::visual_revision;
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardCell, LanguagePair};

use super::args::{IdArg, RmArgs};
use super::liveness;
use super::store::{Phase, SessionRecord, SessionStore};
use super::{Render, json, refuse_if_live, resolve, view};

/// Stop a session's running worker and mark it cancelled unless already terminal.
///
/// The pid is signalled only while the liveness lock proves a live worker owns
/// it, so a stale (possibly reused) pid is never sent a signal; the brief window
/// between that probe and the signal is accepted as residual. The final state is
/// written as one serialized update, so cancel never fails on a concurrency
/// race: a worker that finished first stays published, anything else settles
/// cancelled.
pub(super) fn cancel(args: &IdArg, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let opened = resolve(&store, args.id.as_deref(), render)?;
    if let Some(worker) = &opened.worker
        && liveness::is_held(&store.lock_path(opened.id.as_str()))
    {
        liveness::terminate(worker.pid);
    }
    let updated = store.update(opened.id.as_str(), |record| {
        record.worker = None;
        record.progress = None;
        if !matches!(
            record.phase,
            Phase::Published | Phase::Partial | Phase::Failed | Phase::Cancelled
        ) {
            record.phase = Phase::Cancelled;
        }
        Ok(())
    })?;
    if matches!(render, Render::Json) {
        return json::emit_session(&updated);
    }
    let phase = updated.phase;
    println!("{}", view::header(&updated, phase));
    if matches!(phase, Phase::Cancelled) {
        println!(
            "Stopped the worker — nothing published. What built so far stays cached, so a later generate resumes."
        );
    } else {
        println!(
            "Nothing to stop — the session is already {}.",
            view::phase_label(phase)
        );
    }
    Ok(())
}

/// Delete a session, and with `--cache` its cached card folders too.
pub(super) fn rm(args: &RmArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    refuse_if_live(&store, &record)?;
    if args.cache {
        let root = cache_root(&SystemContext)?;
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        for (term, understanding) in cached_cells(&record) {
            purge_artifacts(root.as_path(), &pair, term.as_str(), understanding.as_str())?;
        }
    }
    store.remove(record.id.as_str())?;
    if matches!(render, Render::Json) {
        return json::emit(&json::RemovedDoc::of(record.id.as_str()));
    }
    if args.cache {
        println!(
            "Removed session {} and its cached cards — nothing left to reuse.",
            record.id
        );
    } else {
        println!(
            "Removed session {}. Its cached cards stay (reused if you recreate it) — add --cache to delete those too.",
            record.id
        );
    }
    Ok(())
}

/// Print the cache directory and exit.
pub(super) fn cache_path(render: Render) -> Result<()> {
    let root = cache_root(&SystemContext)?;
    if matches!(render, Render::Json) {
        return json::emit(&json::CacheDoc::of(root.as_path()));
    }
    println!("{}", root.display());
    Ok(())
}

/// Return every (term, understanding) cache cell a session may own: the committed
/// plan when one exists, otherwise each candidate sense (so an imported or
/// understood session still has its pre-stored meta cells removed).
fn cached_cells(record: &SessionRecord) -> Vec<(String, String)> {
    if !record.drafts.is_empty() {
        return record
            .drafts
            .iter()
            .map(|draft| (draft.term.clone(), draft.understanding.clone()))
            .collect();
    }
    record
        .candidates
        .iter()
        .flat_map(|stored| {
            let candidate = stored.clone().candidate();
            candidate
                .senses()
                .iter()
                .map(|sense| {
                    (
                        candidate.term().to_string(),
                        sense.understanding().to_string(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Delete one card's entire cache folder after leasing every visual revision.
fn purge_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> Result<()> {
    let cache = CardCell::new(root.to_path_buf(), pair, term, understanding).cache();
    let folder = cache.path();
    if !folder.exists() {
        return Ok(());
    }
    let mut revisions = visual_revisions(folder.as_path())?;
    revisions.insert(visual_revision().to_string());
    let mut guards = Vec::with_capacity(revisions.len());
    for revision in &revisions {
        guards.push(
            cache
                .visual(revision.as_str())?
                .hold_visual(VISUAL_LOCK_TIMEOUT)?,
        );
    }
    if !folder.exists() {
        return Ok(());
    }
    let parent = folder
        .parent()
        .context("card cache folder has no parent directory")?;
    let tomb = Builder::new()
        .prefix(".kamishibai-purge-")
        .tempdir_in(parent)?;
    let discarded = tomb.path().to_path_buf();
    tomb.close()?;
    fs::rename(&folder, &discarded)?;
    let moved = Cache::new(
        discarded
            .file_name()
            .and_then(|name| name.to_str())
            .context("purged cache folder name is not UTF-8")?,
        parent,
    );
    let unseen = visual_revisions(discarded.as_path())?
        .difference(&revisions)
        .cloned()
        .collect::<Vec<_>>();
    for revision in unseen {
        guards.push(
            moved
                .visual(revision.as_str())?
                .hold_visual(VISUAL_LOCK_TIMEOUT)?,
        );
    }
    drop(guards);
    fs::remove_dir_all(discarded)?;
    Ok(())
}

/// Return valid visual revision directory names in deterministic lock order.
fn visual_revisions(folder: &Path) -> Result<BTreeSet<String>> {
    let visual = folder.join(VISUAL_DIRECTORY);
    if !visual.exists() {
        return Ok(BTreeSet::new());
    }
    let mut revisions = BTreeSet::new();
    for entry in fs::read_dir(visual)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(revision) = name.to_str() else {
            continue;
        };
        if entry.file_type()?.is_dir()
            && revision.len() == 64
            && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            revisions.insert(revision.to_string());
        }
    }
    Ok(revisions)
}
