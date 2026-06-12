//! The lifecycle-maintenance verbs: `cancel` (stop a running worker), `rm`
//! (delete a session, optionally its cached cards), and `cache-path`.

use anyhow::Result;

use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::LanguagePair;

use super::args::{IdArg, RmArgs};
use super::liveness;
use super::store::{Phase, SessionRecord, SessionStore};
use super::{drop_artifacts, open_checked, refuse_if_live};

/// Stop a session's running worker and mark it cancelled unless already terminal.
///
/// The pid is signalled only while the liveness lock proves a live worker owns
/// it, so a stale (possibly reused) pid is never sent a signal; the brief window
/// between that probe and the signal is accepted as residual. The final state is
/// written as one serialized update, so cancel never fails on a concurrency
/// race: a worker that finished first stays published, anything else settles
/// cancelled.
pub(super) fn cancel(args: &IdArg) -> Result<()> {
    let store = SessionStore::system()?;
    let opened = open_checked(&store, args.id.as_str())?;
    if let Some(worker) = &opened.worker
        && liveness::is_held(&store.lock_path(args.id.as_str()))
    {
        liveness::terminate(worker.pid);
    }
    store.update(args.id.as_str(), |record| {
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
    eprintln!("cancelled session {}", args.id);
    Ok(())
}

/// Delete a session, and with `--cache` its cached card folders too.
pub(super) fn rm(args: &RmArgs) -> Result<()> {
    let store = SessionStore::system()?;
    let record = open_checked(&store, args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    if args.cache {
        let root = cache_root(&SystemContext)?;
        let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
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
    store.remove(args.id.as_str())?;
    eprintln!("removed session {}", args.id);
    Ok(())
}

/// Print the cache directory and exit.
pub(super) fn cache_path() -> Result<()> {
    println!("{}", cache_root(&SystemContext)?.display());
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
