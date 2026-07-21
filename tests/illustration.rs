//! Tests for scene JSON and illustration persistence.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma};
use kamishibai::generation::artifact_cache::{Cache, ILLUSTRATION_FILE, SCENE_FILE};
use kamishibai::generation::manga::{Illustration, Progress, Renderer, Translator};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Counting translator for illustration tests.
#[derive(Clone, Debug)]
struct CountingTranslator {
    calls: Rc<RefCell<usize>>,
    scene: Value,
}

impl CountingTranslator {
    /// Create one counting translator.
    fn new(scene: Value) -> Self {
        Self {
            calls: Rc::new(RefCell::new(0)),
            scene,
        }
    }
}

impl Translator for CountingTranslator {
    /// Return one fixed scene JSON document and count the call.
    fn translate(&self, _sentence: &str, _target: &str) -> Result<Value> {
        *self.calls.borrow_mut() += 1;
        Ok(self.scene.clone())
    }
}

/// Fixed renderer for illustration tests.
#[derive(Clone, Debug, Default)]
struct FixedRenderer;

impl Renderer for FixedRenderer {
    /// Return one fixed grayscale image.
    fn render(&self, _scene: &Value, _progress: &mut dyn Progress) -> Result<DynamicImage> {
        Ok(DynamicImage::ImageLuma8(GrayImage::from_pixel(
            64,
            64,
            Luma([128]),
        )))
    }
}

/// Progress recorder for illustration tests.
#[derive(Clone, Debug, Default)]
struct Recorder {
    events: Vec<(String, String, Option<PathBuf>)>,
}

impl Progress for Recorder {
    fn step(&mut self, name: &str) {
        self.events
            .push((String::from(name), String::from("step"), None));
    }
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>) {
        self.events.push((
            String::from(name),
            String::from(label),
            path.map(PathBuf::from),
        ));
    }
}

/// Return one fixed scene JSON document.
fn scene() -> Value {
    json!({"manga_panel":{"panels":[{"id":"x"}],"meta":{"title":"t","description":"d"}}})
}

/// Return one valid first visual-policy revision.
fn revision_a() -> &'static str {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}

/// Return one valid second visual-policy revision.
fn revision_b() -> &'static str {
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
}

/// Illustration generation writes both the scene JSON and the JPEG file.
#[test]
fn illustration_generation_writes_both_the_scene_json_and_the_jpeg_file() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let cache = Cache::new("manga-en", directory.path()).visual(revision_a())?;
    let illustration = Illustration::new(cache, translator, FixedRenderer);
    let mut progress = Recorder::default();
    let (filename, cached) =
        illustration.generate("The cat is sleeping on the windowsill", "en", &mut progress)?;
    let scene_path = illustration.filepath(SCENE_FILE)?;
    let image_path = illustration.filepath(ILLUSTRATION_FILE)?;
    assert_eq!(
        (filename, cached, scene_path.exists(), image_path.exists()),
        (String::from("picture.jpg"), false, true, true),
        "illustration generation no longer writes both the scene JSON and the JPEG file"
    );
    Ok(())
}

/// Cached scene files skip translator calls and report cached progress.
#[test]
fn cached_scene_files_skip_translator_calls_and_report_cached_progress() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let cache = Cache::new("manga-en", directory.path()).visual(revision_a())?;
    let illustration = Illustration::new(cache, translator.clone(), FixedRenderer);
    std::fs::write(
        illustration.filepath(SCENE_FILE)?,
        serde_json::to_string_pretty(&scene())?,
    )?;
    let mut progress = Recorder::default();
    let _result =
        illustration.generate("The cat is sleeping on the windowsill", "en", &mut progress)?;
    assert_eq!(
        (
            *translator.calls.borrow(),
            progress
                .events
                .iter()
                .find(|event| event.0 == "Composing scene" && event.1 != "step")
                .map(|event| event.1.as_str())
        ),
        (0, Some("cached")),
        "cached scene files no longer skip translator calls and report cached progress"
    );
    Ok(())
}

/// Legacy top-level images are ignored by a revision-scoped illustration.
#[test]
fn legacy_top_level_images_are_ignored_by_revision_scoped_illustrations() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let cache = Cache::new("manga-en", directory.path());
    fs::write(cache.filepath(ILLUSTRATION_FILE)?, b"legacy")?;
    let visual = cache.visual(revision_a())?;
    let illustration = Illustration::new(visual, translator.clone(), FixedRenderer);
    let mut progress = Recorder::default();
    let result =
        illustration.generate("The cat is sleeping on the windowsill", "en", &mut progress)?;
    assert_eq!(
        (
            result.1,
            *translator.calls.borrow(),
            fs::read(cache.filepath(ILLUSTRATION_FILE)?)?,
            illustration.filepath(ILLUSTRATION_FILE)?.exists(),
        ),
        (false, 1, b"legacy".to_vec(), true),
        "a legacy top-level image must not become a current visual cache hit"
    );
    Ok(())
}

/// Matching revisions reuse the cached scene and picture together.
#[test]
fn matching_revisions_reuse_the_cached_scene_and_picture_together() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let cache = Cache::new("manga-en", directory.path()).visual(revision_a())?;
    let illustration = Illustration::new(cache, translator.clone(), FixedRenderer);
    let first = illustration.generate(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    let second = illustration.generate(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    assert_eq!(
        (first.1, second.1, *translator.calls.borrow()),
        (false, true, 1),
        "a matching revision must not regenerate either visual stage"
    );
    Ok(())
}

/// Changed revisions regenerate both the scene and picture.
#[test]
fn changed_revisions_regenerate_both_the_scene_and_picture() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("manga-en", directory.path());
    let translator = CountingTranslator::new(scene());
    let first = Illustration::new(
        cache.visual(revision_a())?,
        translator.clone(),
        FixedRenderer,
    );
    let _result = first.generate(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    let second = Illustration::new(
        cache.visual(revision_b())?,
        translator.clone(),
        FixedRenderer,
    );
    let result = second.generate(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    assert_eq!(
        (result.1, *translator.calls.borrow()),
        (false, 2),
        "a changed revision must not reuse either stale visual stage"
    );
    Ok(())
}

/// The scene stage cannot inspect a sibling revision's cache.
#[test]
fn scene_stage_cannot_inspect_a_sibling_revision_cache() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("manga-en", directory.path());
    let translator = CountingTranslator::new(scene());
    let first = Illustration::new(
        cache.visual(revision_a())?,
        translator.clone(),
        FixedRenderer,
    );
    let _result = first.scene_only(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    let second = Illustration::new(
        cache.visual(revision_b())?,
        translator.clone(),
        FixedRenderer,
    );
    let result = second.scene_only(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    )?;
    assert_eq!(
        (result.1, *translator.calls.borrow()),
        (false, 2),
        "the scene stage must not inspect an artifact from an older revision"
    );
    Ok(())
}

/// The picture stage ignores a scene from a sibling revision.
#[test]
fn picture_stage_ignores_a_scene_from_a_sibling_revision() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::new("manga-en", directory.path());
    let first = cache.visual(revision_a())?;
    fs::write(first.filepath(SCENE_FILE)?, serde_json::to_vec(&scene())?)?;
    fs::write(first.filepath(ILLUSTRATION_FILE)?, b"old")?;
    let second = cache.visual(revision_b())?;
    let illustration = Illustration::new(
        second.clone(),
        CountingTranslator::new(scene()),
        FixedRenderer,
    );
    let result = illustration.picture_only(
        "The cat is sleeping on the windowsill",
        "en",
        &mut Recorder::default(),
    );
    assert_eq!(
        (
            result.is_err(),
            first.exists(SCENE_FILE),
            first.exists(ILLUSTRATION_FILE),
            second.exists(SCENE_FILE),
            second.exists(ILLUSTRATION_FILE),
        ),
        (true, true, true, false, false),
        "the picture stage must fail closed without deleting a sibling revision"
    );
    Ok(())
}

/// Failed scene commits remove the staged scene file.
#[test]
fn failed_scene_commits_remove_the_staged_scene_file() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::failing("manga-en", directory.path(), 0).visual(revision_a())?;
    let probe = cache.path();
    let illustration = Illustration::new(cache, CountingTranslator::new(scene()), FixedRenderer);
    let _error = illustration
        .generate(
            "The cat is sleeping on the windowsill",
            "en",
            &mut Recorder::default(),
        )
        .unwrap_err();
    assert!(
        std::fs::read_dir(probe)?.next().is_none(),
        "failed scene commits must remove every staged scene file"
    );
    Ok(())
}

/// Failed image commits remove the staged image file.
#[test]
fn failed_image_commits_remove_the_staged_image_file() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::failing("manga-en", directory.path(), 1).visual(revision_a())?;
    let probe = cache.path();
    let illustration = Illustration::new(cache, CountingTranslator::new(scene()), FixedRenderer);
    let _error = illustration
        .generate(
            "The cat is sleeping on the windowsill",
            "en",
            &mut Recorder::default(),
        )
        .unwrap_err();
    assert!(
        std::fs::read_dir(probe)?.count() == 1,
        "failed image commits must keep only the committed scene"
    );
    Ok(())
}
