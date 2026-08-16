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
//!
//! Destructive card-cache work takes leases in one fixed order: meta, voice,
//! then visual. Gemini workflows hold only one of those leases at a time, so a
//! cleanup can wait for active work without introducing a nested lock cycle.

mod adjust;
mod args;
mod config;
mod cost_journal;
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
pub(in crate::cli) use cost_journal::SessionCostScope;
pub(in crate::cli) use store::{
    DraftRecord, Phase, ResultRecord, SessionRecord, SessionStore, WorkerHandle, mint_id, now,
};

use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::config::default_store;
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, LEGACY_VISUAL_REVISION_FILE, META_FILE, ROOT_STAGE_LOCK_TIMEOUT,
    RootStage, RootStageGuard, SCENE_FILE, VISUAL_LOCK_TIMEOUT, VOICE_FILE, VisualGuard,
};
use crate::generation::invalidate_card;
use crate::generation::restart_picture_request_series;
use crate::generation::visual_revision;
use crate::languages::{LanguageCode, catalog};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardCell, CardMetaCache, LanguagePair};

use super::error::{self, usage, usage_hint};

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
        Command::AgentContract => super::contract::print(),
        Command::New(args) => new::new(args, render),
        Command::Open(args) => open(args, render, opener),
        Command::Select(args) => curate::select(args, render),
        Command::Exclude(args) => curate::exclude(args, render),
        Command::Correct(args) => curate::correct(args, render),
        Command::Generate(args) => generate::generate(args, render),
        Command::Status(args) => result::status(args, render),
        Command::Adjust(args) => adjust::adjust(args, render),
        Command::Regenerate(args) => generate::regenerate(args, render),
        Command::Result(args) => result::result(args, render),
        Command::Cancel(args) => maintenance::cancel(args, render),
        Command::Ls(args) => result::ls(args, render),
        Command::Rm(args) => maintenance::rm(args, render),
        Command::CachePath => maintenance::cache_path(render),
        Command::Config(args) => config::config(args, render),
        Command::Worker(args) => worker::run_detached_entry(args.id.as_str()),
    }
}

/// Refuse the one `--json` grammar conflict left: the interactive `open`, which
/// takes over the terminal and has no document to print.
fn refuse_json_conflicts(command: &Command, render: Render) -> Result<()> {
    if matches!(render, Render::Json) && matches!(command, Command::Open(_)) {
        return Err(usage("open is interactive; --json does not apply"));
    }
    Ok(())
}

/// Check the session exists and is not being generated, then resume it through
/// the caller's interactive surface.
fn open(args: &args::IdArg, render: Render, opener: &dyn SessionOpener) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
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
        return Err(error::not_found_hint(
            format!("no session \"{id}\""),
            "See what you have: kamishibai ls",
        ));
    }
    store.open(id)
}

/// Return whether a phase is settled — finished for good, so an omitted id
/// never resolves to it while an unfinished session exists. `Interrupted` is
/// deliberately NOT settled (unlike in `view::terminal`): it is an unfinished
/// run awaiting a resume.
fn settled(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Published | Phase::Partial | Phase::Failed | Phase::Cancelled
    )
}

/// The outcome of resolving an omitted session id against the store.
enum Picked {
    /// No sessions exist at all.
    None,
    /// Exactly one session answers the cascade.
    One(Box<SessionRecord>),
    /// Several candidates; the caller lists them instead of acting.
    Ambiguous(Vec<SessionRecord>),
}

/// Decide which session an omitted id means: the only session (settled or
/// not), else the only unfinished one, else ambiguous.
fn pick(records: Vec<SessionRecord>, cache_root: &Path) -> Picked {
    match records.len() {
        0 => Picked::None,
        1 => Picked::One(Box::new(
            records.into_iter().next().expect("invariant: one record"),
        )),
        _ => {
            let mut unsettled = records
                .iter()
                .filter(|record| !settled(view::live_phase(record, cache_root).0));
            match (unsettled.next(), unsettled.next()) {
                (Some(only), None) => Picked::One(Box::new(only.clone())),
                _ => Picked::Ambiguous(records),
            }
        }
    }
}

/// Resolve the session a command acts on. An explicit id behaves exactly as
/// before (absent → exit 3). An omitted id runs the cascade; on ambiguity the
/// text render prints the newest five sessions as ls lines on stdout, and the
/// refusal (exit 5) carries the same five as `ls --json` items for the JSON
/// envelope.
fn resolve(store: &SessionStore, explicit: Option<&str>, render: Render) -> Result<SessionRecord> {
    if let Some(id) = explicit {
        return open_checked(store, id);
    }
    let root = cache_root(&SystemContext)?;
    let _ = render;
    match pick(store.list()?, root.as_path()) {
        Picked::One(record) => Ok(*record),
        Picked::None => Err(error::not_found_hint(
            "no sessions yet",
            "Create one: kamishibai new --word <WORD>",
        )),
        Picked::Ambiguous(records) => {
            let total = records.len();
            let unfinished = records
                .iter()
                .filter(|record| !settled(view::live_phase(record, root.as_path()).0))
                .count();
            let newest: Vec<SessionRecord> = records.into_iter().rev().take(5).collect();
            let mut listing: Vec<String> = newest
                .iter()
                .map(|record| view::summary_line(record, root.as_path()))
                .collect();
            if total > newest.len() {
                listing.push(format!(
                    "…and {} more — kamishibai ls",
                    total - newest.len()
                ));
            }
            let noun = if unfinished >= 2 {
                format!("{unfinished} unfinished sessions")
            } else {
                format!("{total} sessions")
            };
            let sessions = serde_json::to_value(json::ls_items(&newest, root.as_path()))?;
            Err(error::ambiguous(
                format!("{noun} — pass an id (see kamishibai ls)"),
                listing.join("\n"),
                sessions,
            ))
        }
    }
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
    let env = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());
    if env.is_some() {
        return Ok(());
    }
    let saved = default_store(&SystemContext)?
        .read()?
        .api_key
        .filter(|key| !key.trim().is_empty());
    if saved.is_none() {
        return Err(usage_hint(
            "no Gemini API key found",
            "Set GEMINI_API_KEY for this command or run: kamishibai config --key - --json",
        ));
    }
    Ok(())
}

/// Resolve a supported language or refuse with every accepted canonical code.
pub(in crate::cli::session) fn resolve_language(code: &str) -> Result<LanguageCode> {
    let languages = catalog();
    languages.resolve(code).map_err(|_| {
        let supported = languages.codes().map(str::to_uppercase).join(", ");
        usage_hint(
            format!("unknown language '{code}'"),
            format!("Supported languages: {supported}"),
        )
    })
}

/// Refuse an unknown language code before it is used or persisted.
pub(in crate::cli::session) fn validate_language(code: &str) -> Result<()> {
    resolve_language(code).map(|_| ())
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
    invalidate_card(root, pair, term, understanding, keep_meta, keep_meta)
}

/// Delete one corrected card's stale artifacts while retaining its billed correction cost.
#[cfg(test)]
pub(in crate::cli::session) fn drop_corrected_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> Result<()> {
    invalidate_card(root, pair, term, understanding, false, true)
}

/// Delete only missing stages and their dependants, retaining every valid
/// upstream or independent artifact for a failed-card retry.
pub(in crate::cli) fn drop_incomplete_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> Result<()> {
    let cache = CardCell::new(root.to_path_buf(), pair, term, understanding).cache();
    let visual = cache.visual(visual_revision())?;
    let _guards = hold_artifacts(&cache, &visual)?;
    restart_picture_request_series(&visual)?;
    if !cached_meta_is_valid(root, pair, term, understanding) {
        remove_cached_files(&visual, &[SCENE_FILE, ILLUSTRATION_FILE])?;
        remove_cached_files(
            &cache,
            &[
                META_FILE,
                VOICE_FILE,
                SCENE_FILE,
                ILLUSTRATION_FILE,
                LEGACY_VISUAL_REVISION_FILE,
            ],
        )?;
        return Ok(());
    }
    if !cache.exists(VOICE_FILE) {
        remove_cached_files(&cache, &[VOICE_FILE])?;
    }
    if !cached_scene_is_valid(&visual) {
        remove_cached_files(&visual, &[SCENE_FILE, ILLUSTRATION_FILE])?;
    } else if !visual.exists(ILLUSTRATION_FILE) {
        remove_cached_files(&visual, &[ILLUSTRATION_FILE])?;
    }
    Ok(())
}

struct ArtifactGuards {
    _meta: RootStageGuard,
    _voice: RootStageGuard,
    _visual: VisualGuard,
}

/// Acquire every card artifact lease in the module-level destructive order.
fn hold_artifacts(cache: &Cache, visual: &Cache) -> Result<ArtifactGuards> {
    let meta = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
    let voice = cache.hold_root_stage(RootStage::Voice, ROOT_STAGE_LOCK_TIMEOUT)?;
    let visual = visual.hold_visual(VISUAL_LOCK_TIMEOUT)?;
    Ok(ArtifactGuards {
        _meta: meta,
        _voice: voice,
        _visual: visual,
    })
}

/// Return whether the cached card metadata can be decoded for this exact card.
pub(in crate::cli::session) fn cached_meta_is_valid(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> bool {
    matches!(
        CardMetaCache::new(root).load(term, understanding, pair),
        Ok(Some(_))
    )
}

/// Return whether the cached scene satisfies the minimum production structure.
pub(in crate::cli::session) fn cached_scene_is_valid(cache: &Cache) -> bool {
    fs::read(cache.path().join(SCENE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|scene| crate::gemini::validate_cached_scene(&scene).is_ok())
}

/// Remove named files from one cache without treating absence as an error.
fn remove_cached_files(cache: &Cache, files: &[&str]) -> Result<()> {
    for file in files {
        let path = cache.path().join(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command as ProcessCommand, Stdio};
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::store::WorkerHandle;
    use super::*;
    use crate::generation::artifact_cache::{
        ILLUSTRATION_COST_FILE, IMAGE_ATTEMPTS_DIRECTORY, META_COST_FILE, PICTURE_REQUESTS_FILE,
        SCENE_ATTEMPT_FILE, SCENE_COST_FILE, VOICE_COST_FILE,
    };
    use crate::generation::reserve_picture_request;
    use crate::session::{CardMeta, CardMetaCache};

    fn record(id: &str, phase: Phase) -> SessionRecord {
        let mut record = SessionRecord::understood(
            String::from(id),
            format!("2026-06-0{}T00:00:00Z", id.len().min(9)),
            String::from("en"),
            String::from("fr"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard")],
            Vec::new(),
        );
        record.phase = phase;
        record
    }

    #[test]
    fn a_lone_session_resolves_even_when_settled() {
        let home = TempDir::new().expect("tempdir must be created");
        let picked = pick(vec![record("a", Phase::Published)], home.path());
        assert!(
            matches!(picked, Picked::One(one) if one.id == "a"),
            "a lone session must resolve even when its phase is settled"
        );
    }

    #[test]
    fn the_single_unsettled_session_wins_over_settled_ones() {
        let home = TempDir::new().expect("tempdir must be created");
        let picked = pick(
            vec![
                record("done", Phase::Published),
                record("work", Phase::Understood),
                record("gone", Phase::Cancelled),
            ],
            home.path(),
        );
        assert!(
            matches!(picked, Picked::One(one) if one.id == "work"),
            "the single unfinished session must win resolution over settled ones"
        );
    }

    #[test]
    fn an_interrupted_session_counts_as_unsettled_for_resolution() {
        let home = TempDir::new().expect("tempdir must be created");
        let mut interrupted = record("broke", Phase::Generating);
        interrupted.worker = Some(WorkerHandle {
            pid: 999_999,
            started: String::from("t"),
        });
        let picked = pick(
            vec![record("done", Phase::Published), interrupted],
            home.path(),
        );
        assert!(
            matches!(picked, Picked::One(one) if one.id == "broke"),
            "an interrupted session must count as unfinished and win resolution"
        );
    }

    #[test]
    fn two_unfinished_sessions_are_ambiguous() {
        let home = TempDir::new().expect("tempdir must be created");
        let picked = pick(
            vec![
                record("a", Phase::Understood),
                record("b", Phase::Understood),
            ],
            home.path(),
        );
        assert!(
            matches!(picked, Picked::Ambiguous(both) if both.len() == 2),
            "two unfinished sessions must resolve as ambiguous, never silently pick one"
        );
    }

    #[test]
    fn dropped_artifacts_forget_request_costs_and_picture_counter() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path().to_path_buf(), &pair, "canard", "a duck").cache();
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        let sibling = cache
            .visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("sibling revision must be valid");
        for file in [
            META_FILE,
            META_COST_FILE,
            VOICE_FILE,
            VOICE_COST_FILE,
            SCENE_FILE,
            SCENE_COST_FILE,
            ILLUSTRATION_FILE,
            ILLUSTRATION_COST_FILE,
            LEGACY_VISUAL_REVISION_FILE,
        ] {
            fs::write(cache.filepath(file).expect("cache path must resolve"), b"x")
                .expect("cache fixture must be written");
        }
        for file in [
            SCENE_FILE,
            SCENE_ATTEMPT_FILE,
            SCENE_COST_FILE,
            ILLUSTRATION_FILE,
            ILLUSTRATION_COST_FILE,
            PICTURE_REQUESTS_FILE,
        ] {
            fs::write(
                visual.filepath(file).expect("visual path must resolve"),
                b"x",
            )
            .expect("current visual fixture must be written");
            fs::write(
                sibling.filepath(file).expect("sibling path must resolve"),
                b"x",
            )
            .expect("sibling visual fixture must be written");
        }
        fs::create_dir_all(visual.path().join(IMAGE_ATTEMPTS_DIRECTORY))
            .expect("attempt archive must be created");
        fs::write(
            visual
                .path()
                .join(IMAGE_ATTEMPTS_DIRECTORY)
                .join("attempt-0001.json"),
            b"{}",
        )
        .expect("attempt verdict must be written");
        drop_artifacts(home.path(), &pair, "canard", "a duck", false)
            .expect("artifacts must be dropped");
        assert_eq!(
            (
                [
                    META_FILE,
                    META_COST_FILE,
                    VOICE_FILE,
                    VOICE_COST_FILE,
                    SCENE_FILE,
                    SCENE_COST_FILE,
                    ILLUSTRATION_FILE,
                    ILLUSTRATION_COST_FILE,
                    PICTURE_REQUESTS_FILE,
                    LEGACY_VISUAL_REVISION_FILE,
                ]
                .iter()
                .any(|file| cache.path().join(file).exists()),
                [
                    SCENE_FILE,
                    SCENE_ATTEMPT_FILE,
                    SCENE_COST_FILE,
                    ILLUSTRATION_FILE,
                    ILLUSTRATION_COST_FILE,
                    PICTURE_REQUESTS_FILE,
                ]
                .iter()
                .any(|file| visual.path().join(file).exists()),
                [
                    SCENE_FILE,
                    SCENE_ATTEMPT_FILE,
                    SCENE_COST_FILE,
                    ILLUSTRATION_FILE,
                    ILLUSTRATION_COST_FILE,
                    PICTURE_REQUESTS_FILE,
                ]
                .iter()
                .all(|file| sibling.path().join(file).exists()),
                visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
            ),
            (false, false, true, false),
            "regeneration must drop current and legacy artifacts without touching sibling revisions"
        );
    }

    #[test]
    fn corrected_artifacts_keep_billed_meta_cost_while_dropping_stale_meta() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path().to_path_buf(), &pair, "canard", "a duck").cache();
        fs::write(
            cache.filepath(META_FILE).expect("meta path must resolve"),
            b"stale meta",
        )
        .expect("stale meta must be written");
        fs::write(
            cache
                .filepath(META_COST_FILE)
                .expect("meta cost path must resolve"),
            b"billed correction",
        )
        .expect("correction cost must be written");
        drop_corrected_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("corrected artifacts must be dropped");
        assert_eq!(
            (cache.exists(META_FILE), cache.exists(META_COST_FILE)),
            (false, true),
            "correction cleanup deleted its billed cost or retained stale metadata"
        );
    }

    fn fixture_meta() -> CardMeta {
        CardMeta::new(
            "/ka.naʁ/",
            "/lə ka.naʁ naʒ/",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A common concrete noun",
            "Le canard nage",
        )
    }

    fn seed_current_artifacts(
        home: &TempDir,
        pair: &LanguagePair,
        cache: &crate::generation::artifact_cache::Cache,
    ) {
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        CardMetaCache::new(home.path())
            .store("canard", "a duck", pair, &fixture_meta())
            .expect("valid meta fixture must be stored");
        for file in [META_COST_FILE, VOICE_FILE, VOICE_COST_FILE] {
            fs::write(cache.filepath(file).expect("cache path must resolve"), b"x")
                .expect("cache fixture must be written");
        }
        fs::write(
            visual
                .filepath(SCENE_FILE)
                .expect("visual path must resolve"),
            include_bytes!("../../../tests/fixtures/production-scene.json"),
        )
        .expect("valid scene fixture must be written");
        for file in [SCENE_COST_FILE, ILLUSTRATION_FILE, ILLUSTRATION_COST_FILE] {
            fs::write(
                visual.filepath(file).expect("visual path must resolve"),
                b"x",
            )
            .expect("visual fixture must be written");
        }
        seed_picture_counter(&visual, 3, 3);
        fs::write(
            visual
                .filepath(SCENE_ATTEMPT_FILE)
                .expect("cursor path must resolve"),
            br#"{"scene_attempt_index":0}"#,
        )
        .expect("scene cursor must be written");
    }

    fn seed_picture_counter(cache: &Cache, requests: u32, series_requests: u32) {
        fs::write(
            cache
                .filepath(PICTURE_REQUESTS_FILE)
                .expect("picture counter path must resolve"),
            serde_json::to_vec(&serde_json::json!({
                "schema": "kamishibai.picture-request-counter",
                "version": 1,
                "requests": requests,
                "series_requests": series_requests
            }))
            .expect("picture counter must encode"),
        )
        .expect("picture counter must be written");
    }

    fn picture_counter(cache: &Cache) -> (u64, u64) {
        let counter = serde_json::from_slice::<serde_json::Value>(
            fs::read(cache.path().join(PICTURE_REQUESTS_FILE))
                .expect("picture counter must be readable")
                .as_slice(),
        )
        .expect("picture counter must decode");
        (
            counter["requests"]
                .as_u64()
                .expect("total picture requests must be an integer"),
            counter["series_requests"]
                .as_u64()
                .expect("series picture requests must be an integer"),
        )
    }

    fn current_cache(home: &TempDir) -> (LanguagePair, crate::generation::artifact_cache::Cache) {
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path().to_path_buf(), &pair, "canard", "a duck").cache();
        seed_current_artifacts(home, &pair, &cache);
        (pair, cache)
    }

    fn destructive_child(root: &Path, mode: &str) -> Child {
        ProcessCommand::new(std::env::current_exe().expect("test binary must resolve"))
            .args([
                "cli::session::tests::destructive_cleanup_waits_for_root_work_before_deleting",
                "--exact",
                "--nocapture",
            ])
            .env("KAMISHIBAI_DESTRUCTIVE_LOCK_MODE", mode)
            .env("KAMISHIBAI_DESTRUCTIVE_LOCK_ROOT", root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("lock test child must spawn")
    }

    fn finish_child(child: &mut Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().ok().flatten() {
                return status.success();
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn destructive_cleanup_waits_for_root_work_before_deleting() {
        let pair = LanguagePair::new("fr", "en");
        if let Ok(mode) = std::env::var("KAMISHIBAI_DESTRUCTIVE_LOCK_MODE") {
            let root = std::env::var_os("KAMISHIBAI_DESTRUCTIVE_LOCK_ROOT")
                .map(std::path::PathBuf::from)
                .expect("lock test root must be set");
            let cache = CardCell::new(root.clone(), &pair, "canard", "a duck").cache();
            let succeeded = match mode.as_str() {
                "purge" => drop_artifacts(root.as_path(), &pair, "canard", "a duck", false).is_ok(),
                "probe" => cache
                    .hold_root_stage(RootStage::Meta, Duration::ZERO)
                    .is_err(),
                "producer" => (|| -> Result<()> {
                    let _guard = cache.hold_root_stage(RootStage::Meta, Duration::from_secs(5))?;
                    fs::write(cache.filepath(META_FILE)?, b"committed-after-cleanup")?;
                    Ok(())
                })()
                .is_ok(),
                _ => false,
            };
            assert!(
                succeeded,
                "the destructive lock child did not complete its assigned operation"
            );
            return;
        }
        let home = TempDir::new().expect("tempdir must be created");
        let cache = CardCell::new(home.path().to_path_buf(), &pair, "canard", "a duck").cache();
        fs::write(
            cache.filepath(VOICE_FILE).expect("voice path must resolve"),
            b"old",
        )
        .expect("old voice must be seeded");
        let voice = cache
            .hold_root_stage(RootStage::Voice, Duration::ZERO)
            .expect("voice producer lease must be acquired");
        let mut purge = destructive_child(home.path(), "purge");
        let deadline = Instant::now() + Duration::from_secs(5);
        let meta_held = loop {
            let mut probe = destructive_child(home.path(), "probe");
            if finish_child(&mut probe, Duration::from_secs(2)) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
        };
        let purge_waited = purge
            .try_wait()
            .expect("purge child must remain observable")
            .is_none();
        let mut producer = destructive_child(home.path(), "producer");
        std::thread::sleep(Duration::from_millis(100));
        let producer_waited = producer
            .try_wait()
            .expect("producer child must remain observable")
            .is_none();
        drop(voice);
        let purge_succeeded = finish_child(&mut purge, Duration::from_secs(5));
        let producer_succeeded = finish_child(&mut producer, Duration::from_secs(5));
        assert_eq!(
            (
                meta_held,
                purge_waited,
                producer_waited,
                purge_succeeded,
                producer_succeeded,
                fs::read(cache.path().join(META_FILE)).ok(),
                cache.exists(VOICE_FILE),
            ),
            (
                true,
                true,
                true,
                true,
                true,
                Some(b"committed-after-cleanup".to_vec()),
                false,
            ),
            "cleanup did not wait for active work or deleted a later producer commit"
        );
    }

    fn seed_attempt_history(cache: &Cache) {
        let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
        fs::create_dir_all(attempts.as_path()).expect("attempt history must be created");
        fs::write(attempts.join("attempt-0001.json"), b"{}")
            .expect("attempt history must be written");
        fs::write(
            attempts.join("attempt-0001.scene.json"),
            br#"{"manga_panel":{"meta":{"layout_selection":{"scene_attempt_index":1}}}}"#,
        )
        .expect("attempt scene must be written");
    }

    #[test]
    fn failed_picture_retry_preserves_accumulated_picture_cost_and_counter() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        seed_attempt_history(&visual);
        fs::remove_file(visual.path().join(ILLUSTRATION_FILE))
            .expect("picture fixture must be removed");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        let next_series = (0..4).all(|_| reserve_picture_request(&visual).is_ok());
        let beyond = reserve_picture_request(&visual);
        assert_eq!(
            (
                cache.exists(META_FILE),
                cache.exists(VOICE_FILE),
                visual.exists(SCENE_FILE),
                visual.exists(ILLUSTRATION_COST_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
                visual.exists(SCENE_ATTEMPT_FILE),
                visual
                    .path()
                    .join(IMAGE_ATTEMPTS_DIRECTORY)
                    .join("attempt-0001.scene.json")
                    .exists(),
                next_series,
                beyond.is_err(),
                picture_counter(&visual),
            ),
            (true, true, true, true, true, true, true, true, true, (7, 4)),
            "failed-only retry erased evidence or failed to start exactly one authorized series"
        );
    }

    #[test]
    fn failed_scene_retry_drops_scene_and_picture_only() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        seed_attempt_history(&visual);
        fs::remove_file(visual.path().join(SCENE_FILE)).expect("scene fixture must be removed");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        assert_eq!(
            (
                cache.exists(META_FILE),
                cache.exists(VOICE_FILE),
                visual.exists(SCENE_COST_FILE),
                visual.exists(ILLUSTRATION_FILE),
                visual.exists(ILLUSTRATION_COST_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
                visual.exists(SCENE_ATTEMPT_FILE),
                visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
            ),
            (true, true, true, false, true, true, true, true),
            "scene retry erased billed spend, its cursor, or a valid independent artifact"
        );
    }

    #[test]
    fn corrupt_scene_retry_drops_scene_and_picture_only() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::write(
            visual.path().join(SCENE_FILE),
            br#"{"manga_panel":{"panels":[{}]}}"#,
        )
        .expect("scene fixture must be corrupted");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        assert_eq!(
            (
                cache.exists(META_FILE),
                cache.exists(VOICE_FILE),
                visual.exists(SCENE_FILE),
                visual.exists(SCENE_COST_FILE),
                visual.exists(ILLUSTRATION_FILE),
                visual.exists(ILLUSTRATION_COST_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
                visual.exists(SCENE_ATTEMPT_FILE),
            ),
            (true, true, false, true, false, true, true, true),
            "a corrupt scene retry erased billed spend or its durable cursor"
        );
    }

    #[test]
    fn failed_sound_retry_keeps_valid_visuals() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::remove_file(cache.path().join(VOICE_FILE)).expect("sound fixture must be removed");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        assert_eq!(
            (
                cache.exists(META_FILE),
                cache.exists(VOICE_COST_FILE),
                visual.exists(SCENE_FILE),
                visual.exists(ILLUSTRATION_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
            ),
            (true, true, true, true, true),
            "sound retry erased billed spend or a valid visual artifact"
        );
    }

    #[test]
    fn failed_meta_retry_drops_every_dependent_artifact() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        seed_attempt_history(&visual);
        fs::remove_file(cache.path().join(META_FILE)).expect("meta fixture must be removed");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        assert_eq!(
            (
                cache.exists(META_COST_FILE),
                cache.exists(VOICE_FILE),
                cache.exists(VOICE_COST_FILE),
                visual.exists(SCENE_FILE),
                visual.exists(SCENE_COST_FILE),
                visual.exists(ILLUSTRATION_FILE),
                visual.exists(ILLUSTRATION_COST_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
                visual.exists(SCENE_ATTEMPT_FILE),
                visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
            ),
            (
                true, false, true, false, true, false, true, true, true, true
            ),
            "meta retry erased billed spend, its cursor, or its attempt history"
        );
    }

    #[test]
    fn corrupt_meta_retry_drops_every_dependent_artifact() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::write(cache.path().join(META_FILE), b"{}").expect("meta fixture must be corrupted");
        drop_incomplete_artifacts(home.path(), &pair, "canard", "a duck")
            .expect("incomplete artifacts must be dropped");
        assert_eq!(
            (
                cache.exists(META_FILE),
                cache.exists(META_COST_FILE),
                cache.exists(VOICE_FILE),
                cache.exists(VOICE_COST_FILE),
                visual.exists(SCENE_FILE),
                visual.exists(SCENE_COST_FILE),
                visual.exists(ILLUSTRATION_FILE),
                visual.exists(ILLUSTRATION_COST_FILE),
                visual.exists(PICTURE_REQUESTS_FILE),
                visual.exists(SCENE_ATTEMPT_FILE),
            ),
            (
                false, true, false, true, false, true, false, true, true, true
            ),
            "a corrupt meta retry erased billed spend or its durable cursor"
        );
    }

    #[test]
    fn imported_full_reroll_keeps_supplied_meta_and_resets_picture_series() {
        let home = TempDir::new().expect("tempdir must be created");
        let (pair, cache) = current_cache(&home);
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        seed_attempt_history(&visual);
        drop_artifacts(home.path(), &pair, "canard", "a duck", true)
            .expect("artifacts must be dropped");
        let cleared = !visual.exists(PICTURE_REQUESTS_FILE)
            && !visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists();
        let next_series = (0..4).all(|_| reserve_picture_request(&visual).is_ok());
        let beyond = reserve_picture_request(&visual);
        assert_eq!(
            (
                cleared,
                next_series,
                beyond.is_err(),
                picture_counter(&visual),
                [
                    cache.exists(META_FILE),
                    cache.exists(META_COST_FILE),
                    cache.exists(VOICE_FILE),
                    visual.exists(SCENE_FILE),
                    visual.exists(ILLUSTRATION_FILE),
                    visual.exists(SCENE_ATTEMPT_FILE),
                    visual.exists(SCENE_COST_FILE),
                    visual.exists(ILLUSTRATION_COST_FILE),
                    visual.exists(PICTURE_REQUESTS_FILE),
                ],
            ),
            (
                true,
                true,
                true,
                (4, 4),
                [true, true, false, false, false, false, false, false, true],
            ),
            "an imported full reroll did not clear evidence before opening one new picture series"
        );
    }
}
