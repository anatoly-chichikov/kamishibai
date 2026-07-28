//! Tests for filesystem location resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use kamishibai::runtime::locations::{
    Context, LocationArgs, Locations, Platform, cache_home, cache_root, compact_path_with,
    data_home,
};
use tempfile::TempDir;

/// Mock location context for deterministic cross-platform path tests.
#[derive(Clone, Debug)]
struct MockContext {
    cwd: PathBuf,
    home: Option<PathBuf>,
    env: BTreeMap<String, String>,
    platform: Platform,
    cache: Option<PathBuf>,
    data: Option<PathBuf>,
    documents: Option<PathBuf>,
}

impl Context for MockContext {
    fn cwd(&self) -> Result<PathBuf> {
        Ok(self.cwd.clone())
    }

    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn var(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }

    fn platform(&self) -> Platform {
        self.platform
    }

    fn native_cache(&self) -> Option<PathBuf> {
        self.cache.clone()
    }

    fn native_data(&self) -> Option<PathBuf> {
        self.data.clone()
    }

    fn native_documents(&self) -> Option<PathBuf> {
        self.documents.clone()
    }
}

/// Build one mock context rooted in a temporary directory.
fn context(platform: Platform) -> (TempDir, MockContext) {
    let directory = TempDir::new().expect("path test tempdir must exist");
    let home = directory.path().join("home");
    let cwd = directory.path().join("cwd");
    fs::create_dir_all(&home).expect("path test home must exist");
    fs::create_dir_all(&cwd).expect("path test cwd must exist");
    (
        directory,
        MockContext {
            cwd,
            home: Some(home),
            env: BTreeMap::new(),
            platform,
            cache: None,
            data: None,
            documents: None,
        },
    )
}

/// Return the mock home path, which every regular context carries.
fn home(context: &MockContext) -> PathBuf {
    context
        .home
        .clone()
        .expect("regular path test context must have a home")
}

/// Explicit input paths resolve to absolute paths.
#[test]
fn explicit_input_paths_resolve_to_absolute_paths() {
    let (_directory, context) = context(Platform::Linux);
    let path = context.cwd.join("λέξη.json");
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: Some(PathBuf::from("λέξη.json")),
                output: None,
                cache: None,
            },
            context,
        )
        .input()
        .expect("explicit input must resolve"),
        path,
        "explicit input paths were not resolved against the current working directory"
    );
}

/// Environment input paths resolve when no positional path exists.
#[test]
fn environment_input_paths_resolve_when_no_positional_path_exists() {
    let (_directory, mut context) = context(Platform::Linux);
    context
        .env
        .insert(String::from("KAMISHIBAI_INPUT"), String::from("слово.json"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .input()
            .expect("environment input must resolve"),
        context.cwd.join("слово.json"),
        "environment input paths were not resolved when the positional path was absent"
    );
}

/// Current-directory kamishibai.json acts as the fallback input path.
#[test]
fn current_directory_kamishibai_json_acts_as_the_fallback_input_path() {
    let (_directory, context) = context(Platform::Linux);
    let path = context.cwd.join("kamishibai.json");
    fs::write(&path, "{}").expect("fallback input must write");
    assert_eq!(
        Locations::new(LocationArgs::default(), context)
            .input()
            .expect("fallback input must resolve"),
        path,
        "current-directory kamishibai.json was not used as the fallback input path"
    );
}

/// Missing input sources keep their actionable guidance.
#[test]
fn missing_input_sources_keep_actionable_guidance() {
    let (_directory, context) = context(Platform::Linux);
    let error = Locations::new(LocationArgs::default(), context)
        .input()
        .expect_err("missing input must fail")
        .to_string();
    assert!(
        error.contains("KAMISHIBAI_INPUT") && error.contains("kamishibai.json"),
        "missing input no longer identifies either fallback: {error}"
    );
}

/// Explicit output paths have priority over environment and platform defaults.
#[test]
fn explicit_output_paths_have_maximum_priority() {
    let (_directory, mut context) = context(Platform::Linux);
    context.env.insert(
        String::from("KAMISHIBAI_OUTPUT"),
        String::from("ignored-env"),
    );
    context.documents = Some(home(&context).join("Ignored Documents"));
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: None,
                output: Some(PathBuf::from("вывод")),
                cache: None,
            },
            context.clone(),
        )
        .output()
        .expect("explicit output must resolve"),
        context.cwd.join("вывод"),
        "explicit --out no longer has maximum priority"
    );
}

/// Environment output paths have priority over the platform default.
#[test]
fn environment_output_paths_have_second_priority() {
    let (_directory, mut context) = context(Platform::Linux);
    context
        .env
        .insert(String::from("KAMISHIBAI_OUTPUT"), String::from("вывод-env"));
    context.documents = Some(home(&context).join("Ignored Documents"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("environment output must resolve"),
        context.cwd.join("вывод-env"),
        "KAMISHIBAI_OUTPUT no longer has priority over the platform default"
    );
}

/// Darwin output defaults to the native Documents directory.
#[test]
fn darwin_output_defaults_to_native_documents() {
    let (_directory, mut context) = context(Platform::Darwin);
    context.documents = Some(home(&context).join("Documents"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("Darwin output must resolve"),
        home(&context).join("Documents").join("Kamishibai"),
        "Darwin output no longer uses the native Documents directory"
    );
}

/// Windows output defaults to the native Documents known folder.
#[test]
fn windows_output_defaults_to_the_documents_known_folder() {
    let (_directory, mut context) = context(Platform::Windows);
    context.documents = Some(home(&context).join("Moved").join("Documents"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("Windows output must resolve"),
        home(&context)
            .join("Moved")
            .join("Documents")
            .join("Kamishibai"),
        "Windows output assumed a profile Documents child instead of its known folder"
    );
}

/// Linux output honors the Documents path loaded from XDG user directories.
#[test]
fn linux_output_honors_the_xdg_documents_directory() {
    let (_directory, mut context) = context(Platform::Linux);
    context.documents = Some(home(&context).join("Документы"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("Linux XDG output must resolve"),
        home(&context).join("Документы").join("Kamishibai"),
        "Linux output ignored the configured XDG Documents directory"
    );
}

/// Linux output falls back to the conventional Documents child.
#[test]
fn linux_output_falls_back_to_the_home_documents_directory() {
    let (_directory, context) = context(Platform::Linux);
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("Linux fallback output must resolve"),
        home(&context).join("Documents").join("Kamishibai"),
        "Linux output no longer falls back to ~/Documents/Kamishibai"
    );
}

/// Unknown platforms fall back to a visible home directory child.
#[test]
fn unknown_platform_output_falls_back_to_home() {
    let (_directory, context) = context(Platform::Other);
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .output()
            .expect("generic fallback output must resolve"),
        home(&context).join("Kamishibai"),
        "unknown-platform output no longer falls back to ~/Kamishibai"
    );
}

/// Default output does not depend on the invocation's current directory.
#[test]
fn default_output_does_not_depend_on_the_current_directory() {
    let (directory, mut first) = context(Platform::Linux);
    let mut second = first.clone();
    first.cwd = directory.path().join("first");
    second.cwd = directory.path().join("second");
    assert_eq!(
        (
            Locations::new(LocationArgs::default(), first)
                .output()
                .expect("first default output must resolve"),
            Locations::new(LocationArgs::default(), second)
                .output()
                .expect("second default output must resolve"),
        ),
        (
            directory
                .path()
                .join("home")
                .join("Documents")
                .join("Kamishibai"),
            directory
                .path()
                .join("home")
                .join("Documents")
                .join("Kamishibai"),
        ),
        "default output still depends on the current directory"
    );
}

/// Missing Documents and home paths refuse with both supported overrides.
#[test]
fn missing_documents_and_home_refuse_with_output_overrides() {
    let (_directory, mut context) = context(Platform::Other);
    context.home = None;
    let error = Locations::new(LocationArgs::default(), context)
        .output()
        .expect_err("missing output roots must fail")
        .to_string();
    assert!(
        error.contains("--out") && error.contains("KAMISHIBAI_OUTPUT"),
        "missing output roots did not explain either override: {error}"
    );
}

/// Display paths shorten only the current home prefix.
#[test]
fn display_paths_shorten_the_current_home_prefix() {
    let (_directory, context) = context(Platform::Linux);
    let path = home(&context).join("Documents").join("Kamishibai");
    assert_eq!(
        compact_path_with(&context, path.as_path()),
        PathBuf::from("~")
            .join("Documents")
            .join("Kamishibai")
            .to_string_lossy(),
        "display paths no longer shorten the current home prefix"
    );
}

/// Absolute overrides resolve without a home directory.
#[test]
fn absolute_overrides_do_not_require_a_home_directory() {
    let (directory, mut context) = context(Platform::Windows);
    context.home = None;
    let data = directory.path().join("absolute-data");
    let cache = directory.path().join("absolute-cache");
    let output = directory.path().join("absolute-output");
    context.env.insert(
        String::from("KAMISHIBAI_DATA"),
        data.to_string_lossy().into_owned(),
    );
    context.env.insert(
        String::from("KAMISHIBAI_CACHE"),
        cache.to_string_lossy().into_owned(),
    );
    context.env.insert(
        String::from("KAMISHIBAI_OUTPUT"),
        output.to_string_lossy().into_owned(),
    );
    assert_eq!(
        (
            data_home(&context).expect("absolute data override must resolve"),
            cache_root(&context).expect("absolute cache override must resolve"),
            Locations::new(LocationArgs::default(), context)
                .output()
                .expect("absolute output override must resolve"),
        ),
        (data, cache, output),
        "absolute path overrides unexpectedly required a home directory"
    );
}

/// Explicit cache paths take precedence over all other cache rules.
#[test]
fn explicit_cache_paths_take_precedence_over_all_other_cache_rules() {
    let (_directory, context) = context(Platform::Linux);
    assert_eq!(
        Locations::new(
            LocationArgs {
                path: None,
                output: None,
                cache: Some(PathBuf::from("кэш")),
            },
            context.clone(),
        )
        .cache()
        .expect("explicit cache must resolve"),
        context.cwd.join("кэш"),
        "explicit cache paths no longer take precedence over default cache rules"
    );
}

/// Environment cache paths take precedence over XDG fallback rules.
#[test]
fn environment_cache_paths_take_precedence_over_xdg_fallback_rules() {
    let (_directory, mut context) = context(Platform::Linux);
    context
        .env
        .insert(String::from("KAMISHIBAI_CACHE"), String::from("μνήμη"));
    context
        .env
        .insert(String::from("XDG_CACHE_HOME"), String::from("ignored"));
    assert_eq!(
        Locations::new(LocationArgs::default(), context.clone())
            .cache()
            .expect("environment cache must resolve"),
        context.cwd.join("μνήμη"),
        "environment cache paths no longer take precedence over XDG fallback rules"
    );
}

/// Linux cache home keeps the XDG override.
#[test]
fn linux_cache_home_keeps_the_xdg_override() {
    let (_directory, mut context) = context(Platform::Linux);
    context
        .env
        .insert(String::from("XDG_CACHE_HOME"), String::from("cache-root"));
    assert_eq!(
        cache_home(&context).expect("Linux cache home must resolve"),
        context.cwd.join("cache-root"),
        "Linux cache home no longer keeps the XDG override"
    );
}

/// Linux data home keeps the XDG override.
#[test]
fn linux_data_home_keeps_the_xdg_override() {
    let (_directory, mut context) = context(Platform::Linux);
    context
        .env
        .insert(String::from("XDG_DATA_HOME"), String::from("data-root"));
    assert_eq!(
        data_home(&context).expect("Linux data home must resolve"),
        context.cwd.join("data-root"),
        "Linux data home no longer keeps the XDG override"
    );
}

/// Darwin cache home keeps the Library special-case.
#[test]
fn darwin_cache_home_keeps_the_library_special_case() {
    let (_directory, context) = context(Platform::Darwin);
    assert_eq!(
        cache_home(&context).expect("Darwin cache home must resolve"),
        home(&context).join("Library").join("Caches"),
        "Darwin cache home no longer keeps the Library special-case"
    );
}

/// Darwin data home keeps the Application Support special-case.
#[test]
fn darwin_data_home_keeps_the_application_support_special_case() {
    let (_directory, context) = context(Platform::Darwin);
    assert_eq!(
        data_home(&context).expect("Darwin data home must resolve"),
        home(&context).join("Library").join("Application Support"),
        "Darwin data home no longer keeps the Application Support special-case"
    );
}

/// Windows cache home uses the native local application data directory.
#[test]
fn windows_cache_home_uses_the_native_directory_without_home() {
    let (directory, mut context) = context(Platform::Windows);
    context.home = None;
    context.cache = Some(directory.path().join("LocalAppData"));
    assert_eq!(
        cache_home(&context).expect("Windows native cache must resolve"),
        directory.path().join("LocalAppData"),
        "Windows cache home did not use the native local application data directory"
    );
}

/// Windows data home uses the native roaming application data directory.
#[test]
fn windows_data_home_uses_the_native_directory_without_home() {
    let (directory, mut context) = context(Platform::Windows);
    context.home = None;
    context.data = Some(directory.path().join("RoamingAppData"));
    assert_eq!(
        data_home(&context).expect("Windows native data must resolve"),
        directory.path().join("RoamingAppData"),
        "Windows data home did not use the native roaming application data directory"
    );
}

/// Default cache root appends the kamishibai directory name.
#[test]
fn default_cache_root_appends_the_kamishibai_directory_name() {
    let (_directory, context) = context(Platform::Linux);
    assert_eq!(
        cache_root(&context).expect("default cache root must resolve"),
        home(&context).join(".cache").join("kamishibai"),
        "default cache root no longer appends the kamishibai directory name"
    );
}
