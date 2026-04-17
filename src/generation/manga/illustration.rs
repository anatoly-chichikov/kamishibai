use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use serde_json::Value;

use crate::generation::IllustrationGenerator;
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

    /// Generate one cached illustration and report its filename and cache state.
    pub fn generate(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        let digest = &format!("{:x}", md5::compute(format!("{target}\0{sentence}")))[..12];
        let filename = format!("{digest}.jpg");
        let scenefile = format!("{digest}.json");
        let imagepath = self.cache.filepath(&filename)?;
        if self.cache.exists(&filename) {
            self.cached(&scenefile, progress)?;
            progress.done("Rendering manga", "cached", Some(imagepath.as_path()));
            return Ok((filename, true));
        }
        let scene = self.scene(sentence, target, &scenefile, progress)?;
        progress.step("Rendering manga");
        let image = self.renderer.render(&scene, progress)?;
        self.commit(&filename, &image)?;
        progress.done("Rendering manga", "rendered", Some(imagepath.as_path()));
        Ok((filename, false))
    }

    fn cached(&self, scenefile: &str, progress: &mut dyn Progress) -> Result<()> {
        if self.cache.exists(scenefile) {
            let path = self.cache.filepath(scenefile)?;
            progress.done("Composing scene", "cached", Some(path.as_path()));
            return Ok(());
        }
        progress.done("Composing scene", "cached", None);
        Ok(())
    }

    fn scene(
        &self,
        sentence: &str,
        target: &str,
        scenefile: &str,
        progress: &mut dyn Progress,
    ) -> Result<Value> {
        let scenepath = self.cache.filepath(scenefile)?;
        progress.step("Composing scene");
        if self.cache.exists(scenefile) {
            let scene = serde_json::from_str::<Value>(&fs::read_to_string(&scenepath)?)?;
            progress.done("Composing scene", "cached", Some(scenepath.as_path()));
            return Ok(scene);
        }
        let scene = self.translator.translate(sentence, target)?;
        let staged = self.cache.stage(".json")?;
        let result =
            write_scene(&staged, &scene).and_then(|_| self.cache.commit(&staged, scenefile));
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

impl<T, R> IllustrationGenerator for Illustration<T, R>
where
    T: Translator,
    R: Renderer,
{
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        target: &str,
        progress: &mut dyn Progress,
    ) -> Result<(String, bool)> {
        Illustration::<T, R>::generate(self, sentence, target, progress)
    }

    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Illustration::<T, R>::filepath(self, filename)
    }
}
