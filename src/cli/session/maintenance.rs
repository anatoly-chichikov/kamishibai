//! The lifecycle-maintenance verbs: `cancel` (stop a running worker), `rm`
//! (delete a session, optionally its cached cards), and `cache-path`.

use anyhow::Result;

use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::LanguagePair;

use super::args::{IdArg, RmArgs};
use super::liveness;
use super::store::{Phase, SessionRecord, SessionStore};
use super::{Render, drop_artifacts, json, refuse_if_live, resolve, view};

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
            drop_artifacts(
                root.as_path(),
                &pair,
                term.as_str(),
                understanding.as_str(),
                false,
            )?;
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
