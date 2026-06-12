//! Persistent asynchronous generation sessions — the non-interactive flow.
//!
//! A session is a directory under the cache that an agent drives across separate
//! invocations: `new` (understand the `--word`s) → curate the understanding with
//! `select`/`exclude`/`correct` → `generate` (a managed background worker
//! generates and publishes) → `status`/`result`, with `regenerate` to push
//! corrections. Output defaults to plain text — the one machine-relevant value
//! of each command (a session id, a path) printed bare on stdout so it is
//! captured with `$(...)`, everything else on stderr — and [`Render::Json`]
//! switches it to exactly one JSON document per invocation (see `json`).
//!
//! This module is the thin router plus the preconditions every verb shares
//! (`open_checked`, `refuse_if_live`, `preflight_key`, `reset_to_understood`,
//! `drop_artifacts`). The verbs themselves live one per concern: `new`,
//! `generate` (generate/regenerate), `result` (status/result/ls),
//! `maintenance` (cancel/rm/cache-path), and `curate` (select/exclude/correct).
//! The clap grammar lives in `args`; `cli.rs` only routes a parsed `Command`.
//! This layer never links the TUI: `open` hands the checked record to the
//! caller-supplied [`SessionOpener`] port, which the TUI side implements.

mod args;
mod curate;
mod generate;
mod json;
mod liveness;
mod maintenance;
mod new;
mod result;
mod store;
mod view;
mod worker;

pub(super) use args::Command;
pub(in crate::cli) use store::{
    DraftRecord, Phase, ResultRecord, SessionRecord, SessionStore, WorkerHandle, mint_id, now,
};

use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::config::default_store;
use crate::generation::artifact_cache::{ILLUSTRATION_FILE, META_FILE, SCENE_FILE, VOICE_FILE};
use crate::runtime::locations::SystemContext;
use crate::session::{CardCell, LanguagePair};

use super::error::{not_found, usage};

/// Port through which `open` hands a checked session to the interactive
/// surface; the TUI side implements it, so this layer never links the TUI.
pub(super) trait SessionOpener {
    /// Take over the terminal with this session resumed.
    fn open(&self, record: &SessionRecord) -> Result<()>;
}

/// How a session command renders its stdout: the plain-text contract, or one
/// JSON document per invocation. A representation choice only — exit codes,
/// locking, and session semantics are identical in both modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Render {
    /// Human-readable plain text (the default contract).
    Text,
    /// One machine-readable JSON document per invocation.
    Json,
}

/// Route one parsed command to its handler; refusals carry their exit code.
/// JSON-mode grammar conflicts are refused here, before any handler runs, so
/// the verbs never police the flag themselves.
pub(super) fn handle(command: &Command, render: Render, opener: &dyn SessionOpener) -> Result<()> {
    refuse_json_conflicts(command, render)?;
    match command {
        Command::New(args) => new::new(args, render),
        Command::Open(args) => open(args, opener),
        Command::Select(args) => curate::select(args, render),
        Command::Exclude(args) => curate::exclude(args, render),
        Command::Correct(args) => curate::correct(args, render),
        Command::Generate(args) => generate::generate(args, render),
        Command::Status(args) => result::status(args, render),
        Command::Regenerate(args) => generate::regenerate(args, render),
        Command::Result(args) => result::result(args, render),
        Command::Cancel(args) => maintenance::cancel(args, render),
        Command::Ls(args) => result::ls(args, render),
        Command::Rm(args) => maintenance::rm(args, render),
        Command::CachePath => maintenance::cache_path(render),
        Command::Worker(args) => worker::run_detached_entry(args.id.as_str()),
    }
}

/// Refuse `--json` combinations that would fight over stdout: `-q` (a plain
/// projection), the `result` path selectors (paths are document fields), and
/// the interactive `open`.
fn refuse_json_conflicts(command: &Command, render: Render) -> Result<()> {
    if matches!(render, Render::Text) {
        return Ok(());
    }
    let quiet = match command {
        Command::Open(_) => {
            return Err(usage("open is interactive; --json does not apply"));
        }
        Command::Result(args) if args.deck || args.pdf || args.dir => {
            return Err(usage(
                "--json carries the paths as fields; drop --deck/--pdf/--dir",
            ));
        }
        Command::New(args) => args.quiet,
        Command::Generate(args) => args.quiet,
        Command::Status(args) => args.quiet,
        Command::Result(args) => args.quiet,
        Command::Ls(args) => args.quiet,
        _ => false,
    };
    if quiet {
        return Err(usage(
            "--json and -q are mutually exclusive; the JSON document is already capturable",
        ));
    }
    Ok(())
}

/// Check the session exists and is not being generated, then resume it through
/// the caller's interactive surface.
fn open(args: &args::IdArg, opener: &dyn SessionOpener) -> Result<()> {
    let store = SessionStore::system()?;
    let record = open_checked(&store, args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    opener.open(&record)
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
