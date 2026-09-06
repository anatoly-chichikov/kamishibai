//! The lifecycle-maintenance verbs: `cancel` (stop a running worker), `rm`
//! (delete a session, optionally its cached cards), and `cache-path`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tempfile::Builder;

use crate::generation::artifact_cache::{
    ROOT_STAGE_LOCK_TIMEOUT, RootStage, VISUAL_DIRECTORY, VISUAL_LOCK_TIMEOUT,
};
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
        for cell in cached_cells(root.as_path(), &pair, &record) {
            purge_cell(cell)?;
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

/// Return every cache cell a session may own: contextual committed drafts when
/// present, otherwise each candidate's legacy singleton sense.
fn cached_cells(root: &Path, pair: &LanguagePair, record: &SessionRecord) -> Vec<CardCell> {
    if !record.drafts.is_empty() {
        return record
            .drafts
            .iter()
            .map(|draft| super::cell_for_draft(root, pair, draft))
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
                    CardCell::new(
                        root.to_path_buf(),
                        pair,
                        candidate.term(),
                        sense.understanding(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Delete one card after leasing meta, voice, then every sorted visual revision.
#[cfg(test)]
fn purge_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> Result<()> {
    purge_cell(CardCell::new(root.to_path_buf(), pair, term, understanding))
}

fn purge_cell(cell: CardCell) -> Result<()> {
    let cache = cell.cache();
    let folder = cache.path();
    let meta = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
    let voice = cache.hold_root_stage(RootStage::Voice, ROOT_STAGE_LOCK_TIMEOUT)?;
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
    let unseen = visual_revisions(discarded.as_path())?
        .difference(&revisions)
        .cloned()
        .collect::<Vec<_>>();
    for revision in unseen {
        guards.push(
            cache
                .visual(revision.as_str())?
                .hold_visual(VISUAL_LOCK_TIMEOUT)?,
        );
    }
    drop(guards);
    drop(voice);
    drop(meta);
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

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::*;
    use crate::generation::artifact_cache::{META_FILE, VOICE_FILE};

    fn purge_child(root: &Path) -> Child {
        Command::new(std::env::current_exe().expect("test binary must resolve"))
            .args([
                "cli::session::maintenance::tests::cache_purge_waits_for_root_work_before_deleting",
                "--exact",
                "--nocapture",
            ])
            .env("KAMISHIBAI_CACHE_PURGE_TEST", "purge")
            .env("KAMISHIBAI_CACHE_PURGE_ROOT", root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cache purge child must spawn")
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

    fn race_child(root: &Path, mode: &str) -> Child {
        Command::new(std::env::current_exe().expect("test binary must resolve"))
            .args([
                "cli::session::maintenance::tests::cache_purge_keeps_one_lock_domain_across_rename",
                "--exact",
                "--nocapture",
            ])
            .env("KAMISHIBAI_CACHE_PURGE_RACE", mode)
            .env("KAMISHIBAI_CACHE_PURGE_ROOT", root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cache race child must spawn")
    }

    fn wait_for(path: &Path, child: &mut Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if path.exists() {
                return true;
            }
            if child.try_wait().ok().flatten().is_some() || Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn append(path: &Path, event: &[u8]) -> Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(event)?;
        Ok(())
    }

    fn try_hold_opened(file: &File) -> Result<bool> {
        #[cfg(unix)]
        let locked =
            match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => true,
                Err(error)
                    if error == rustix::io::Errno::WOULDBLOCK
                        || error == rustix::io::Errno::AGAIN =>
                {
                    false
                }
                Err(error) => return Err(error.into()),
            };
        #[cfg(not(unix))]
        let locked = match file.try_lock() {
            Ok(()) => true,
            Err(std::fs::TryLockError::WouldBlock) => false,
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        };
        Ok(locked)
    }

    fn hold_opened(file: &File, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if try_hold_opened(file)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("opened cache lock remained held past its test deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn run_race_child(root: &Path, mode: &str, pair: &LanguagePair) -> Result<()> {
        let cache = CardCell::new(root.to_path_buf(), pair, "canard", "a duck").cache();
        match mode {
            "purge" => purge_artifacts(root, pair, "canard", "a duck"),
            "waiting" => {
                let lock = cache.root_stage_lock_path(RootStage::Meta);
                fs::create_dir_all(lock.parent().context("test lock has no parent")?)?;
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(lock)?;
                fs::write(root.join("waiting-opened"), b"opened")?;
                hold_opened(&file, Duration::from_secs(5))?;
                fs::create_dir_all(cache.path())?;
                fs::write(root.join("waiting-entered"), b"entered")?;
                let release = root.join("release-waiting");
                let deadline = Instant::now() + Duration::from_secs(5);
                while !release.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !release.exists() {
                    anyhow::bail!("waiting producer was not released");
                }
                fs::write(cache.filepath(META_FILE)?, b"waiting")?;
                append(root.join("commits").as_path(), b"waiting\n")
            }
            "late" => {
                let lock = cache.root_stage_lock_path(RootStage::Meta);
                fs::create_dir_all(lock.parent().context("test lock has no parent")?)?;
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(lock)?;
                fs::write(root.join("late-opened"), b"opened")?;
                let allow = root.join("allow-late");
                let deadline = Instant::now() + Duration::from_secs(5);
                while !allow.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !allow.exists() {
                    anyhow::bail!("late producer was not allowed to try its lock");
                }
                if !try_hold_opened(&file)? {
                    fs::write(root.join("late-blocked"), b"blocked")?;
                    hold_opened(&file, Duration::from_secs(5))?;
                }
                fs::write(root.join("late-entered"), b"entered")?;
                fs::write(cache.filepath(META_FILE)?, b"late")?;
                append(root.join("commits").as_path(), b"late\n")
            }
            _ => anyhow::bail!("unknown cache purge race mode"),
        }
    }

    #[test]
    fn cache_purge_keeps_one_lock_domain_across_rename() {
        let pair = LanguagePair::new("fr", "en");
        if let Ok(mode) = std::env::var("KAMISHIBAI_CACHE_PURGE_RACE") {
            let root = std::env::var_os("KAMISHIBAI_CACHE_PURGE_ROOT")
                .map(std::path::PathBuf::from)
                .expect("cache purge root must be set");
            assert!(
                run_race_child(root.as_path(), mode.as_str(), &pair).is_ok(),
                "the cache purge race child did not finish its assigned operation"
            );
            return;
        }
        let home = TempDir::new().expect("tempdir must be created");
        let root = home.path();
        let cache = CardCell::new(root.to_path_buf(), &pair, "canard", "a duck").cache();
        fs::write(
            cache.filepath(VOICE_FILE).expect("voice path must resolve"),
            b"old",
        )
        .expect("old voice must be seeded");
        let voice = cache
            .hold_root_stage(RootStage::Voice, Duration::ZERO)
            .expect("voice producer lease must be acquired");
        let mut purge = race_child(root, "purge");
        let deadline = Instant::now() + Duration::from_secs(5);
        let purge_holds_meta = loop {
            match cache.hold_root_stage(RootStage::Meta, Duration::ZERO) {
                Ok(guard) => drop(guard),
                Err(_) => break true,
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut waiting = race_child(root, "waiting");
        let waiting_opened = wait_for(
            root.join("waiting-opened").as_path(),
            &mut waiting,
            Duration::from_secs(2),
        );
        drop(voice);
        let purge_succeeded = finish_child(&mut purge, Duration::from_secs(5));
        let waiting_entered = wait_for(
            root.join("waiting-entered").as_path(),
            &mut waiting,
            Duration::from_secs(2),
        );
        let mut late = race_child(root, "late");
        let late_opened = wait_for(
            root.join("late-opened").as_path(),
            &mut late,
            Duration::from_secs(2),
        );
        fs::write(root.join("allow-late"), b"allow")
            .expect("late producer permission must be written");
        let late_blocked = wait_for(
            root.join("late-blocked").as_path(),
            &mut late,
            Duration::from_secs(2),
        );
        fs::write(root.join("release-waiting"), b"release")
            .expect("waiting producer release must be written");
        let waiting_succeeded = finish_child(&mut waiting, Duration::from_secs(5));
        let late_succeeded = finish_child(&mut late, Duration::from_secs(5));
        assert_eq!(
            (
                purge_holds_meta,
                waiting_opened,
                purge_succeeded,
                waiting_entered,
                late_opened,
                late_blocked,
                waiting_succeeded,
                late_succeeded,
                fs::read_to_string(root.join("commits")).ok(),
                fs::read(cache.path().join(META_FILE)).ok(),
            ),
            (
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                Some(String::from("waiting\nlate\n")),
                Some(b"late".to_vec()),
            ),
            "cache purge split one card into concurrent old-path and recreated-path lock domains"
        );
    }

    #[test]
    fn cache_purge_waits_for_root_work_before_deleting() {
        let pair = LanguagePair::new("fr", "en");
        if std::env::var("KAMISHIBAI_CACHE_PURGE_TEST").as_deref() == Ok("purge") {
            let root = std::env::var_os("KAMISHIBAI_CACHE_PURGE_ROOT")
                .map(std::path::PathBuf::from)
                .expect("cache purge root must be set");
            assert!(
                purge_artifacts(root.as_path(), &pair, "canard", "a duck").is_ok(),
                "the cache purge child did not finish its assigned cleanup"
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
        let mut purge = purge_child(home.path());
        let deadline = Instant::now() + Duration::from_secs(5);
        let meta_held = loop {
            match cache.hold_root_stage(RootStage::Meta, Duration::ZERO) {
                Ok(guard) => drop(guard),
                Err(_) => break true,
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let purge_waited = purge
            .try_wait()
            .expect("cache purge child must remain observable")
            .is_none();
        let producer_root = home.path().to_path_buf();
        let producer_pair = pair.clone();
        let producer = std::thread::spawn(move || {
            let producer = CardCell::new(producer_root, &producer_pair, "canard", "a duck").cache();
            let Ok(_guard) = producer.hold_root_stage(RootStage::Meta, Duration::from_secs(5))
            else {
                return false;
            };
            producer
                .filepath(META_FILE)
                .and_then(|path| fs::write(path, b"committed-after-cleanup").map_err(Into::into))
                .is_ok()
        });
        std::thread::sleep(Duration::from_millis(100));
        let producer_waited = !producer.is_finished();
        drop(voice);
        let purge_succeeded = finish_child(&mut purge, Duration::from_secs(5));
        let producer_succeeded = producer.join().unwrap_or(false);
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
            "cache purge did not wait for active work or deleted a later producer commit"
        );
    }
}
