//! Tests for scene OCR routing and manga validation.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::rc::Rc;

use anyhow::{Result, bail};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use kamishibai::generation::manga::{
    BorderDetector, ImageSource, ImageText, MangaRenderer, Progress, Renderer, SceneText,
    TextDetector, TextDetectors, TextEnsemble,
};
use serde_json::{Value, json};
use tempfile::tempdir;

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

/// Create one scene document with an explicit page device.
fn device_scene(panels: usize, target: &str, device: &str) -> Value {
    let mut value = scene(panels, target);
    value["manga_panel"]["page_design"] = json!({
        "special_device": {
            "kind": device
        }
    });
    value
}

/// Create one scene document carrying a materialized registry layout.
fn active_layout_scene(panels: usize, target: &str) -> Value {
    let mut value = scene(panels, target);
    value["manga_panel"]["canvas"] = json!({
        "width": 1024,
        "height": 1024
    });
    value["manga_panel"]["panels"] = Value::Array(match panels {
        1 => vec![json!({
            "id": "p1",
            "bounds": {"x": 32, "y": 32, "width": 960, "height": 960},
            "frame": {"shape": "rectangle"}
        })],
        2 => vec![
            json!({
                "id": "p1",
                "bounds": {"x": 32, "y": 32, "width": 448, "height": 960},
                "frame": {"shape": "tall_rectangle"}
            }),
            json!({
                "id": "p2",
                "bounds": {"x": 544, "y": 32, "width": 448, "height": 960},
                "frame": {"shape": "tall_rectangle"}
            }),
        ],
        _ => panic!("invariant: active layout test supports one or two panels"),
    });
    value["manga_panel"]["panel_layout"] = json!({
        "active_layout": {
            "template_id": "test-rectangles-v1"
        }
    });
    value
}

/// Create one canonical T-bottom scene whose visible separator may shift vertically.
fn t_bottom_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {"id": "p1", "bounds": {"x": 16, "y": 16, "width": 480, "height": 376}, "frame": {"shape": "rectangle"}},
        {"id": "p2", "bounds": {"x": 512, "y": 16, "width": 496, "height": 376}, "frame": {"shape": "rectangle"}},
        {"id": "p3", "bounds": {"x": 16, "y": 408, "width": 992, "height": 600}, "frame": {"shape": "wide_rectangle"}}
    ]);
    value
}

/// Create one slanted T-bottom scene whose first panel crosses into the payoff.
fn slanted_crossing_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {
            "id": "p1",
            "bounds": {"x": 16, "y": 16, "width": 400, "height": 376},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[16, 16], [416, 16], [352, 392], [16, 392]]
            }
        },
        {
            "id": "p2",
            "bounds": {"x": 368, "y": 16, "width": 640, "height": 376},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[432, 16], [1008, 16], [1008, 392], [368, 392]]
            }
        },
        {
            "id": "p3",
            "bounds": {"x": 16, "y": 408, "width": 992, "height": 600},
            "frame": {"shape": "wide_rectangle"}
        }
    ]);
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("slanted-t-bottom-3-p2-v1");
    let mut value = active_device_scene(value, "crossing");
    value["manga_panel"]["page_design"]["special_device"]["target_panel"] = json!("p3");
    value["manga_panel"]["panels"][0]["continuity"]["breakout"]["destination_panel"] = json!("p3");
    value
}

/// Create one slanted left rail whose second beat leads into a dominant right panel.
fn slanted_rail_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {
            "id": "p1",
            "bounds": {"x": 16, "y": 16, "width": 300, "height": 432},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[16, 16], [316, 16], [316, 448], [16, 384]]
            }
        },
        {
            "id": "p2",
            "bounds": {"x": 16, "y": 400, "width": 300, "height": 608},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[16, 400], [316, 464], [316, 1008], [16, 1008]]
            }
        },
        {
            "id": "p3",
            "bounds": {"x": 332, "y": 16, "width": 676, "height": 992},
            "frame": {"shape": "wide_rectangle"}
        }
    ]);
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("slanted-dominant-rail-3-p2-v1");
    value["manga_panel"]["page_design"]["special_device"] = json!({
        "kind": "none",
        "source_panel": "",
        "target_panel": "",
        "subject_id": ""
    });
    value
}

/// Create one declared three-column scene whose panel centers share a horizontal line.
fn horizontal_triptych_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {"id": "p1", "bounds": {"x": 16, "y": 16, "width": 320, "height": 992}, "frame": {"shape": "rectangle"}},
        {"id": "p2", "bounds": {"x": 352, "y": 16, "width": 320, "height": 992}, "frame": {"shape": "rectangle"}},
        {"id": "p3", "bounds": {"x": 688, "y": 16, "width": 320, "height": 992}, "frame": {"shape": "rectangle"}}
    ]);
    value
}

/// Create one declared two-panel page whose canonical gutter slopes down-right.
fn diagonal_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {
            "id": "p1",
            "bounds": {"x": 16, "y": 16, "width": 600, "height": 992},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[16, 16], [520, 16], [616, 1008], [16, 1008]]
            }
        },
        {
            "id": "p2",
            "bounds": {"x": 536, "y": 16, "width": 472, "height": 992},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[536, 16], [1008, 16], [1008, 1008], [632, 1008]]
            }
        }
    ]);
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("test-diagonal-2-v1");
    value
}

/// Attach one fully materialized special device to a registry scene.
fn active_device_scene(mut value: Value, device: &str) -> Value {
    let panels = value["manga_panel"]["panels"]
        .as_array_mut()
        .expect("invariant: device test panels must be an array");
    for panel in panels.iter_mut() {
        panel["frame"]["border"] = json!("solid");
        panel["frame"]["z_index"] = json!(0);
        panel["frame"]["parent_panel"] = json!("");
        panel["frame"]["overlaps_panel"] = json!("");
        panel["continuity"]["breakout"] = json!({
            "enabled": false,
            "subject_id": "",
            "edge": "empty",
            "destination_panel": ""
        });
    }
    let (target, subject) = match device {
        "open_frame" => ("", ""),
        "crossing" => ("p2", "actor"),
        _ => ("p2", ""),
    };
    value["manga_panel"]["page_design"] = json!({
        "special_device": {
            "kind": device,
            "source_panel": "p1",
            "target_panel": target,
            "subject_id": subject
        }
    });
    match device {
        "crossing" => {
            value["manga_panel"]["panels"][0]["continuity"]["breakout"] = json!({
                "enabled": true,
                "subject_id": "actor",
                "edge": "right",
                "destination_panel": "p2"
            });
        }
        "overlap" => {
            value["manga_panel"]["panels"][0]["bounds"]["width"] = json!(576);
            value["manga_panel"]["panels"][0]["frame"]["overlaps_panel"] = json!("p2");
            value["manga_panel"]["panels"][0]["frame"]["z_index"] = json!(1);
        }
        "inset" => {
            value["manga_panel"]["panels"][0]["bounds"] =
                json!({"x": 16, "y": 16, "width": 800, "height": 376});
            value["manga_panel"]["panels"][1]["bounds"] =
                json!({"x": 588, "y": 40, "width": 200, "height": 150});
            value["manga_panel"]["panels"][1]["frame"]["shape"] = json!("inset");
            value["manga_panel"]["panels"][1]["frame"]["parent_panel"] = json!("p1");
            value["manga_panel"]["panels"][1]["frame"]["z_index"] = json!(1);
        }
        "open_frame" => {
            value["manga_panel"]["panels"][0]["frame"]["shape"] = json!("open_frame");
            value["manga_panel"]["panels"][0]["frame"]["border"] = json!("none");
        }
        _ => {}
    }
    value
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

/// Create one white-framed image with a vertical white gutter through the center.
fn vertically_guttered(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = framed(size, margin);
    let left = (size - gutter) / 2;
    for y in 0..size {
        for x in left..(left + gutter) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create one canonical two-panel image with no gutter inside either panel.
fn rectangular_panels() -> GrayImage {
    let mut image = framed(32, 1);
    for y in 0..32 {
        for x in 15..17 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create two closed regions separated by a diagonal white gutter.
fn diagonal_panels() -> GrayImage {
    let mut image = framed(32, 1);
    for y in 0u32..32 {
        let left = 7u32.saturating_add(y / 2);
        for x in left..left.saturating_add(2).min(32) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create two regions matching the declared mild down-right diagonal.
fn declared_diagonal_panels() -> GrayImage {
    let mut image = framed(64, 1);
    for y in 0u32..64 {
        let left = 32u32.saturating_add(y.saturating_mul(6) / 63);
        for x in left..left.saturating_add(2).min(64) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create an undeclared third region inside the first declared diagonal panel.
fn extra_diagonal_panel() -> GrayImage {
    let mut image = declared_diagonal_panels();
    for y in 31..33 {
        for x in 0..38 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create one crossed T page with either a canonical slanted or straight upper divider.
fn crossed_slanted_t_bottom_panels(slanted: bool) -> GrayImage {
    let mut image = framed(128, 1);
    for y in 0u32..51 {
        let left = if slanted {
            52u32.saturating_sub(y.saturating_mul(8) / 50)
        } else {
            48
        };
        for x in left..left.saturating_add(2).min(128) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 49..51 {
        for x in 0..128 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 49..51 {
        for x in 10..18 {
            image.put_pixel(x, y, Luma([0]));
        }
    }
    image
}

/// Create one ordinary slanted T page whose upper separator is steeper than declared.
fn shifted_slanted_t_bottom_panels() -> GrayImage {
    let mut image = framed(128, 1);
    for y in 0u32..51 {
        let left = 65u32.saturating_sub(y.saturating_mul(19) / 50);
        for x in left..left.saturating_add(2).min(128) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 49..51 {
        for x in 0..128 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create one three-panel left rail with a slanted or straight internal divider.
fn slanted_rail_panels(slanted: bool) -> GrayImage {
    let mut image = framed(128, 1);
    for y in 0..128 {
        for x in 40..42 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for x in 0u32..42 {
        let top = if slanted {
            48u32.saturating_add(x.saturating_mul(8) / 41)
        } else {
            52
        };
        for y in top..top.saturating_add(2).min(128) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create a missing panel boundary with an unrelated near-white contour away from its corridor.
fn unrelated_near_white_contour() -> GrayImage {
    let mut image = framed(128, 1);
    for y in 0u32..51 {
        let left = 52u32.saturating_sub(y.saturating_mul(8) / 50);
        for x in left..left.saturating_add(2).min(128) {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 49..51 {
        for x in 0..46 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 49..72 {
        for x in 44..46 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 70..72 {
        for x in 44..128 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 70..72 {
        image.put_pixel(60, y, Luma([0]));
    }
    image
}

fn expanded_white(image: &GrayImage, threshold: u8) -> GrayImage {
    let mut expanded = image.clone();
    for y in 0..image.height() {
        for x in 0..image.width() {
            let left = x.saturating_sub(1);
            let top = y.saturating_sub(1);
            let right = x.saturating_add(1).min(image.width().saturating_sub(1));
            let bottom = y.saturating_add(1).min(image.height().saturating_sub(1));
            if (top..=bottom).any(|other_y| {
                (left..=right).any(|other_x| image.get_pixel(other_x, other_y)[0] >= threshold)
            }) {
                expanded.put_pixel(x, y, Luma([255]));
            }
        }
    }
    expanded
}

/// Create one three-panel T page whose main separator is lower than canonical coordinates.
fn shifted_t_bottom_panels() -> GrayImage {
    let mut image = framed(32, 1);
    for y in 0..17 {
        for x in 15..17 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 15..17 {
        for x in 0..32 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Create one three-panel T page whose bottom region contains every triptych center.
fn upper_t_bottom_panels() -> GrayImage {
    let mut image = framed(32, 1);
    for y in 0..13 {
        for x in 15..17 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 11..13 {
        for x in 0..32 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    image
}

/// Join only the declared top crossing pair while preserving the bottom panel.
fn crossed_t_bottom_panels() -> GrayImage {
    let mut image = shifted_t_bottom_panels();
    for y in 6..10 {
        for x in 15..17 {
            image.put_pixel(x, y, Luma([0]));
        }
    }
    image
}

/// Join the first top panel to the unrelated bottom panel.
fn wrongly_crossed_t_bottom_panels() -> GrayImage {
    let mut image = shifted_t_bottom_panels();
    for y in 15..17 {
        for x in 6..10 {
            image.put_pixel(x, y, Luma([0]));
        }
    }
    image
}

/// Create one white-framed image with a light content band and no dark frame rails.
fn light_banded(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = GrayImage::from_pixel(size, size, Luma([192]));
    let top = (size - gutter) / 2;
    for y in 0..size {
        for x in 0..size {
            if x < margin
                || y < margin
                || x >= size - margin
                || y >= size - margin
                || (y >= top && y < top + gutter)
            {
                image.put_pixel(x, y, Luma([255]));
            }
        }
    }
    image
}

/// Create one white image containing one large empty solid frame.
fn blank_panel(size: u32, margin: u32) -> GrayImage {
    let mut image = GrayImage::from_pixel(size, size, Luma([255]));
    for point in margin..(size - margin) {
        image.put_pixel(point, margin, Luma([0]));
        image.put_pixel(point, size - margin - 1, Luma([0]));
        image.put_pixel(margin, point, Luma([0]));
        image.put_pixel(size - margin - 1, point, Luma([0]));
    }
    image
}

/// Create one anti-aliased horizontal gutter with solid frame rails.
fn antialiased_gutter(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = framed(size, margin);
    let top = (size - gutter) / 2;
    for y in top..(top + gutter) {
        for x in 0..size {
            image.put_pixel(x, y, Luma([248]));
        }
        image.put_pixel(size / 3, y, Luma([235]));
    }
    image
}

/// Create one anti-aliased vertical gutter with solid frame rails.
fn antialiased_vertical_gutter(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = framed(size, margin);
    let left = (size - gutter) / 2;
    for x in left..(left + gutter) {
        for y in 0..size {
            image.put_pixel(x, y, Luma([248]));
        }
        image.put_pixel(x, size / 3, Luma([235]));
    }
    image
}

/// Create one nearly white band interrupted by dark image content.
fn marked_band(size: u32, margin: u32, gutter: u32) -> GrayImage {
    let mut image = guttered(size, margin, gutter);
    let top = (size - gutter) / 2;
    for y in top..(top + gutter) {
        image.put_pixel(size / 3, y, Luma([0]));
        image.put_pixel((size * 2) / 3, y, Luma([0]));
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

/// OCR ensembles preserve every script recognizer result for production validation.
#[test]
fn text_ensemble_runs_every_script_recognizer() -> Result<()> {
    let latin = FixedText::new("coordinates");
    let japanese = FixedText::new("ドン");
    let ensemble = TextEnsemble::new(vec![latin.clone(), japanese.clone()]);
    let detected = ImageText::detected(&ensemble, &GrayImage::from_pixel(4, 4, Luma([255])))?;
    assert_eq!(
        (detected, *latin.calls.borrow(), *japanese.calls.borrow(),),
        (String::from("coordinates ドン"), 1, 1),
        "OCR ensemble skipped one configured script recognizer"
    );
    Ok(())
}

/// The border detector reports dark edges.
#[test]
fn border_detector_reports_dark_edges() {
    assert_eq!(
        BorderDetector::new(2, 6, 240, 2).borders(&GrayImage::from_pixel(12, 12, Luma([0]))),
        vec![
            String::from("top"),
            String::from("bottom"),
            String::from("left"),
            String::from("right"),
        ],
        "border detector no longer reports dark edges"
    );
}

/// The border detector does not mistake the outer frame for a gutter.
#[test]
fn border_detector_does_not_count_the_outer_frame_as_a_gutter() {
    assert!(
        !BorderDetector::new(2, 6, 240, 2).gutter(&framed(16, 4)),
        "border detector mistakes the outer frame for a gutter"
    );
}

/// The border detector finds horizontal white gutter runs.
#[test]
fn border_detector_finds_horizontal_white_gutter_runs() {
    assert!(
        BorderDetector::new(2, 6, 240, 1).gutter(&guttered(12, 1, 2)),
        "border detector no longer finds horizontal white gutter runs"
    );
}

/// The border detector finds vertical white gutter runs.
#[test]
fn border_detector_finds_vertical_white_gutter_runs() {
    assert!(
        BorderDetector::new(2, 6, 240, 1).gutter(&vertically_guttered(12, 1, 2)),
        "border detector no longer finds vertical white gutter runs"
    );
}

/// The border detector rejects a light content band without frame rails.
#[test]
fn border_detector_does_not_count_a_light_content_band_as_a_gutter() {
    assert!(
        !BorderDetector::new(2, 6, 240, 2).gutter(&light_banded(32, 2, 4)),
        "border detector mistakes a light content band for a gutter"
    );
}

/// The border detector rejects one empty panel whose white interior is too wide.
#[test]
fn border_detector_does_not_count_a_blank_panel_as_a_gutter() {
    assert!(
        !BorderDetector::new(2, 6, 240, 2).gutter(&blank_panel(32, 3)),
        "border detector mistakes a blank panel for a gutter"
    );
}

/// The border detector accepts a nearly white anti-aliased gutter.
#[test]
fn border_detector_finds_an_antialiased_gutter() {
    assert!(
        BorderDetector::new(4, 8, 240, 1).gutter(&antialiased_gutter(128, 1, 4)),
        "border detector rejects an anti-aliased gutter"
    );
}

/// The border detector accepts a nearly white anti-aliased vertical gutter.
#[test]
fn border_detector_finds_an_antialiased_vertical_gutter() {
    assert!(
        BorderDetector::new(4, 8, 240, 1).gutter(&antialiased_vertical_gutter(128, 1, 4)),
        "border detector rejects an anti-aliased vertical gutter"
    );
}

/// The border detector does not average dark image content out of a white band.
#[test]
fn border_detector_does_not_average_dark_content_out_of_a_white_band() {
    assert!(
        !BorderDetector::new(4, 8, 240, 1).gutter(&marked_band(128, 1, 4)),
        "border detector averages dark image content out of a white band"
    );
}

/// The production topology detector counts closed regions independently of separator geometry.
#[test]
fn border_detector_counts_closed_panel_regions() {
    let detector = BorderDetector::new(2, 6, 240, 1);
    assert_eq!(
        [
            detector.regions(&framed(32, 1)),
            detector.regions(&rectangular_panels()),
            detector.regions(&shifted_t_bottom_panels()),
            detector.regions(&crossed_t_bottom_panels()),
            detector.regions(&wrongly_crossed_t_bottom_panels()),
        ],
        [1, 2, 3, 2, 2],
        "topology detector no longer counts geometry independent panel regions"
    );
}

/// The renderer retries when OCR text is detected before a valid frame appears.
#[test]
fn renderer_retries_when_ocr_text_is_detected_before_a_valid_frame_appears() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![guttered(32, 1, 2), guttered(32, 1, 2)]),
        2,
        ScriptedText::new(&["слово", ""]),
        BorderDetector::new(2, 6, 240, 1),
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
        BorderDetector::new(2, 6, 240, 2),
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
        BorderDetector::new(2, 6, 240, 1),
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

/// The renderer accepts expressive geometry without a straight gutter while retaining validation.
#[test]
fn renderer_accepts_expressive_geometry_while_retaining_ocr_and_border_validation() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            framed(32, 1),
            GrayImage::from_pixel(32, 32, Luma([0])),
            framed(32, 1),
        ]),
        3,
        ScriptedText::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(&device_scene(2, "en", "overlap"), &mut progress)
        .expect("expressive layout must render after OCR and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from("OCR detected text: 'word'"),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("White border missing on: top, bottom, left, right"),
                ),
            ],
        ),
        "expressive geometry bypasses OCR or outer border validation"
    );
}

/// The renderer keeps straight-gutter validation for explicit ordinary layouts.
#[test]
fn renderer_rejects_explicit_ordinary_layouts_without_a_gutter() {
    let reasons = ["none", "master_view"].map(|device| {
        MangaRenderer::new(
            QueueSource::new(vec![framed(16, 1)]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&device_scene(2, "en", device), &mut Recorder::default())
        .expect_err("ordinary multi-panel layout without a gutter must be rejected")
        .to_string()
    });
    assert_eq!(
        reasons,
        [
            String::from("Rejected after 1 attempts: No white gutter found"),
            String::from("Rejected after 1 attempts: No white gutter found"),
        ],
        "ordinary page devices no longer require a straight gutter"
    );
}

/// Registry multi-panel geometry retains topology, OCR, and outer-border checks.
#[test]
fn renderer_accepts_registry_geometry_while_retaining_ocr_and_border_validation() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            framed(32, 1),
            GrayImage::from_pixel(32, 32, Luma([0])),
            rectangular_panels(),
        ]),
        3,
        ScriptedText::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(&active_layout_scene(2, "en"), &mut progress)
        .expect("registry layout must render after OCR and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from("OCR detected text: 'word'"),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("White border missing on: top, bottom, left, right"),
                ),
            ],
        ),
        "registry geometry bypasses topology, OCR, or outer border validation"
    );
}

/// Registry validation rejects one splash when the selected layout requires multiple panels.
#[test]
fn renderer_rejects_registry_multi_panel_scene_rendered_as_one_splash() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![framed(32, 1)]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&active_layout_scene(2, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        "registry renderer still accepts one splash for a multi panel selection"
    );
}

/// Production OCR keeps a short Latin hallucination as audit noise instead of rejecting artwork.
#[test]
fn renderer_accepts_short_latin_ocr_noise_for_registry_artwork() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedText::new(&["un un"]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    assert_eq!(
        (
            renderer
                .render(&active_layout_scene(2, "en"), &mut progress)?
                .color()
                .has_color(),
            progress.retries,
        ),
        (false, Vec::new()),
        "short Latin OCR noise still rejects clean registry artwork"
    );
    Ok(())
}

/// Production geometry accepts the right topology when a model shifts one separator.
#[test]
fn renderer_accepts_shifted_separator_in_a_multi_panel_registry_page() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![shifted_t_bottom_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    assert_eq!(
        (
            renderer
                .render(&t_bottom_layout_scene("en"), &mut progress)?
                .color()
                .has_color(),
            progress.retries,
        ),
        (false, Vec::new()),
        "shifted canonical separator is still mistaken for an extra panel"
    );
    Ok(())
}

/// A slanted separator may move locally while retaining its direction and three regions.
#[test]
fn renderer_accepts_a_steeper_slanted_separator_with_exact_topology() -> Result<()> {
    let mut scene = slanted_crossing_layout_scene("en");
    scene["manga_panel"]["page_design"]["special_device"] = json!({
        "kind": "none",
        "source_panel": "",
        "target_panel": "",
        "subject_id": ""
    });
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![shifted_slanted_t_bottom_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let rendered = renderer.render(&scene, &mut Recorder::default())?;
    assert!(
        !rendered.color().has_color(),
        "steeper slanted separator is still rejected despite retaining exact topology"
    );
    Ok(())
}

/// The emphasis rail requires its declared horizontal divider to retain a real slope.
#[test]
fn renderer_accepts_slanted_rail_and_rejects_straightened_rail() {
    let accepted = [true, false].map(|slanted| {
        MangaRenderer::new(
            QueueSource::new(vec![slanted_rail_panels(slanted)]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&slanted_rail_layout_scene("en"), &mut Recorder::default())
        .is_ok()
    });
    assert_eq!(
        accepted,
        [true, false],
        "emphasis rail loses its horizontal slant contract"
    );
}

/// Local bridge repair cannot promote a white contour outside the merged pair corridor.
#[test]
fn renderer_rejects_unrelated_near_white_contour_outside_gutter_corridor() {
    let image = unrelated_near_white_contour();
    let detector = BorderDetector::new(2, 6, 240, 1);
    let regions = detector.regions(&image);
    let globally_expanded = detector.regions(&expanded_white(&image, 250));
    let rejected = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedText::new(&[""]),
        detector,
    )
    .render(
        &{
            let mut scene = slanted_crossing_layout_scene("en");
            scene["manga_panel"]["page_design"]["special_device"] = json!({
                "kind": "none",
                "source_panel": "",
                "target_panel": "",
                "subject_id": ""
            });
            scene
        },
        &mut Recorder::default(),
    )
    .is_err();
    assert_eq!(
        (regions, globally_expanded, rejected),
        (2, 3, true),
        "unrelated white contour still manufactures a missing panel region"
    );
}

/// An open-frame declaration cannot stand in for a missing source panel.
#[test]
fn renderer_rejects_open_frame_with_a_blank_source_region() {
    let mut image = shifted_t_bottom_panels();
    for y in 1..15 {
        for x in 1..15 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(
                &active_device_scene(t_bottom_layout_scene("en"), "open_frame"),
                &mut Recorder::default(),
            )
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        "open frame accepts a blank source as one semantic panel"
    );
}

/// Registry geometry rejects the right region count when it realizes a different layout.
#[test]
fn renderer_rejects_t_grid_for_a_declared_horizontal_triptych() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![upper_t_bottom_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(
                &horizontal_triptych_layout_scene("en"),
                &mut Recorder::default(),
            )
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        "registry renderer accepts a T grid for a declared horizontal triptych"
    );
}

/// A diagonal diptych requires exactly two regions and multiple slope witnesses per panel.
#[test]
fn renderer_rejects_extra_panel_and_straight_split_for_declared_diagonal() {
    let extra = extra_diagonal_panel();
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&extra);
    let rejected = [extra, rectangular_panels(), declared_diagonal_panels()].map(|image| {
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&diagonal_layout_scene("en"), &mut Recorder::default())
        .is_err()
    });
    assert_eq!(
        (regions, rejected),
        (3, [true, true, false]),
        "registry renderer accepts an extra panel or straightens a declared diagonal"
    );
}

/// The strong diagonal emphasis template keeps its divider oblique on the tolerant path.
#[test]
fn renderer_accepts_strong_diagonal_and_rejects_straightened_split() {
    let mut scene = diagonal_layout_scene("en");
    scene["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("diagonal-split-2-end-strong-v1");
    scene["manga_panel"]["page_design"]["special_device"] = json!({
        "kind": "none",
        "source_panel": "",
        "target_panel": "",
        "subject_id": ""
    });
    let accepted = [declared_diagonal_panels(), rectangular_panels()].map(|image| {
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&scene, &mut Recorder::default())
        .is_ok()
    });
    assert_eq!(
        accepted,
        [true, false],
        "strong diagonal emphasis accepts a straightened two-region split"
    );
}

/// Production validation archives every accepted and rejected raw image attempt.
#[test]
fn renderer_archives_registry_image_attempts_with_verdicts() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels(), rectangular_panels()]),
        2,
        ScriptedText::new(&["word", ""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let second = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            temporary.path().join("attempt-0001.png").is_file(),
            temporary.path().join("attempt-0002.png").is_file(),
            first["status"].as_str(),
            first["reason"].as_str(),
            second["status"].as_str(),
        ),
        (
            false,
            true,
            true,
            Some("rejected"),
            Some("OCR detected text: 'word'"),
            Some("accepted"),
        ),
        "production image attempts or their validation verdicts were discarded"
    );
    Ok(())
}

/// Registry one-panel geometry retries an image that contains an internal gutter.
#[test]
fn renderer_retries_registry_one_panel_geometry_with_an_internal_gutter() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![guttered(32, 1, 2), framed(32, 1)]),
        2,
        ScriptedText::new(&["", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    assert_eq!(
        (
            renderer
                .render(&active_layout_scene(1, "en"), &mut progress)?
                .color()
                .has_color(),
            progress.retries,
        ),
        (
            false,
            vec![(
                String::from("Rendering manga"),
                1,
                String::from("Unexpected internal gutter in one-panel layout"),
            )],
        ),
        "registry one-panel geometry no longer retries an internal gutter"
    );
    Ok(())
}

/// Registry one-panel geometry rejects a diagonal split that has no straight gutter.
#[test]
fn renderer_rejects_registry_one_panel_geometry_split_diagonally() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![diagonal_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&active_layout_scene(1, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Unexpected internal gutter in one-panel layout"),
        "registry renderer still accepts a diagonal diptych as one splash"
    );
}

/// Every structural device requires its complete materialized relation and valid topology.
#[test]
fn renderer_accepts_fully_materialized_structural_devices_with_valid_topology() {
    let accepted = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        let image = match device {
            "crossing" => crossed_t_bottom_panels(),
            _ => shifted_t_bottom_panels(),
        };
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), device),
            &mut Recorder::default(),
        )
        .is_ok()
    });
    assert_eq!(
        accepted,
        [true, true, true, true],
        "fully materialized structural devices cannot retain exact topology"
    );
}

/// A crossing must visibly connect its declared pair instead of retaining an ordinary grid.
#[test]
fn renderer_rejects_crossing_with_an_unbroken_gutter() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![shifted_t_bottom_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(
                &active_device_scene(t_bottom_layout_scene("en"), "crossing"),
                &mut Recorder::default(),
            )
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        "crossing accepts an ordinary grid without a visible panel connection"
    );
}

/// Crossing preserves the declared slant even when its merged region count still matches.
#[test]
fn renderer_rejects_straightened_slanted_crossing_with_same_region_count() {
    let images = [
        crossed_slanted_t_bottom_panels(false),
        crossed_slanted_t_bottom_panels(true),
    ];
    let detector = BorderDetector::new(2, 6, 240, 1);
    let regions = images.each_ref().map(|image| detector.regions(image));
    let rejected = images.map(|image| {
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(
            &slanted_crossing_layout_scene("en"),
            &mut Recorder::default(),
        )
        .is_err()
    });
    assert_eq!(
        (regions, rejected),
        ([2, 2], [true, false]),
        "crossing accepts a straight divider that contradicts its declared slanted layout"
    );
}

/// Crossing region proof reads canonical polygon arrays when a panel is trapezoidal.
#[test]
fn renderer_uses_polygon_centers_for_crossing_region_proof() {
    let mut scene = active_device_scene(t_bottom_layout_scene("en"), "crossing");
    scene["manga_panel"]["panels"][0]["frame"]["polygon"] =
        json!([[16, 16], [496, 16], [496, 392], [16, 392]]);
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![crossed_t_bottom_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert!(
        renderer.render(&scene, &mut Recorder::default()).is_ok(),
        "crossing region proof cannot decode canonical polygon coordinates"
    );
}

/// Crossing proof fails closed when a polygon center or bounds center leaves the canvas.
#[test]
fn renderer_rejects_crossing_anchors_outside_the_registry_canvas() {
    let mut polygon = active_device_scene(t_bottom_layout_scene("en"), "crossing");
    polygon["manga_panel"]["panels"][0]["frame"]["polygon"] =
        json!([[1024, 16], [1024, 16], [1024, 392], [1024, 392]]);
    let mut bounds = active_device_scene(t_bottom_layout_scene("en"), "crossing");
    bounds["manga_panel"]["panels"][0]["bounds"] =
        json!({"x": 1008, "y": 16, "width": 32, "height": 376});
    let rejected = [polygon, bounds].map(|scene| {
        MangaRenderer::new(
            QueueSource::new(vec![crossed_t_bottom_panels()]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&scene, &mut Recorder::default())
        .is_err()
    });
    assert_eq!(
        rejected,
        [true, true],
        "crossing proof clamps an out-of-canvas anchor into a valid image region"
    );
}

/// A valid device cannot excuse the disappearance of an unrelated panel region.
#[test]
fn renderer_rejects_unrelated_panel_loss_for_structural_devices() {
    let rejected = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        let image = if device == "crossing" {
            wrongly_crossed_t_bottom_panels()
        } else {
            rectangular_panels()
        };
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), device),
            &mut Recorder::default(),
        )
        .is_err()
    });
    assert_eq!(
        rejected,
        [true, true, true, true],
        "structural device still hides an unrelated missing panel region"
    );
}

/// Kind-only structural declarations never relax registry topology validation.
#[test]
fn renderer_rejects_structural_devices_without_materialized_relations() {
    let rejected = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        let mut scene = t_bottom_layout_scene("en");
        scene["manga_panel"]["page_design"] = json!({"special_device": {"kind": device}});
        MangaRenderer::new(
            QueueSource::new(vec![shifted_t_bottom_panels()]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&scene, &mut Recorder::default())
        .is_err()
    });
    assert_eq!(
        rejected,
        [true, true, true, true],
        "kind-only structural declaration still relaxes registry validation"
    );
}

/// A crossing still rejects OCR and a missing outer frame before accepting connected panels.
#[test]
fn renderer_keeps_ocr_and_outer_border_checks_for_crossing() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            crossed_t_bottom_panels(),
            GrayImage::from_pixel(32, 32, Luma([0])),
            crossed_t_bottom_panels(),
        ]),
        3,
        ScriptedText::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), "crossing"),
            &mut progress,
        )
        .expect("valid crossing must render after OCR and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from("OCR detected text: 'word'"),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("White border missing on: top, bottom, left, right"),
                ),
            ],
        ),
        "crossing bypasses OCR or outer border validation"
    );
}

/// A structural device cannot erase more than its single declared panel relation.
#[test]
fn renderer_rejects_two_missing_regions_for_structural_devices() {
    let reasons = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        MangaRenderer::new(
            QueueSource::new(vec![framed(32, 1)]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), device),
            &mut Recorder::default(),
        )
        .expect_err("structural device with two missing regions must be rejected")
        .to_string()
    });
    assert_eq!(
        reasons,
        [
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        ],
        "structural devices erase more than one declared panel relation"
    );
}

/// Content continuity and diagonal flow retain every declared panel region.
#[test]
fn renderer_keeps_exact_topology_for_non_merging_devices() {
    let reasons = ["none", "master_view", "diagonal_release"].map(|device| {
        MangaRenderer::new(
            QueueSource::new(vec![framed(32, 1)]),
            1,
            ScriptedText::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(
            &active_device_scene(active_layout_scene(2, "en"), device),
            &mut Recorder::default(),
        )
        .expect_err("non-merging device without every panel region must be rejected")
        .to_string()
    });
    assert_eq!(
        reasons,
        [
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
            String::from("Rejected after 1 attempts: Registered panel topology was not detected"),
        ],
        "non-merging devices no longer retain exact registry topology"
    );
}

/// Even an open frame cannot turn a declared splash into a hidden split page.
#[test]
fn renderer_keeps_one_panel_devices_as_one_region() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![diagonal_panels()]),
        1,
        ScriptedText::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(
                &active_device_scene(active_layout_scene(1, "en"), "open_frame"),
                &mut Recorder::default(),
            )
            .unwrap_err()
            .to_string(),
        String::from("Rejected after 1 attempts: Unexpected internal gutter in one-panel layout"),
        "one-panel device accepts a hidden split page"
    );
}
