//! Filesystem path resolution for the CLI contract.

use std::env;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow};
use thiserror::Error;

const APPLICATION: &str = "kamishibai";
const DOCUMENTS_APPLICATION: &str = "Kamishibai";

/// Supported platform branches for location resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    /// Apple's desktop filesystem conventions.
    Darwin,
    /// Freedesktop/XDG filesystem conventions.
    Linux,
    /// Windows Known Folder conventions.
    Windows,
    /// A platform without a dedicated location convention.
    Other,
}

/// Environment and filesystem context for location resolution.
pub trait Context {
    /// Return the current working directory.
    fn cwd(&self) -> Result<PathBuf>;
    /// Return the current user home directory when one is available.
    fn home(&self) -> Option<PathBuf>;
    /// Return one optional environment variable.
    fn var(&self, name: &str) -> Option<String>;
    /// Return the active platform branch.
    fn platform(&self) -> Platform;
    /// Return the platform-native user cache directory when available.
    fn native_cache(&self) -> Option<PathBuf> {
        None
    }
    /// Return the platform-native user data directory when available.
    fn native_data(&self) -> Option<PathBuf> {
        None
    }
    /// Return the platform-native Documents directory when available.
    fn native_documents(&self) -> Option<PathBuf> {
        None
    }
}

/// Live system context for production location resolution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemContext;

/// Failure to infer a visible default study-output directory.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("default output directory cannot be determined; pass --out DIR or set KAMISHIBAI_OUTPUT")]
pub struct OutputUnavailable;

impl Context for SystemContext {
    fn cwd(&self) -> Result<PathBuf> {
        Ok(env::current_dir()?)
    }

    fn home(&self) -> Option<PathBuf> {
        dirs::home_dir()
    }

    fn var(&self, name: &str) -> Option<String> {
        env::var(name).ok()
    }

    fn platform(&self) -> Platform {
        if cfg!(target_os = "macos") {
            return Platform::Darwin;
        }
        if cfg!(target_os = "linux") {
            return Platform::Linux;
        }
        if cfg!(target_os = "windows") {
            return Platform::Windows;
        }
        Platform::Other
    }

    fn native_cache(&self) -> Option<PathBuf> {
        dirs::cache_dir()
    }

    fn native_data(&self) -> Option<PathBuf> {
        dirs::data_dir()
    }

    fn native_documents(&self) -> Option<PathBuf> {
        dirs::document_dir()
    }
}

/// CLI arguments relevant to filesystem location resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocationArgs {
    /// Explicit input JSON path.
    pub path: Option<PathBuf>,
    /// Explicit study output directory.
    pub output: Option<PathBuf>,
    /// Explicit artifact cache directory.
    pub cache: Option<PathBuf>,
}

/// Resolve input, output, and cache paths for one invocation.
#[derive(Clone, Debug)]
pub struct Locations<C> {
    args: LocationArgs,
    context: C,
}

impl<C> Locations<C>
where
    C: Context,
{
    /// Create one location resolver.
    pub fn new(args: LocationArgs, context: C) -> Self {
        Self { args, context }
    }

    /// Return the input JSON path.
    pub fn input(&self) -> Result<PathBuf> {
        if let Some(path) = self.args.path.as_ref() {
            return resolve(&self.context, path);
        }
        if let Some(path) = self.context.var("KAMISHIBAI_INPUT") {
            return resolve(&self.context, Path::new(path.as_str()));
        }
        let path = self.context.cwd()?.join("kamishibai.json");
        if path.is_file() {
            return Ok(normalize(path));
        }
        Err(anyhow!(
            "Input JSON path is not set; pass a path, set KAMISHIBAI_INPUT, or place kamishibai.json in the current directory"
        ))
    }

    /// Return the output directory.
    pub fn output(&self) -> Result<PathBuf> {
        if let Some(path) = self.args.output.as_ref() {
            return resolve(&self.context, path);
        }
        if let Some(path) = self.context.var("KAMISHIBAI_OUTPUT") {
            return resolve(&self.context, Path::new(path.as_str()));
        }
        default_output(&self.context)
    }

    /// Return the cache directory.
    pub fn cache(&self) -> Result<PathBuf> {
        if let Some(path) = self.args.cache.as_ref() {
            return resolve(&self.context, path);
        }
        cache_root(&self.context)
    }
}

/// Return the platform-specific user cache home.
pub fn cache_home<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    match context.platform() {
        Platform::Darwin => context
            .native_cache()
            .or_else(|| {
                context
                    .home()
                    .map(|home| home.join("Library").join("Caches"))
            })
            .map(normalize)
            .ok_or_else(|| missing_location("cache", "KAMISHIBAI_CACHE")),
        Platform::Windows => context
            .native_cache()
            .map(normalize)
            .ok_or_else(|| missing_location("cache", "KAMISHIBAI_CACHE")),
        Platform::Linux | Platform::Other => {
            if let Some(path) = context.var("XDG_CACHE_HOME") {
                return resolve(context, Path::new(path.as_str()));
            }
            context
                .home()
                .map(|home| normalize(home.join(".cache")))
                .ok_or_else(|| missing_location("cache", "KAMISHIBAI_CACHE"))
        }
    }
}

/// Return the platform-specific user data home.
pub fn data_home<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    if let Some(path) = context.var("KAMISHIBAI_DATA") {
        return resolve(context, Path::new(path.as_str()));
    }
    match context.platform() {
        Platform::Darwin => context
            .native_data()
            .or_else(|| {
                context
                    .home()
                    .map(|home| home.join("Library").join("Application Support"))
            })
            .map(normalize)
            .ok_or_else(|| missing_location("data", "KAMISHIBAI_DATA")),
        Platform::Windows => context
            .native_data()
            .map(normalize)
            .ok_or_else(|| missing_location("data", "KAMISHIBAI_DATA")),
        Platform::Linux | Platform::Other => {
            if let Some(path) = context.var("XDG_DATA_HOME") {
                return resolve(context, Path::new(path.as_str()));
            }
            context
                .home()
                .map(|home| normalize(home.join(".local").join("share")))
                .ok_or_else(|| missing_location("data", "KAMISHIBAI_DATA"))
        }
    }
}

/// Return the kamishibai cache root.
pub fn cache_root<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    if let Some(path) = context.var("KAMISHIBAI_CACHE") {
        return resolve(context, Path::new(path.as_str()));
    }
    Ok(cache_home(context)?.join(APPLICATION))
}

/// Return one display path with the current home directory shortened to `~`.
#[must_use]
pub fn compact_path(path: &Path) -> String {
    compact_path_with(&SystemContext, path)
}

/// Return one display path with the supplied context home shortened to `~`.
#[must_use]
pub fn compact_path_with<C>(context: &C, path: &Path) -> String
where
    C: Context,
{
    let Some(home) = context.home() else {
        return path.to_string_lossy().into_owned();
    };
    let Ok(relative) = path.strip_prefix(home) else {
        return path.to_string_lossy().into_owned();
    };
    if relative.as_os_str().is_empty() {
        return String::from("~");
    }
    Path::new("~").join(relative).to_string_lossy().into_owned()
}

fn default_output<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    if let Some(documents) = context.native_documents() {
        return Ok(normalize(documents.join(DOCUMENTS_APPLICATION)));
    }
    if context.platform() == Platform::Linux {
        if let Some(home) = context.home() {
            return Ok(normalize(
                home.join("Documents").join(DOCUMENTS_APPLICATION),
            ));
        }
    } else if let Some(home) = context.home() {
        return Ok(normalize(home.join(DOCUMENTS_APPLICATION)));
    }
    Err(OutputUnavailable.into())
}

fn resolve<C>(context: &C, path: &Path) -> Result<PathBuf>
where
    C: Context,
{
    if path.is_absolute() {
        return Ok(normalize(path.to_path_buf()));
    }
    let text = path.to_string_lossy();
    if text == "~" {
        return context.home().map(normalize).ok_or_else(missing_home);
    }
    if let Some(value) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return context
            .home()
            .map(|home| normalize(home.join(value)))
            .ok_or_else(missing_home);
    }
    Ok(normalize(context.cwd()?.join(path)))
}

fn missing_location(kind: &str, override_name: &str) -> anyhow::Error {
    anyhow!("{kind} directory cannot be determined; set {override_name} to an absolute path")
}

fn missing_home() -> anyhow::Error {
    anyhow!("home directory cannot be determined; use an absolute path")
}

fn normalize(path: PathBuf) -> PathBuf {
    let mut value = PathBuf::new();
    for item in path.components() {
        match item {
            Component::CurDir => {}
            Component::ParentDir => {
                value.pop();
            }
            Component::RootDir => {
                value.push(item.as_os_str());
            }
            Component::Normal(_) | Component::Prefix(_) => {
                value.push(item.as_os_str());
            }
        }
    }
    value
}
