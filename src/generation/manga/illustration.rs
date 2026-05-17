use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use serde_json::Value;

use crate::generation::artifact_cache::Cache;

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

    /// Return the cache key digest shared by scene and picture artifacts.
    pub fn digest(&self, sentence: &str, target: &str) -> String {
        let raw = format!("{:x}", md5::compute(format!("{target}\0{sentence}")));
        raw[..12].to_string()
    }

    /// Return the cached scene filename for one (sentence, target) pair.
    pub fn scene_filename(&self, sentence: &str, target: &str) -> String {
        format!("{}.json", self.digest(sentence, target))
    }

    /// Return the cached picture filename for one (sentence, target) pair.
    pub fn picture_filename(&self, sentence: &str, target: &str) -> String {
        format!("{}.jpg", self.digest(sentence, target))
    }

    /// Generate one cached illustration and report its filename and cache state.
    pub fn generate(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let filename = self.picture_filename(sentence, target);
        let imagepath = self.cache.filepath(&filename)?;
        if self.cache.exists(&filename) {
            self.cached_scene(sentence, target, progress)?;
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((filename, true));
        }
        let scene = self.scene(sentence, target, progress)?;
        progress.step("Rendering manga");
        let image = self.renderer.render(&scene, progress)?;
        self.commit(&filename, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((filename, false))
    }

    /// Stage one: produce or load the cached scene JSON. Returns the cached filename.
    pub fn scene_only(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let scenefile = self.scene_filename(sentence, target);
        let scenepath = self.cache.filepath(&scenefile)?;
        progress.step("Composing scene");
        if self.cache.exists(&scenefile) {
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok((scenefile, true));
        }
        let scene = self.translator.translate(sentence, target)?;
        let staged = self.cache.stage(".json")?;
        let result =
            write_scene(&staged, &scene).and_then(|_| self.cache.commit(&staged, &scenefile));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result?;
        progress.done("Composing scene", "translated", Some(scenepath.as_path()));
        Ok((scenefile, false))
    }

    /// Stage two: render the picture from a cached scene JSON. Requires `scene_only`
    /// to have run for the same (sentence, target) so the scene JSON sits in cache.
    pub fn picture_only(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let filename = self.picture_filename(sentence, target);
        let imagepath = self.cache.filepath(&filename)?;
        if self.cache.exists(&filename) {
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((filename, true));
        }
        let scenefile = self.scene_filename(sentence, target);
        if !self.cache.exists(&scenefile) {
            bail!("scene JSON missing for picture stage; run scene_only first");
        }
        let scenepath = self.cache.filepath(&scenefile)?;
        let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
        progress.step("Rendering manga");
        let image = self.renderer.render(&scene, progress)?;
        self.commit(&filename, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((filename, false))
    }

    fn cached_scene(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<()> {
        let scenefile = self.scene_filename(sentence, target);
        if self.cache.exists(&scenefile) {
            let path = self.cache.filepath(&scenefile)?;
            progress.done("Composing scene", "cached", Some(path.as_path()));
            return Ok(());
        }
        progress.done("Composing scene", "cached", None);
        Ok(())
    }

    fn scene(&self, sentence: &str, target: &str, progress: &mut dyn Progress) -> Result<Value> {
        let scenefile = self.scene_filename(sentence, target);
        let scenepath = self.cache.filepath(&scenefile)?;
        progress.step("Composing scene");
        if self.cache.exists(&scenefile) {
            let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok(scene);
        }
        let scene = self.translator.translate(sentence, target)?;
        let staged = self.cache.stage(".json")?;
        let result =
            write_scene(&staged, &scene).and_then(|_| self.cache.commit(&staged, &scenefile));
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
