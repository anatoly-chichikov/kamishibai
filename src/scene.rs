//! Scene OCR validation, image rendering, and illustration persistence.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow, bail};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GrayImage};
use ocr_rs::OcrResult_;
use serde_json::Value;

use crate::cache::FileCache;
use crate::gemini::{GeminiClient, Transport};
use crate::ocr;

const DEFAULT_LANG: &str = "eng";
type Lazy = Rc<QuietEngine>;

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

struct Redirect {
    sink: File,
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
}

impl Redirect {
    /// Redirect stdout and stderr into one sink file.
    fn new(sink: File) -> Result<Self> {
        let item = Self {
            sink,
            stdout: Some(saved_stdout()?),
            stderr: Some(saved_stderr()?),
        };
        if let Err(error) = item.mute() {
            let _ = item.restore();
            return Err(error);
        }
        Ok(item)
    }

    /// Redirect stdout and stderr into the sink file.
    fn mute(&self) -> Result<()> {
        flushed()?;
        muted(&self.sink)
    }

    /// Restore stdout and stderr after one redirect.
    fn restore(mut self) -> Result<()> {
        flushed()?;
        restored_stdout(
            self.stdout
                .take()
                .ok_or_else(|| anyhow!("Saved stdout descriptor is missing"))?,
        )?;
        restored_stderr(
            self.stderr
                .take()
                .ok_or_else(|| anyhow!("Saved stderr descriptor is missing"))?,
        )
    }
}

/// Return the process-wide redirect gate.
fn gate() -> &'static Mutex<()> {
    static CELL: OnceLock<Mutex<()>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(()))
}

/// Run one closure while holding the process-wide redirect gate.
fn locked<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _guard = gate()
        .lock()
        .map_err(|_| anyhow!("Redirect gate is poisoned"))?;
    action()
}

/// Run one closure while stdout and stderr are redirected to /dev/null.
fn hush<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    locked(|| quiet(action))
}

/// Run one closure while stdout and stderr stay redirected to /dev/null.
fn quiet<T, F>(action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let sink = File::options().read(true).write(true).open("/dev/null")?;
    let item = Redirect::new(sink)?;
    let result = action();
    item.restore()?;
    result
}

/// Drop one value while stdout and stderr stay redirected to /dev/null.
fn discarded<T>(item: T) -> Result<()> {
    quiet(|| {
        drop(item);
        Ok(())
    })
}

/// Flush the noisy output stream before one descriptor swap.
fn flushed() -> Result<()> {
    std::io::stdout().flush()?;
    std::io::stderr().flush()?;
    Ok(())
}

/// Return one duplicate of stdout.
fn saved_stdout() -> Result<OwnedFd> {
    rustix::io::dup(std::io::stdout())
        .map_err(|error| anyhow!("Failed to duplicate stdout: {}", error))
}

/// Return one duplicate of stderr.
fn saved_stderr() -> Result<OwnedFd> {
    rustix::io::dup(std::io::stderr())
        .map_err(|error| anyhow!("Failed to duplicate stderr: {}", error))
}

/// Redirect stdout and stderr into the sink file.
fn muted(sink: &File) -> Result<()> {
    rustix::stdio::dup2_stdout(sink)
        .map_err(|error| anyhow!("Failed to redirect stdout: {}", error))?;
    rustix::stdio::dup2_stderr(sink)
        .map_err(|error| anyhow!("Failed to redirect stderr: {}", error))
}

/// Restore stdout from the saved descriptor.
fn restored_stdout(saved: OwnedFd) -> Result<()> {
    rustix::stdio::dup2_stdout(&saved)
        .map_err(|error| anyhow!("Failed to restore stdout: {}", error))
}

/// Restore stderr from the saved descriptor.
fn restored_stderr(saved: OwnedFd) -> Result<()> {
    rustix::stdio::dup2_stderr(&saved)
        .map_err(|error| anyhow!("Failed to restore stderr: {}", error))
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

struct QuietEngine {
    item: RefCell<Option<Result<Rc<ocr_rs::OcrEngine>, String>>>,
}

impl QuietEngine {
    /// Create one shared quiet engine slot.
    fn new() -> Self {
        Self {
            item: RefCell::new(None),
        }
    }

    /// Return whether the engine slot is still empty.
    fn empty(&self) -> bool {
        self.item.borrow().is_none()
    }

    /// Store one resolved engine result.
    fn store(&self, item: Result<Rc<ocr_rs::OcrEngine>, String>) {
        *self.item.borrow_mut() = Some(item);
    }

    /// Return one cloned engine handle from the slot.
    fn engine(&self) -> Result<Rc<ocr_rs::OcrEngine>> {
        match self
            .item
            .borrow()
            .as_ref()
            .expect("text detector engine state must be initialized")
        {
            Ok(item) => Ok(item.clone()),
            Err(error) => Err(anyhow!(error.clone())),
        }
    }
}

impl Drop for QuietEngine {
    /// Drop the cached OCR engine while native diagnostics stay muted.
    fn drop(&mut self) {
        if let Some(item) = self.item.get_mut().take() {
            let _ = locked(|| discarded(item));
        }
    }
}

/// Detect text with PaddleOCR after resolving one legacy OCR token string.
#[derive(Clone)]
pub struct TextDetector {
    cache: PathBuf,
    engine: Lazy,
    threshold: i32,
    lang: String,
}

impl TextDetector {
    /// Create one detector with the default OCR language.
    pub fn new(threshold: i32) -> Self {
        Self::cached(threshold, DEFAULT_LANG, std::env::temp_dir())
    }

    /// Create one detector with a custom OCR language string and default cache root.
    pub fn custom(threshold: i32, lang: impl Into<String>) -> Self {
        Self::cached(threshold, lang, std::env::temp_dir())
    }

    /// Create one detector with a custom OCR language string and explicit cache root.
    pub fn cached(threshold: i32, lang: impl Into<String>, cache: impl Into<PathBuf>) -> Self {
        Self {
            cache: cache.into(),
            engine: Rc::new(QuietEngine::new()),
            threshold,
            lang: lang.into(),
        }
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
            .collect::<Vec<String>>();
        Self::custom(threshold, resolved(lang.into(), inventory.as_slice()))
    }

    /// Return the resolved OCR language selection.
    pub fn selection(&self) -> &str {
        self.lang.as_str()
    }

    /// Return the lazily initialized OCR engine.
    fn engine(&self) -> Result<Rc<ocr_rs::OcrEngine>> {
        if self.engine.empty() {
            let item = hush(|| ocr::engine(self.lang.as_str(), self.cache.as_path()).map(Rc::new))
                .map_err(|error| error.to_string());
            self.engine.store(item);
        }
        self.engine.engine()
    }
}

impl std::fmt::Debug for TextDetector {
    /// Render one stable debug view for test diagnostics.
    fn fmt(&self, item: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        item.debug_struct("TextDetector")
            .field("cache", &self.cache)
            .field("threshold", &self.threshold)
            .field("lang", &self.lang)
            .finish()
    }
}

impl PartialEq for TextDetector {
    /// Compare one detector by its stable configuration fields.
    fn eq(&self, other: &Self) -> bool {
        self.cache == other.cache && self.threshold == other.threshold && self.lang == other.lang
    }
}

impl Eq for TextDetector {}

/// Convert OCR results into one filtered whitespace-normalized text string.
fn extracted(items: &[OcrResult_], threshold: i32) -> String {
    let mut rows = items.iter().collect::<Vec<_>>();
    rows.sort_by_key(|item| (item.bbox.rect.top(), item.bbox.rect.left()));
    rows.into_iter()
        .filter_map(|item| word(item, threshold))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return one filtered OCR text fragment when the result is confident enough.
fn word(item: &OcrResult_, threshold: i32) -> Option<String> {
    let score = (item.confidence * 100.0) as i32;
    let text = item.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if score > threshold && text.chars().count() >= 2 {
        return Some(text);
    }
    None
}

impl ImageText for TextDetector {
    /// Return the detected OCR text for one image.
    fn detected(&self, image: &GrayImage) -> Result<String> {
        let image = DynamicImage::ImageLuma8(image.clone());
        let engine = self.engine()?;
        let items = hush(|| Ok(engine.recognize(&image)?))
            .map_err(|error| anyhow!("OCR failed for '{}': {}", self.lang, error))?;
        Ok(extracted(items.as_slice(), self.threshold))
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

fn resolved(lang: String, available: &[String]) -> String {
    let supported = lang
        .split('+')
        .filter(|code| available.iter().any(|item| item == *code))
        .collect::<Vec<_>>();
    if supported.is_empty() {
        return String::from(DEFAULT_LANG);
    }
    supported.join("+")
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
    use std::fs;

    use anyhow::Result;

    use super::{Redirect, TextDetector, discarded, locked, quiet, resolved};

    struct NoisyDrop;

    impl Drop for NoisyDrop {
        /// Write to stdout and stderr when the test value is dropped.
        fn drop(&mut self) {
            let _ = rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes());
            let _ = rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes());
        }
    }

    /// Redirect routes noisy process output into the sink file.
    #[test]
    fn redirect_routes_stdout_and_stderr_into_the_sink_file() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes())?;
            rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes())?;
            item.restore()
        })?;
        assert_eq!(
            fs::read_to_string(sink.path())?,
            String::from("άλφα\nβήτα\n"),
            "redirect no longer routes stdout and stderr into the sink file"
        );
        Ok(())
    }

    /// Quiet redirection discards noisy process output inside the closure.
    #[test]
    fn quiet_redirection_discards_stdout_and_stderr_inside_the_closure() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            quiet(|| {
                rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes())?;
                rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes())?;
                Ok(())
            })?;
            item.restore()
        })?;
        assert_eq!(
            fs::read_to_string(sink.path())?,
            String::new(),
            "quiet redirection no longer discards stdout and stderr inside the closure"
        );
        Ok(())
    }

    /// Discarded drops mute stdout and stderr during value destruction.
    #[test]
    fn discarded_drops_mute_stdout_and_stderr_during_value_destruction() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            discarded(NoisyDrop)?;
            item.restore()
        })?;
        assert_eq!(
            fs::read_to_string(sink.path())?,
            String::new(),
            "discarded drops no longer mute stdout and stderr during value destruction"
        );
        Ok(())
    }

    /// Explicit inventories keep supported OCR tokens in order.
    #[test]
    fn explicit_inventories_keep_supported_ocr_tokens_in_order() {
        assert_eq!(
            resolved(
                String::from("eng+ell"),
                &[
                    String::from("eng"),
                    String::from("ell"),
                    String::from("osd")
                ]
            ),
            String::from("eng+ell"),
            "explicit inventories no longer keep supported ocr tokens in order"
        );
    }

    /// Explicit inventories drop unsupported OCR tokens.
    #[test]
    fn explicit_inventories_drop_unsupported_ocr_tokens() {
        assert_eq!(
            resolved(
                String::from("eng+ell"),
                &[String::from("eng"), String::from("osd")]
            ),
            String::from("eng"),
            "explicit inventories no longer drop unsupported ocr tokens"
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
