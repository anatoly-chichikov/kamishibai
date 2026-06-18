use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use serde_json::Value;

use crate::generation::artifact_cache::{Cache, ILLUSTRATION_FILE, SCENE_FILE};

use super::{Progress, Renderer, Translator};

/// Cached illustration generator with scene JSON persistence.
#[derive(Clone, Debug)]
pub struct Illustration<T, R> {
    cache: Cache,
    translator: T,
    renderer: R,
}

impl<T, R> Illustration<T, R>
where
    T: Translator,
    R: Renderer,
{
    /// Create one cached illustration generator.
    pub fn new(cache: Cache, translator: T, renderer: R) -> Self {
        Self {
            cache,
            translator,
            renderer,
        }
    }

    /// Return the absolute path for one cached filename.
    pub fn filepath(&self, filename: &str) -> Result<PathBuf> {
        self.cache.filepath(filename)
    }

    /// Generate this card's cached illustration and report its cache state.
    ///
    /// One illustration belongs to one card folder, so the file is always
    /// `picture.jpg`; the cache hit is decided by the folder.
    pub fn generate(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let imagepath = self.cache.filepath(ILLUSTRATION_FILE)?;
        if self.cache.exists(ILLUSTRATION_FILE) {
            self.cached_scene(progress)?;
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((ILLUSTRATION_FILE.to_string(), true));
        }
        let scene = self.scene(sentence, target, progress)?;
        progress.step("Rendering manga");
        let image = self.renderer.render(&scene, progress)?;
        self.commit(ILLUSTRATION_FILE, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((ILLUSTRATION_FILE.to_string(), false))
    }

    /// Stage one: produce or load this card's cached scene JSON (`scene.json`).
    pub fn scene_only(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        progress.step("Composing scene");
        if self.cache.exists(SCENE_FILE) {
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok((SCENE_FILE.to_string(), true));
        }
        let scene = self.translator.translate(sentence, target)?;
        let staged = self.cache.stage(".json")?;
        let result =
            write_scene(&staged, &scene).and_then(|_| self.cache.commit(&staged, SCENE_FILE));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result?;
        progress.done("Composing scene", "translated", Some(scenepath.as_path()));
        Ok((SCENE_FILE.to_string(), false))
    }

    /// Stage two: render `picture.jpg` from this card's cached `scene.json`.
    /// Requires `scene_only` to have run for this card so the scene JSON exists.
    pub fn picture_only(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let _ = (sentence, target);
        let imagepath = self.cache.filepath(ILLUSTRATION_FILE)?;
        if self.cache.exists(ILLUSTRATION_FILE) {
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((ILLUSTRATION_FILE.to_string(), true));
        }
        if !self.cache.exists(SCENE_FILE) {
            bail!("scene JSON missing for picture stage; run scene_only first");
        }
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
        progress.step("Rendering manga");
        let image = self.renderer.render(&scene, progress)?;
        self.commit(ILLUSTRATION_FILE, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((ILLUSTRATION_FILE.to_string(), false))
    }

    fn cached_scene(&self, progress: &mut dyn Progress) -> Result<()> {
        if self.cache.exists(SCENE_FILE) {
            let path = self.cache.filepath(SCENE_FILE)?;
            progress.done("Composing scene", "cached", Some(path.as_path()));
            return Ok(());
        }
        progress.done("Composing scene", "cached", None);
        Ok(())
    }

    fn scene(&self, sentence: &str, target: &str, progress: &mut dyn Progress) -> Result<Value> {
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        progress.step("Composing scene");
        if self.cache.exists(SCENE_FILE) {
            let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok(scene);
        }
        let scene = self.translator.translate(sentence, target)?;
        let staged = self.cache.stage(".json")?;
        let result =
            write_scene(&staged, &scene).and_then(|_| self.cache.commit(&staged, SCENE_FILE));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result?;
        progress.done("Composing scene", "translated", Some(scenepath.as_path()));
        Ok(scene)
    }

    fn commit(&self, filename: &str, image: &DynamicImage) -> Result<()> {
        let staged = self.cache.stage(".jpg")?;
        let result = write_image(&staged, image).and_then(|_| self.cache.commit(&staged, filename));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result
    }
}

fn write_scene(path: &Path, scene: &Value) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(scene)?)?;
    Ok(())
}

fn write_image(path: &Path, image: &DynamicImage) -> Result<()> {
    let writer = BufWriter::new(File::create(path)?);
    let mut encoder = JpegEncoder::new_with_quality(writer, 60);
    encoder.encode_image(image)?;
    Ok(())
}
