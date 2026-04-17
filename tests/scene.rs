//! Tests for scene OCR routing and manga validation.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::rc::Rc;

use anyhow::{Result, bail};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use kamishibai::generation::manga::{
    BorderDetector, ImageSource, ImageText, MangaRenderer, Progress, Renderer, SceneText,
    TextDetector, TextDetectors,
};
use serde_json::{Value, json};

/// Fixed OCR detector for routing tests.
#[derive(Clone, Debug)]
struct FixedText {
    calls: Rc<RefCell<usize>>,
    value: String,
}

impl FixedText {
    /// Create one fixed OCR detector.
    fn new(value: &str) -> Self {
        Self {
            calls: Rc::new(RefCell::new(0)),
            value: String::from(value),
        }
    }
}

impl ImageText for FixedText {
    /// Return one fixed OCR payload.
    fn detected(&self, _image: &GrayImage) -> Result<String> {
        *self.calls.borrow_mut() += 1;
        Ok(self.value.clone())
    }
}

/// Scripted scene OCR for renderer tests.
#[derive(Clone, Debug)]
struct ScriptedText {
    values: Rc<RefCell<VecDeque<String>>>,
}

impl ScriptedText {
    /// Create one scripted scene OCR detector.
    fn new(values: &[&str]) -> Self {
        Self {
            values: Rc::new(RefCell::new(
                values.iter().map(|value| String::from(*value)).collect(),
            )),
        }
    }
}

impl SceneText for ScriptedText {
    /// Return the next scripted OCR payload.
    fn detected(&self, _scene: &Value, _image: &GrayImage) -> Result<String> {
        Ok(self.values.borrow_mut().pop_front().unwrap_or_default())
    }
}

/// Scripted image source for renderer tests.
#[derive(Clone, Debug)]
struct QueueSource {
    values: Rc<RefCell<VecDeque<GrayImage>>>,
}

impl QueueSource {
    /// Create one scripted image source.
    fn new(values: Vec<GrayImage>) -> Self {
        Self {
            values: Rc::new(RefCell::new(values.into())),
        }
    }
}

impl ImageSource for QueueSource {
    /// Return one scripted PNG payload.
    fn image(&self, _scene: &Value) -> Result<Vec<u8>> {
        let Some(image) = self.values.borrow_mut().pop_front() else {
            bail!("image source ran out of scripted images");
        };
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image).write_to(&mut cursor, ImageFormat::Png)?;
        Ok(cursor.into_inner())
    }
}

/// Retry recorder for renderer tests.
#[derive(Clone, Debug, Default)]
struct Recorder {
    retries: Vec<(String, usize, String)>,
}

impl Progress for Recorder {
    /// Ignore progress step events in renderer tests.
    fn step(&mut self, _name: &str) {}

    /// Ignore progress completion events in renderer tests.
    fn done(&mut self, _name: &str, _label: &str, _path: Option<&std::path::Path>) {}

    /// Record retry events for later inspection.
    fn retry(&mut self, name: &str, attempt: usize, reason: &str) {
        self.retries
            .push((String::from(name), attempt, String::from(reason)));
    }
}

/// Create one scene document with the requested panel count and target language.
fn scene(panels: usize, target: &str) -> Value {
    json!({
        "manga_panel": {
            "meta": {
                "target_lang": target
            },
            "panels": (0..panels)
                .map(|index| json!({"id": format!("panel-{index}")}))
                .collect::<Vec<_>>()
        }
    })
}

/// Create one white image with a dark center.
fn framed(size: u32, margin: u32) -> GrayImage {
    let mut image = GrayImage::from_pixel(size, size, Luma([0]));
    for y in 0..size {
        for x in 0..size {
            if x < margin || y < margin || x >= size - margin || y >= size - margin {
                image.put_pixel(x, y, Luma([255]));
            }
        }
    }
    image
}

/// Create one white image with a white gutter across the center.
fn guttered(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = framed(size, margin);
    let top = (size - gutter) / 2;
    for y in top..(top + gutter) {
        for x in 0..size {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// A detector keeps every requested installed language.
#[test]
fn listed_detector_keeps_every_requested_installed_language() {
    assert_eq!(
        TextDetector::listed(50, "eng+ell", ["eng", "ell", "osd"]).selection(),
        "eng+ell",
        "listed detector no longer keeps every requested installed language"
    );
}

/// A detector drops languages that are not installed.
#[test]
fn listed_detector_drops_languages_that_are_not_installed() {
    assert_eq!(
        TextDetector::listed(50, "eng+ell", ["eng", "osd"]).selection(),
        "eng",
        "listed detector no longer drops languages that are not installed"
    );
}

/// A detector falls back to English when nothing is installed.
#[test]
fn listed_detector_falls_back_to_english_when_nothing_is_installed() {
    assert_eq!(
        TextDetector::listed(50, "missing", ["osd"]).selection(),
        "eng",
        "listed detector no longer falls back to English when nothing is installed"
    );
}

/// Scene OCR routing picks the detector for the target language.
#[test]
fn scene_ocr_routing_picks_the_detector_for_the_target_language() -> Result<()> {
    let english = FixedText::new("english");
    let greek = FixedText::new("greek");
    let detectors = TextDetectors::new(
        BTreeMap::from([
            (String::from("en"), english.clone()),
            (String::from("el"), greek.clone()),
        ]),
        FixedText::new("fallback"),
    );
    assert_eq!(
        detectors.detected(&scene(1, "el"), &GrayImage::from_pixel(4, 4, Luma([255])))?,
        String::from("greek"),
        "scene OCR routing no longer picks the detector for the target language"
    );
    Ok(())
}

/// Scene OCR routing falls back when the target language is unknown.
#[test]
fn scene_ocr_routing_falls_back_when_the_target_language_is_unknown() -> Result<()> {
    let detectors = TextDetectors::new(
        BTreeMap::from([(String::from("en"), FixedText::new("english"))]),
        FixedText::new("fallback"),
    );
    assert_eq!(
        detectors.detected(&scene(1, "ga"), &GrayImage::from_pixel(4, 4, Luma([255])))?,
        String::from("fallback"),
        "scene OCR routing no longer falls back when the target language is unknown"
    );
    Ok(())
}

/// The border detector reports dark edges.
#[test]
fn border_detector_reports_dark_edges() {
    assert_eq!(
        BorderDetector::new(2, 240, 2).borders(&GrayImage::from_pixel(12, 12, Luma([0]))),
        vec![
            String::from("top"),
            String::from("bottom"),
            String::from("left"),
            String::from("right"),
        ],
        "border detector no longer reports dark edges"
    );
}

/// The border detector finds white gutter runs.
#[test]
fn border_detector_finds_white_gutter_runs() {
    assert!(
        BorderDetector::new(2, 240, 1).gutter(&guttered(12, 1, 2)),
        "border detector no longer finds white gutter runs"
    );
}

/// The renderer retries when OCR text is detected before a valid frame appears.
#[test]
fn renderer_retries_when_ocr_text_is_detected_before_a_valid_frame_appears() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![guttered(32, 1, 2), guttered(32, 1, 2)]),
        2,
        ScriptedText::new(&["слово", ""]),
        BorderDetector::new(2, 240, 1),
    );
    let mut progress = Recorder::default();
    assert_eq!(
        (
            renderer
                .render(&scene(2, "ru"), &mut progress)?
                .color()
                .has_color(),
            progress.retries,
        ),
        (
            false,
            vec![(
                String::from("Rendering manga"),
                1,
                String::from("OCR detected text: 'слово'"),
            )],
        ),
        "renderer no longer retries when OCR text is detected before a valid frame appears"
    );
    Ok(())
}

/// The renderer rejects a frame after the last missing-border attempt.
#[test]
fn renderer_rejects_a_frame_after_the_last_missing_border_attempt() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(16, 16, Luma([0]))]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 240, 2),
    );
    assert_eq!(
        renderer
            .render(&scene(1, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from(
            "Rejected after 1 attempts: White border missing on: top, bottom, left, right"
        ),
        "renderer no longer rejects a frame after the last missing border attempt"
    );
}

/// The renderer rejects a multi-panel frame when no gutter appears.
#[test]
fn renderer_rejects_a_multi_panel_frame_when_no_gutter_appears() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![framed(16, 1)]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&scene(2, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: No white gutter found"),
        "renderer no longer rejects a multi panel frame when no gutter appears"
    );
}
