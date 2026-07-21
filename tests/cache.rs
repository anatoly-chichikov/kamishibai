//! Tests for the persistent media cache.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::Result;
use kamishibai::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, SCENE_FILE, VISUAL_DIRECTORY, VISUAL_LOCK_FILE,
};
use tempfile::TempDir;

/// Named cache paths stay absolute and nested under the cache name.
#[test]
fn named_cache_paths_stay_absolute_and_nested_under_the_cache_name() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("audio-en", directory.path());
    let path = cache.filepath("demo.wav")?;
    assert_eq!(
        (cache.root(), cache.path(), path),
        (
            PathBuf::from(directory.path()),
            directory.path().join("audio-en"),
            directory.path().join("audio-en").join("demo.wav")
        ),
        "named cache paths no longer stay absolute and nested under the cache name"
    );
    Ok(())
}

/// Staged cache files commit into the final filename.
#[test]
fn staged_cache_files_commit_into_the_final_filename() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("audio-en", directory.path());
    let staged = cache.stage(".wav")?;
    fs::write(&staged, b"demo")?;
    cache.commit(&staged, "final.wav")?;
    assert!(
        cache.exists("final.wav"),
        "staged cache files no longer commit into the final filename"
    );
    Ok(())
}

/// Visual revisions resolve to separate content-addressed directories.
#[test]
fn visual_revisions_resolve_to_separate_content_addressed_directories() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("cards/test", directory.path());
    let first = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let second = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    assert_eq!(
        (cache.visual(first)?.path(), cache.visual(second)?.path()),
        (
            cache.path().join(VISUAL_DIRECTORY).join(first),
            cache.path().join(VISUAL_DIRECTORY).join(second),
        ),
        "different visual revisions must not resolve to the same cache directory"
    );
    Ok(())
}

/// Visual revisions reject values that cannot be SHA-256 digests.
#[test]
fn visual_revisions_reject_non_sha_256_paths() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("cards/test", directory.path());
    assert!(
        cache.visual("short").is_err()
            && cache
                .visual("gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg")
                .is_err(),
        "visual revisions must not accept a short value or nonhexadecimal path"
    );
    Ok(())
}

/// One revision cannot overwrite a sibling revision's visual artifacts.
#[test]
fn visual_revision_artifacts_preserve_sibling_revisions() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("cards/test", directory.path());
    let first = cache.visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    let second =
        cache.visual("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")?;
    fs::write(first.filepath(SCENE_FILE)?, b"first")?;
    fs::write(second.filepath(ILLUSTRATION_FILE)?, b"second")?;
    assert_eq!(
        (
            fs::read(first.filepath(SCENE_FILE)?)?,
            fs::read(second.filepath(ILLUSTRATION_FILE)?)?,
            first.exists(ILLUSTRATION_FILE),
            second.exists(SCENE_FILE),
        ),
        (b"first".to_vec(), b"second".to_vec(), false, false),
        "one visual revision must not leak artifacts into a sibling revision"
    );
    Ok(())
}

/// A configurable producer lease creates the revision-local advisory lock.
#[test]
fn visual_revision_locks_use_the_configured_wait_bound() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("cards/test", directory.path());
    let visual =
        cache.visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    let _guard = visual.hold_visual(Duration::ZERO)?;
    assert!(
        visual.exists(VISUAL_LOCK_FILE),
        "a visual producer lease must not omit its revision-local lock file"
    );
    Ok(())
}

/// Child-process entry point that holds one real cross-process visual lease.
#[test]
fn visual_lock_child_holds_one_cross_process_lease() -> Result<()> {
    let Ok(root) = std::env::var("KAMISHIBAI_VISUAL_LOCK_TEST_ROOT") else {
        return Ok(());
    };
    let cache = Cache::new("cards/test", root);
    let visual =
        cache.visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    let _guard = visual.hold_visual(Duration::ZERO)?;
    fs::write(cache.root().join("lease-ready"), b"ready")?;
    sleep(Duration::from_millis(250));
    assert!(
        visual.exists(VISUAL_LOCK_FILE),
        "the child process lost its visual lease before the hold window ended"
    );
    Ok(())
}

/// A competing process times out, then acquires the lease after its owner exits.
#[test]
fn visual_revision_locks_serialize_competing_processes() -> Result<()> {
    let directory = TempDir::new()?;
    let signal = directory.path().join("lease-ready");
    let mut child = Command::new(std::env::current_exe()?)
        .arg("visual_lock_child_holds_one_cross_process_lease")
        .arg("--exact")
        .arg("--nocapture")
        .env("KAMISHIBAI_VISUAL_LOCK_TEST_ROOT", directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let ready = wait_for(&signal, &mut child, Duration::from_secs(2));
    let cache = Cache::new("cards/test", directory.path());
    let visual =
        cache.visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
    let blocked = visual.hold_visual(Duration::from_millis(30)).is_err();
    let exited = finish(&mut child, Duration::from_secs(2));
    let acquired = visual.hold_visual(Duration::ZERO).is_ok();
    assert_eq!(
        (ready, blocked, exited, acquired),
        (true, true, true, true),
        "visual leases did not serialize two real processes within their wait bounds"
    );
    Ok(())
}

/// Wait until a child creates one signal path or exits early.
fn wait_for(path: &std::path::Path, child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return true;
        }
        if child.try_wait().ok().flatten().is_some() || Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(10));
    }
}

/// Reap one child within a deadline, killing it if the deadline expires.
fn finish(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        sleep(Duration::from_millis(10));
    }
}
