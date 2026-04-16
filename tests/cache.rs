//! Tests for the persistent media cache.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use kamishibai::generation::artifact_cache::Cache;
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
