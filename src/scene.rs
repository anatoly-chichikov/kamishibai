//! Scene OCR validation, image rendering, and illustration persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GrayImage, ImageFormat};
use leptess::LepTess;
use serde_json::Value;

use crate::cache::FileCache;
use crate::gemini::{GeminiClient, Transport};

const DEFAULT_LANG: &str = "eng";
static LANGUAGES: OnceLock<BTreeSet<String>> = OnceLock::new();

/// Report scene and illustration progress events.
pub trait Progress {
    /// Signal the start of one step.
    fn step(&mut self, name: &str);
    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>);
    /// Signal one retry within rendering.
    fn retry(&mut self, _name: &str, _attempt: usize, _reason: &str) {}
}

/// Translate one sentence into a scene JSON document.
pub trait Translator {
    /// Return one scene JSON document for the sentence and target language.
    fn translate(&self, sentence: &str, target: &str) -> Result<Value>;
}

/// Render one scene JSON document into an image.
pub trait Renderer {
    /// Return one rendered image for the scene and word.
    fn render(
        &self,
        scene: &Value,
        word: &str,
        progress: &mut dyn Progress,
    ) -> Result<DynamicImage>;
}

/// Detect OCR text from one grayscale image.
pub trait ImageText {
    /// Return the detected OCR text for one image.
    fn detected(&self, image: &GrayImage) -> Result<String>;
}

/// Detect OCR text from one scene and grayscale image pair.
pub trait SceneText {
    /// Return the detected OCR text for one scene and image pair.
    fn detected(&self, scene: &Value, image: &GrayImage) -> Result<String>;
}

/// Render one scene JSON payload into raw image bytes.
pub trait ImageSource {
    /// Return one encoded image payload for the scene and word.
    fn image(&self, scene: &Value, word: &str) -> Result<Vec<u8>>;
}

impl<T> ImageSource for GeminiClient<T>
where
    T: Transport,
{
    /// Return one encoded image payload for the scene and word.
    fn image(&self, scene: &Value, word: &str) -> Result<Vec<u8>> {
        GeminiClient::<T>::image(self, scene, word)
    }
}

/// Detect text with Tesseract after resolving installed languages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDetector {
    threshold: i32,
    lang: String,
}

impl TextDetector {
    /// Create one detector with the default OCR language.
    pub fn new(threshold: i32) -> Self {
        Self::listed(threshold, DEFAULT_LANG, installed().iter().cloned())
    }

    /// Create one detector with a custom OCR language string.
    pub fn custom(threshold: i32, lang: impl Into<String>) -> Self {
        Self::listed(threshold, lang, installed().iter().cloned())
    }

    /// Create one detector from an explicit language inventory.
    pub fn listed(
        threshold: i32,
        lang: impl Into<String>,
        available: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let inventory = available
            .into_iter()
            .map(Into::into)
            .collect::<BTreeSet<String>>();
        Self {
            threshold,
            lang: resolved(lang.into(), &inventory),
        }
    }

    /// Return the resolved OCR language selection.
    pub fn selection(&self) -> &str {
        self.lang.as_str()
    }
}

impl ImageText for TextDetector {
    /// Return the detected OCR text for one image.
    fn detected(&self, image: &GrayImage) -> Result<String> {
        let mut engine = LepTess::new(None, self.lang.as_str())?;
        engine.set_image_from_mem(encoded(image)?.as_slice())?;
        Ok(extracted(engine.get_tsv_text(0)?.as_str(), self.threshold))
    }
}

/// Route scene OCR by the scene target language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDetectors<D> {
    detectors: BTreeMap<String, D>,
    fallback: D,
}

impl<D> TextDetectors<D>
where
    D: ImageText,
{
    /// Create one scene-aware OCR router.
    pub fn new(detectors: BTreeMap<String, D>, fallback: D) -> Self {
        Self {
            detectors,
            fallback,
        }
    }
}

impl<D> SceneText for TextDetectors<D>
where
    D: ImageText,
{
    /// Return the detected OCR text for one scene and image pair.
    fn detected(&self, scene: &Value, image: &GrayImage) -> Result<String> {
        let code = scene
            .get("manga_panel")
            .and_then(|root| root.get("meta"))
            .and_then(|meta| meta.get("target_lang"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let detector = self.detectors.get(code).unwrap_or(&self.fallback);
        detector.detected(image)
    }
}

/// Detect white borders and gutters in one grayscale image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorderDetector {
    width: usize,
    brightness: u8,
    margin: usize,
}

impl BorderDetector {
    /// Create one border detector.
    pub fn new(width: usize, brightness: u8, margin: usize) -> Self {
        Self {
            width,
            brightness,
            margin,
        }
    }

    /// Return whether one white horizontal gutter exists.
    pub fn gutter(&self, image: &GrayImage) -> bool {
        if self.width == 0 {
            return true;
        }
        let mut run = 0usize;
        for y in 0..image.height() {
            if row(image, y) > f64::from(self.brightness) {
                run += 1;
                if run >= self.width {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }

    /// Return the edge names that fail the white border check.
    pub fn borders(&self, image: &GrayImage) -> Vec<String> {
        let mut failed = Vec::new();
        let rows = self.margin.min(image.height() as usize) as u32;
        let cols = self.margin.min(image.width() as usize) as u32;
        if rows > 0 && band(image, 0, 0, image.width(), rows) <= f64::from(self.brightness) {
            failed.push(String::from("top"));
        }
        if rows > 0
            && band(image, 0, image.height() - rows, image.width(), rows)
                <= f64::from(self.brightness)
        {
            failed.push(String::from("bottom"));
        }
        if cols > 0 && band(image, 0, 0, cols, image.height()) <= f64::from(self.brightness) {
            failed.push(String::from("left"));
        }
        if cols > 0
            && band(image, image.width() - cols, 0, cols, image.height())
                <= f64::from(self.brightness)
        {
            failed.push(String::from("right"));
        }
        failed
    }
}

/// Render one scene through Gemini and reject invalid manga images.
#[derive(Clone, Debug)]
pub struct MangaRenderer<C, D> {
    client: C,
    retries: usize,
    text: D,
    border: BorderDetector,
}

impl<C, D> MangaRenderer<C, D> {
    /// Create one validating manga renderer.
    pub fn new(client: C, retries: usize, text: D, border: BorderDetector) -> Self {
        Self {
            client,
            retries,
            text,
            border,
        }
    }
}

impl<C, D> Renderer for MangaRenderer<C, D>
where
    C: ImageSource,
    D: SceneText,
{
    /// Return one rendered image for the scene and word.
    fn render(
        &self,
        scene: &Value,
        word: &str,
        progress: &mut dyn Progress,
    ) -> Result<DynamicImage> {
        let panels = scene
            .get("manga_panel")
            .and_then(|root| root.get("panels"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut reason = String::new();
        for attempt in 0..self.retries {
            let gray =
                image::load_from_memory(self.client.image(scene, word)?.as_slice())?.into_luma8();
            let found = self.text.detected(scene, &gray)?;
            if !found.is_empty() {
                reason = format!("OCR detected text: '{found}'");
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            let failed = self.border.borders(&gray);
            if !failed.is_empty() {
                reason = format!("White border missing on: {}", failed.join(", "));
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            if panels > 1 && !self.border.gutter(&gray) {
                reason = String::from("No white gutter found");
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            return Ok(DynamicImage::ImageLuma8(gray));
        }
        bail!(
            "Rejected after {} attempts for '{}': {}",
            self.retries,
            word,
            reason
        );
    }
}

/// Cached illustration generator with scene JSON persistence.
#[derive(Clone, Debug)]
pub struct Illustration<C, T, R> {
    cache: C,
    translator: T,
    renderer: R,
}

impl<C, T, R> Illustration<C, T, R>
where
    C: FileCache,
    T: Translator,
    R: Renderer,
{
    /// Create one cached illustration generator.
    pub fn new(cache: C, translator: T, renderer: R) -> Self {
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
        word: &str,
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
        let image = self.renderer.render(&scene, word, progress)?;
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

fn installed() -> &'static BTreeSet<String> {
    LANGUAGES.get_or_init(system)
}

fn system() -> BTreeSet<String> {
    let Ok(output) = Command::new("tesseract").arg("--list-langs").output() else {
        return BTreeSet::new();
    };
    if !output.status.success() {
        return BTreeSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

fn resolved(lang: String, available: &BTreeSet<String>) -> String {
    let supported = lang
        .split('+')
        .filter(|code| available.contains(*code))
        .collect::<Vec<_>>();
    if supported.is_empty() {
        return String::from(DEFAULT_LANG);
    }
    supported.join("+")
}

fn encoded(image: &GrayImage) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image.clone()).write_to(&mut cursor, ImageFormat::Png)?;
    Ok(cursor.into_inner())
}

fn extracted(tsv: &str, threshold: i32) -> String {
    tsv.lines()
        .skip(1)
        .filter_map(word)
        .filter(|(score, text)| *score > threshold && text.chars().count() >= 2)
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn word(line: &str) -> Option<(i32, String)> {
    let parts = line.splitn(12, '\t').collect::<Vec<_>>();
    if parts.len() != 12 {
        return None;
    }
    Some((parts[10].parse().ok()?, String::from(parts[11].trim())))
}

fn row(image: &GrayImage, y: u32) -> f64 {
    band(image, 0, y, image.width(), 1)
}

fn band(image: &GrayImage, x: u32, y: u32, width: u32, height: u32) -> f64 {
    let mut total = 0u64;
    let mut count = 0u64;
    for ypos in y..(y + height) {
        for xpos in x..(x + width) {
            total += u64::from(image.get_pixel(xpos, ypos)[0]);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    total as f64 / count as f64
}

fn write_scene(path: &Path, scene: &Value) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(scene)?)?;
    Ok(())
}

fn write_image(path: &Path, image: &DynamicImage) -> Result<()> {
    let writer = BufWriter::new(fs::File::create(path)?);
    let mut encoder = JpegEncoder::new_with_quality(writer, 60);
    encoder.encode_image(image)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TextDetector, extracted};

    /// TSV extraction keeps confident multi-character words.
    #[test]
    fn tsv_extraction_keeps_confident_multi_character_words() {
        let tsv = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t95\tλόγος\n5\t1\t1\t1\t1\t2\t0\t0\t1\t1\t91\tkot";
        assert_eq!(
            extracted(tsv, 60),
            String::from("λόγος kot"),
            "TSV extraction no longer keeps confident multi character words"
        );
    }

    /// TSV extraction drops low-confidence and single-character tokens.
    #[test]
    fn tsv_extraction_drops_low_confidence_and_single_character_tokens() {
        let tsv = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t0\t0\t1\t1\t60\tab\n5\t1\t1\t1\t1\t2\t0\t0\t1\t1\t90\ta\n5\t1\t1\t1\t1\t3\t0\t0\t1\t1\t45\tλόγος";
        assert_eq!(
            extracted(tsv, 60),
            String::new(),
            "TSV extraction no longer drops low confidence and single character tokens"
        );
    }

    /// The default detector still resolves to English.
    #[test]
    fn default_detector_still_resolves_to_english() {
        assert_eq!(
            TextDetector::new(60).selection(),
            "eng",
            "default detector no longer resolves to English"
        );
    }
}
