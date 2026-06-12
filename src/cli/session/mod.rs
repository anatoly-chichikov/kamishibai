//! Persistent asynchronous generation sessions — the non-interactive flow.
//!
//! A session is a directory under the cache that an agent drives across separate
//! invocations: `new` (understand the `--word`s) → curate the understanding with
//! `select`/`exclude`/`correct` → `generate` (a managed background worker
//! generates and publishes) → `status`/`result`, with `regenerate` to push
//! corrections. Output is plain text only — never JSON. The one machine-relevant
//! value of each command (a session id, a path) is printed bare on stdout so it
//! is captured with `$(...)`; everything else goes to stderr.
//!
//! This module is the thin router plus the preconditions every verb shares
//! (`open_checked`, `refuse_if_live`, `preflight_key`, `reset_to_understood`,
//! `drop_artifacts`). The verbs themselves live one per concern: `new` (create
//! and open), `generate` (generate/regenerate), `result` (status/result/ls),
//! `maintenance` (cancel/rm/cache-path), and `curate` (select/exclude/correct).
//! The clap grammar lives in `args`; `cli.rs` only routes a parsed `Command`.

mod args;
mod bridge;
mod curate;
mod generate;
mod liveness;
mod maintenance;
mod new;
mod result;
mod store;
mod view;
mod worker;

pub(super) use args::Command;
pub(super) use bridge::TuiSession;

use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::config::default_store;
use crate::generation::artifact_cache::{ILLUSTRATION_FILE, META_FILE, SCENE_FILE, VOICE_FILE};
use crate::runtime::locations::SystemContext;
use crate::session::{CardCell, LanguagePair};

use super::error::{not_found, usage};

use store::{Phase, SessionRecord, SessionStore};

/// Route one parsed command to its handler; refusals carry their exit code.
pub(super) fn handle(command: &Command) -> Result<()> {
    match command {
        Command::New(args) => new::new(args),
        Command::Open(args) => new::open(args),
        Command::Select(args) => curate::select(args),
        Command::Exclude(args) => curate::exclude(args),
        Command::Correct(args) => curate::correct(args),
        Command::Generate(args) => generate::generate(args),
        Command::Status(args) => result::status(args),
        Command::Regenerate(args) => generate::regenerate(args),
        Command::Result(args) => result::result(args),
        Command::Cancel(args) => maintenance::cancel(args),
        Command::Ls(args) => result::ls(args),
        Command::Rm(args) => maintenance::rm(args),
        Command::CachePath => maintenance::cache_path(),
        Command::Worker(args) => worker::run_detached_entry(args.id.as_str()),
    }
}

/// Open one session, refusing with the not-found exit code (3) when it is
/// absent; a present-but-corrupt session stays an operational failure (1).
pub(in crate::cli::session) fn open_checked(
    store: &SessionStore,
    id: &str,
) -> Result<SessionRecord> {
    if !store.exists(id) {
        return Err(not_found(format!("no session '{id}'")));
    }
    store.open(id)
}

/// Refuse a mutating verb while a worker is provably alive (holds the lock) —
/// even one that has not yet recorded its pid in the session file.
pub(in crate::cli::session) fn refuse_if_live(
    store: &SessionStore,
    record: &SessionRecord,
) -> Result<()> {
    if !liveness::is_held(&store.lock_path(&record.id)) {
        return Ok(());
    }
    Err(usage(match &record.worker {
        Some(worker) => format!(
            "session '{}' has a running worker (pid {}); cancel it first",
            record.id, worker.pid
        ),
        None => format!(
            "session '{}' has a running worker; cancel it first",
            record.id
        ),
    }))
}

/// Fail fast when no Gemini API key is reachable, before any flow that needs one.
pub(in crate::cli::session) fn preflight_key() -> Result<()> {
    let saved = default_store(&SystemContext)
        .ok()
        .and_then(|store| store.read().ok())
        .and_then(|prefs| prefs.api_key)
        .filter(|key| !key.is_empty());
    let env = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());
    if env.is_none() && saved.is_none() {
        return Err(usage(
            "no Gemini API key found in GEMINI_API_KEY or saved preferences; set GEMINI_API_KEY",
        ));
    }
    Ok(())
}

/// Reset a session to understood, clearing any worker/result/error/progress so
/// the next `generate` re-derives and re-runs the plan from the curation.
pub(in crate::cli::session) fn reset_to_understood(record: &mut SessionRecord) {
    record.phase = Phase::Understood;
    record.result = None;
    record.error = None;
    record.progress = None;
    record.worker = None;
}

/// Delete one card's cached media (and its meta unless `keep_meta`), forcing just
/// that card to regenerate on the next run.
pub(in crate::cli::session) fn drop_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
    keep_meta: bool,
) -> Result<()> {
    let cache = CardCell::new(root.to_path_buf(), pair, term, understanding).cache();
    let folder = cache.path();
    for file in [VOICE_FILE, SCENE_FILE, ILLUSTRATION_FILE] {
        let path = folder.join(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    if !keep_meta {
        let path = folder.join(META_FILE);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}
