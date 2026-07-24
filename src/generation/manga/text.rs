use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use image::{DynamicImage, GrayImage};
use ocr_rs::OcrResult_;
use serde_json::Value;

use crate::generation::manga::ocr_bundle as ocr;

use super::contracts::{ImageText, SceneText};
use super::redirect::{discarded, hush, locked};

const DEFAULT_LANG: &str = "eng";
type Lazy = Rc<QuietEngine>;

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

/// Combine multiple OCR recognizers so every configured script can reject visible writing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEnsemble<D> {
    detectors: Vec<D>,
}

impl<D> TextEnsemble<D> {
    /// Create one OCR ensemble from independent recognizers.
    pub fn new(detectors: Vec<D>) -> Self {
        Self { detectors }
    }
}

impl<D> ImageText for TextEnsemble<D>
where
    D: ImageText,
{
    /// Return every nonempty recognizer result in configured order.
    fn detected(&self, image: &GrayImage) -> Result<String> {
        Ok(self
            .detectors
            .iter()
            .map(|detector| detector.detected(image))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "))
    }
}

impl<D> SceneText for TextEnsemble<D>
where
    D: ImageText,
{
    /// Return every nonempty recognizer result for one scene image.
    fn detected(&self, _scene: &Value, image: &GrayImage) -> Result<String> {
        ImageText::detected(self, image)
    }
}

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

impl SceneText for TextDetector {
    /// Return the detected OCR text for one scene and image pair.
    fn detected(&self, _scene: &Value, image: &GrayImage) -> Result<String> {
        ImageText::detected(self, image)
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

/// Return the resolved OCR language selection for one explicit inventory.
pub(super) fn resolved(lang: String, available: &[String]) -> String {
    let supported = lang
        .split('+')
        .filter(|code| available.iter().any(|item| item == *code))
        .collect::<Vec<_>>();
    if supported.is_empty() {
        return String::from(DEFAULT_LANG);
    }
    supported.join("+")
}
