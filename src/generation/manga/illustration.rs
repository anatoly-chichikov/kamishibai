use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use serde_json::Value;

use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, SCENE_FILE, SOFT_FILE,
};

use super::{Progress, Renderer, Translator};
use crate::generation::{VisualFailure, VisualResult};

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
        self.scene_only_observed(sentence, target, progress)
            .map_err(VisualFailure::into_error)
    }

    /// Perform this visual stage while preserving typed moderation refusals.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn scene_only_observed(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> VisualResult<(String, bool)> {
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        progress.step("Composing scene");
        if self.cache.exists(SCENE_FILE) {
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok((SCENE_FILE.to_string(), true));
        }
        let scene = self.translator.translate_observed(sentence, target)?;
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
        self.picture_only_observed(sentence, target, progress)
            .map_err(VisualFailure::into_error)
    }

    /// Perform this visual stage while preserving typed moderation refusals.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn picture_only_observed(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> VisualResult<(String, bool)> {
        let _ = (sentence, target);
        let imagepath = self.cache.filepath(ILLUSTRATION_FILE)?;
        if self.cache.exists(ILLUSTRATION_FILE) {
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((ILLUSTRATION_FILE.to_string(), true));
        }
        if !self.cache.exists(SCENE_FILE) {
            return Err(anyhow::anyhow!(
                "scene JSON missing for picture stage; run scene_only first"
            )
            .into());
        }
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
        progress.step("Rendering manga");
        let image = self.renderer.render_observed(&scene, progress)?;
        if self.cache.exists(SOFT_FILE)
            && let Some(scene) =
                super::render::salvaged_scene(&self.cache.path().join(IMAGE_ATTEMPTS_DIRECTORY))?
        {
            let staged = self.cache.stage(".json")?;
            write_scene(&staged, &scene)?;
            self.cache.commit(&staged, SCENE_FILE)?;
        }
        self.commit(ILLUSTRATION_FILE, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((ILLUSTRATION_FILE.to_string(), false))
    }

    /// Recompose the scene and render a missing picture without exposing a rejected scene.
    ///
    /// A cached picture remains authoritative. When the picture is absent, the replacement
    /// scene is rendered in memory before either accepted artifact replaces the cache.
    pub fn picture_with_recomposed_scene(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        self.picture_with_recomposed_scene_observed(sentence, target, progress)
            .map_err(VisualFailure::into_error)
    }

    /// Perform this visual stage while preserving typed moderation refusals.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn picture_with_recomposed_scene_observed(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> VisualResult<(String, bool)> {
        let imagepath = self.cache.filepath(ILLUSTRATION_FILE)?;
        if self.cache.exists(ILLUSTRATION_FILE) {
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((ILLUSTRATION_FILE.to_string(), true));
        }
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        progress.step("Composing scene");
        let scene = self.translator.translate_observed(sentence, target)?;
        progress.step("Rendering manga");
        let image = self.renderer.render_observed(&scene, progress)?;
        let scene = if self.cache.exists(SOFT_FILE) {
            super::render::salvaged_scene(&self.cache.path().join(IMAGE_ATTEMPTS_DIRECTORY))?
                .unwrap_or(scene)
        } else {
            scene
        };
        self.publish(&scene, &image)?;
        progress.done("Composing scene", "translated", Some(scenepath.as_path()));
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((ILLUSTRATION_FILE.to_string(), false))
    }

    /// Publish the best archived frame without contacting a model after terminal moderation.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn salvage(&self) -> VisualResult<Option<String>> {
        let directory = self.cache.filepath(IMAGE_ATTEMPTS_DIRECTORY)?;
        let Some(bytes) = super::render::salvaged_best(&directory) else {
            return Ok(None);
        };
        let image = DynamicImage::ImageLuma8(image::load_from_memory(&bytes)?.into_luma8());
        let scene = super::render::salvaged_scene(&directory)?
            .context("salvaged frame has no archived scene")?;
        self.publish(&scene, &image)?;
        Ok(Some(ILLUSTRATION_FILE.to_owned()))
    }

    fn publish(&self, scene: &Value, image: &DynamicImage) -> VisualResult<()> {
        let scenepath = self.cache.filepath(SCENE_FILE)?;
        let staged_scene = self.cache.stage(".json")?;
        let staged_image = match self.cache.stage(".jpg") {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_file(&staged_scene);
                return Err(error.into());
            }
        };
        let previous_scene = if self.cache.exists(SCENE_FILE) {
            let path = match self.cache.stage(".previous.json") {
                Ok(path) => path,
                Err(error) => {
                    let _ = fs::remove_file(&staged_scene);
                    let _ = fs::remove_file(&staged_image);
                    return Err(error.into());
                }
            };
            if let Err(error) = fs::copy(&scenepath, &path) {
                let _ = fs::remove_file(&staged_scene);
                let _ = fs::remove_file(&staged_image);
                let _ = fs::remove_file(&path);
                return Err(error.into());
            }
            Some(path)
        } else {
            None
        };
        let result =
            write_scene(&staged_scene, scene).and_then(|_| write_image(&staged_image, image));
        if let Err(error) = result {
            let _ = fs::remove_file(&staged_scene);
            let _ = fs::remove_file(&staged_image);
            remove_optional(previous_scene.as_deref());
            return Err(error.into());
        }
        if let Err(error) = self.cache.commit(&staged_scene, SCENE_FILE) {
            let _ = fs::remove_file(&staged_scene);
            let _ = fs::remove_file(&staged_image);
            remove_optional(previous_scene.as_deref());
            return Err(error.into());
        }
        if let Err(error) = self.cache.commit(&staged_image, ILLUSTRATION_FILE) {
            let _ = fs::remove_file(&staged_image);
            restore_scene(&scenepath, previous_scene.as_deref()).with_context(|| {
                format!("scene rollback failed after picture commit failed: {error:#}")
            })?;
            return Err(error.into());
        }
        remove_optional(previous_scene.as_deref());
        Ok(())
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

fn restore_scene(scene: &Path, previous: Option<&Path>) -> Result<()> {
    match previous {
        Some(previous) => {
            fs::remove_file(scene)?;
            fs::rename(previous, scene)?;
        }
        None => fs::remove_file(scene)?,
    }
    Ok(())
}

fn remove_optional(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}
