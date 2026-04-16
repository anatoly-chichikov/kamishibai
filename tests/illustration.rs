//! Tests for scene JSON and illustration persistence.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma};
use kamishibai::generation::artifact_cache::Cache;
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
    fn render(
        &self,
        _scene: &Value,
        _word: &str,
        _progress: &mut dyn Progress,
    ) -> Result<DynamicImage> {
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

/// Illustration generation writes both the scene JSON and the JPEG file.
#[test]
fn illustration_generation_writes_both_the_scene_json_and_the_jpeg_file() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let illustration = Illustration::new(
        Cache::new("manga-en", directory.path()),
        translator,
        FixedRenderer,
    );
    let mut progress = Recorder::default();
    let (filename, cached) = illustration.generate(
        "The cat is sleeping on the windowsill",
        "кошка",
        "en",
        &mut progress,
    )?;
    let digest = "0f7acb8b6e5b";
    let scene_path = illustration.filepath(&format!("{digest}.json"))?;
    let image_path = illustration.filepath(&format!("{digest}.jpg"))?;
    assert_eq!(
        (filename, cached, scene_path.exists(), image_path.exists()),
        (String::from("0f7acb8b6e5b.jpg"), false, true, true),
        "illustration generation no longer writes both the scene JSON and the JPEG file"
    );
    Ok(())
}

/// Cached scene files skip translator calls and report cached progress.
#[test]
fn cached_scene_files_skip_translator_calls_and_report_cached_progress() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let illustration = Illustration::new(
        Cache::new("manga-en", directory.path()),
        translator.clone(),
        FixedRenderer,
    );
    let digest = "0f7acb8b6e5b";
    std::fs::write(
        illustration.filepath(&format!("{digest}.json"))?,
        serde_json::to_string_pretty(&scene())?,
    )?;
    let mut progress = Recorder::default();
    let _result = illustration.generate(
        "The cat is sleeping on the windowsill",
        "кошка",
        "en",
        &mut progress,
    )?;
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

/// Legacy cached images omit the missing scene path.
#[test]
fn legacy_cached_images_omit_the_missing_scene_path() -> Result<()> {
    let directory = TempDir::new()?;
    let translator = CountingTranslator::new(scene());
    let illustration = Illustration::new(
        Cache::new("manga-en", directory.path()),
        translator,
        FixedRenderer,
    );
    let digest = "0f7acb8b6e5b";
    let image = illustration.filepath(&format!("{digest}.jpg"))?;
    DynamicImage::ImageLuma8(GrayImage::from_pixel(64, 64, Luma([128]))).save(&image)?;
    let mut progress = Recorder::default();
    let _result = illustration.generate(
        "The cat is sleeping on the windowsill",
        "кошка",
        "en",
        &mut progress,
    )?;
    assert_eq!(
        progress
            .events
            .iter()
            .find(|event| event.0 == "Composing scene")
            .and_then(|event| event.2.clone()),
        None,
        "legacy cached images no longer omit the missing scene path"
    );
    Ok(())
}

/// Failed scene commits remove the staged scene file.
#[test]
fn failed_scene_commits_remove_the_staged_scene_file() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::failing("manga-en", directory.path(), 0);
    let probe = cache.path();
    let illustration = Illustration::new(cache, CountingTranslator::new(scene()), FixedRenderer);
    let _error = illustration
        .generate(
            "The cat is sleeping on the windowsill",
            "кошка",
            "en",
            &mut Recorder::default(),
        )
        .unwrap_err();
    assert!(
        std::fs::read_dir(probe)?.next().is_none(),
        "failed scene commits no longer remove the staged scene file"
    );
    Ok(())
}

/// Failed image commits remove the staged image file.
#[test]
fn failed_image_commits_remove_the_staged_image_file() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::failing("manga-en", directory.path(), 1);
    let probe = cache.path();
    let illustration = Illustration::new(cache, CountingTranslator::new(scene()), FixedRenderer);
    let _error = illustration
        .generate(
            "The cat is sleeping on the windowsill",
            "кошка",
            "en",
            &mut Recorder::default(),
        )
        .unwrap_err();
    assert!(
        std::fs::read_dir(probe)?.count() == 1,
        "failed image commits no longer remove the staged image file"
    );
    Ok(())
}
