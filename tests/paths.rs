//! Tests for filesystem location resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use kamishibai::runtime::locations::{
    Context, LocationArgs, Locations, Platform, cache_home, cache_root, data_home,
};
use tempfile::TempDir;

/// Mock location context for deterministic path tests.
#[derive(Clone, Debug)]
struct MockContext {
    cwd: PathBuf,
    home: PathBuf,
    env: BTreeMap<String, String>,
    platform: Platform,
}

impl Context for MockContext {
    fn cwd(&self) -> Result<PathBuf> {
        Ok(self.cwd.clone())
    }
    fn home(&self) -> Result<PathBuf> {
        Ok(self.home.clone())
    }
    fn var(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
    fn platform(&self) -> Platform {
        self.platform
    }
}

/// Build one mock context rooted in a temporary directory.
fn context(platform: Platform) -> Result<(TempDir, MockContext)> {
    let directory = TempDir::new()?;
    let home = directory.path().join("home");
    let cwd = directory.path().join("cwd");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&cwd)?;
    Ok((
        directory,
        MockContext {
            cwd,
            home,
            env: BTreeMap::new(),
            platform,
        },
    ))
}

/// Explicit input paths resolve to absolute paths.
#[test]
fn explicit_input_paths_resolve_to_absolute_paths() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    let path = context.cwd.join("λέξη.json");
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: Some(PathBuf::from("λέξη.json")),
                output: None,
                cache: None
            },
            context
        )
        .input()?,
        path,
        "explicit input paths were not resolved against the current working directory"
    );
    Ok(())
}

/// Environment input paths resolve when no positional path exists.
#[test]
fn environment_input_paths_resolve_when_no_positional_path_exists() -> Result<()> {
    let (_directory, mut context) = context(Platform::Other)?;
    context
        .env
        .insert(String::from("KAMISHIBAI_INPUT"), String::from("слово.json"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone()).input()?,
        context.cwd.join("слово.json"),
        "environment input paths were not resolved when the positional path was absent"
    );
    Ok(())
}

/// Current-directory kamishibai.json acts as the fallback input path.
#[test]
fn current_directory_kamishibai_json_acts_as_the_fallback_input_path() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    let path = context.cwd.join("kamishibai.json");
    fs::write(&path, "{}")?;
    assert_eq!(
        Locations::new(LocationArgs::default(), context).input()?,
        path,
        "current-directory kamishibai.json was not used as the fallback input path"
    );
    Ok(())
}

/// Missing input sources keep the frozen validation message.
#[test]
fn missing_input_sources_keep_the_frozen_validation_message() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    assert_eq!(
        Locations::new(LocationArgs::default(), context)
            .input()
            .unwrap_err()
            .to_string(),
        "Input JSON path is not set; pass a path, set KAMISHIBAI_INPUT, or place kamishibai.json in the current directory",
        "missing input sources no longer raise the frozen validation message"
    );
    Ok(())
}

/// Explicit output paths take precedence over the default location.
#[test]
fn explicit_output_paths_take_precedence_over_the_default_location() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: Some(PathBuf::from("kamishibai.json")),
                output: Some(PathBuf::from("вывод")),
                cache: None
            },
            context.clone()
        )
        .output()?,
        context.cwd.join("вывод"),
        "explicit output paths no longer take precedence over the default location"
    );
    Ok(())
}

/// Default output paths use the current directory kamishibai-out name.
#[test]
fn default_output_paths_use_the_current_directory_kamishibai_out_name() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone()).output()?,
        context.cwd.join("kamishibai-out"),
        "default output paths no longer use the current directory kamishibai out name"
    );
    Ok(())
}

/// Explicit cache paths take precedence over all other cache rules.
#[test]
fn explicit_cache_paths_take_precedence_over_all_other_cache_rules() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: Some(PathBuf::from("kamishibai.json")),
                output: None,
                cache: Some(PathBuf::from("кэш"))
            },
            context.clone()
        )
        .cache()?,
        context.cwd.join("кэш"),
        "explicit cache paths no longer take precedence over the default cache rules"
    );
    Ok(())
}

/// Environment cache paths take precedence over XDG fallback rules.
#[test]
fn environment_cache_paths_take_precedence_over_xdg_fallback_rules() -> Result<()> {
    let (_directory, mut context) = context(Platform::Other)?;
    context
        .env
        .insert(String::from("KAMISHIBAI_CACHE"), String::from("μνήμη"));
    context
        .env
        .insert(String::from("XDG_CACHE_HOME"), String::from("ignored"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone()).cache()?,
        context.cwd.join("μνήμη"),
        "environment cache paths no longer take precedence over XDG fallback rules"
    );
    Ok(())
}

/// Linux cache home keeps the XDG override.
#[test]
fn linux_cache_home_keeps_the_xdg_override() -> Result<()> {
    let (_directory, mut context) = context(Platform::Other)?;
    context
        .env
        .insert(String::from("XDG_CACHE_HOME"), String::from("cache-root"));
    assert_eq!(
        cache_home(&context)?,
        context.cwd.join("cache-root"),
        "linux cache home no longer keeps the XDG override"
    );
    Ok(())
}

/// Linux data home keeps the XDG override.
#[test]
fn linux_data_home_keeps_the_xdg_override() -> Result<()> {
    let (_directory, mut context) = context(Platform::Other)?;
    context
        .env
        .insert(String::from("XDG_DATA_HOME"), String::from("data-root"));
    assert_eq!(
        data_home(&context)?,
        context.cwd.join("data-root"),
        "linux data home no longer keeps the XDG override"
    );
    Ok(())
}

/// Darwin cache home keeps the Library special-case.
#[test]
fn darwin_cache_home_keeps_the_library_special_case() -> Result<()> {
    let (_directory, context) = context(Platform::Darwin)?;
    assert_eq!(
        cache_home(&context)?,
        context.home.join("Library").join("Caches"),
        "darwin cache home no longer keeps the Library special-case"
    );
    Ok(())
}

/// Darwin data home keeps the Application Support special-case.
#[test]
fn darwin_data_home_keeps_the_application_support_special_case() -> Result<()> {
    let (_directory, context) = context(Platform::Darwin)?;
    assert_eq!(
        data_home(&context)?,
        context.home.join("Library").join("Application Support"),
        "darwin data home no longer keeps the Application Support special-case"
    );
    Ok(())
}

/// Default cache root appends the kamishibai directory name.
#[test]
fn default_cache_root_appends_the_kamishibai_directory_name() -> Result<()> {
    let (_directory, context) = context(Platform::Other)?;
    assert_eq!(
        cache_root(&context)?,
        context.home.join(".cache").join("kamishibai"),
        "default cache root no longer appends the kamishibai directory name"
    );
    Ok(())
}
