//! Filesystem path resolution for the CLI contract.

use std::env;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow};

/// Supported platform branches for location resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Darwin,
    Other,
}

/// Environment and filesystem context for location resolution.
pub trait Context {
    /// Return the current working directory.
    fn cwd(&self) -> Result<PathBuf>;
    /// Return the current user home directory.
    fn home(&self) -> Result<PathBuf>;
    /// Return one optional environment variable.
    fn var(&self, name: &str) -> Option<String>;
    /// Return the active platform branch.
    fn platform(&self) -> Platform;
}

/// Live system context for production location resolution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemContext;

impl Context for SystemContext {
    /// Return the current working directory.
    fn cwd(&self) -> Result<PathBuf> {
        Ok(env::current_dir()?)
    }

    /// Return the current user home directory.
    fn home(&self) -> Result<PathBuf> {
        env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable is not set"))
    }

    /// Return one optional environment variable.
    fn var(&self, name: &str) -> Option<String> {
        env::var(name).ok()
    }

    /// Return the active platform branch.
    fn platform(&self) -> Platform {
        if cfg!(target_os = "macos") {
            return Platform::Darwin;
        }
        Platform::Other
    }
}

/// CLI arguments relevant to filesystem location resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocationArgs {
    pub path: Option<PathBuf>,
    pub output: Option<PathBuf>,
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
        Ok(normalize(self.context.cwd()?.join("kamishibai-out")))
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
    if context.platform() == Platform::Darwin {
        return Ok(normalize(context.home()?.join("Library").join("Caches")));
    }
    if let Some(path) = context.var("XDG_CACHE_HOME") {
        return resolve(context, Path::new(path.as_str()));
    }
    Ok(normalize(context.home()?.join(".cache")))
}

/// Return the platform-specific user data home.
pub fn data_home<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    if context.platform() == Platform::Darwin {
        return Ok(normalize(
            context.home()?.join("Library").join("Application Support"),
        ));
    }
    if let Some(path) = context.var("XDG_DATA_HOME") {
        return resolve(context, Path::new(path.as_str()));
    }
    Ok(normalize(context.home()?.join(".local").join("share")))
}

/// Return the kamishibai cache root.
pub fn cache_root<C>(context: &C) -> Result<PathBuf>
where
    C: Context,
{
    if let Some(path) = context.var("KAMISHIBAI_CACHE") {
        return resolve(context, Path::new(path.as_str()));
    }
    Ok(cache_home(context)?.join("kamishibai"))
}

fn resolve<C>(context: &C, path: &Path) -> Result<PathBuf>
where
    C: Context,
{
    let home = context.home()?;
    let cwd = context.cwd()?;
    let expanded = expand(path, &home);
    if expanded.is_absolute() {
        return Ok(normalize(expanded));
    }
    Ok(normalize(cwd.join(expanded)))
}

fn expand(path: &Path, home: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return home.to_path_buf();
    }
    if let Some(value) = text.strip_prefix("~/") {
        return home.join(value);
    }
    path.to_path_buf()
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
