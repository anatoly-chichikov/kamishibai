//! Tests for scene OCR routing and manga validation.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::rc::Rc;

use anyhow::{Result, bail};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use kamishibai::generation::manga::{
    BorderDetector, ImageSource, ImageText, MangaRenderer, Progress, RecallJudge, RecallReview,
    Renderer, SceneText, TextDetector, TextDetectors, TextEnsemble, TextJudge, TextReview,
    TextReviewGate,
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

/// Scripted image recall judge for renderer tests.
#[derive(Clone, Debug)]
struct ScriptedRecall {
    values: Rc<RefCell<VecDeque<RecallReview>>>,
    images: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl ScriptedRecall {
    /// Create one scripted image recall judge.
    fn new(values: &[&str]) -> Self {
        Self {
            values: Rc::new(RefCell::new(
                values.iter().map(|value| recall_review(value)).collect(),
            )),
            images: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl RecallJudge for ScriptedRecall {
    /// Return the next scripted image recall verdict.
    fn review(&self, _scene: &Value, image: &[u8]) -> Result<RecallReview> {
        self.images.borrow_mut().push(image.to_vec());
        self.values
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("recall judge ran out of scripted verdicts"))
    }
}

fn recall_review(reading: &str) -> RecallReview {
    if reading == "zoom-safe" {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "literal_writing_present": false,
            "literal_evidence": [],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("zoom-inspected recall review must decode");
    }
    if let Some(description) = reading.strip_prefix("technical:") {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "literal_writing_present": true,
            "literal_evidence": [{
                "description": description,
                "location": "drafting sheet in right panel",
                "kind": "TECHNICAL_DIAGRAM"
            }],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("technical-diagram recall review must decode");
    }
    if let Some(description) = reading.strip_prefix("literal:") {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "literal_writing_present": true,
            "literal_evidence": [{
                "description": description,
                "location": "open book in upper panel",
                "kind": "PSEUDO_WRITING"
            }],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("literal recall review must decode");
    }
    if reading == "missing-required-subject" {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "scene_fidelity_decision": "REJECT",
            "scene_fidelity_evidence": [{
                "requirement": "panel p1 requires agitated_companion, a tall man shouting while leaning forward",
                "observed": "only the weary seated speaker is visible and no second person appears",
                "location": "both panels",
                "kind": "MISSING_REQUIRED_SUBJECT"
            }],
            "literal_writing_present": false,
            "literal_evidence": [],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("missing required subject recall review must decode");
    }
    if reading == "broken-subject-continuity" {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "scene_fidelity_decision": "REJECT",
            "scene_fidelity_evidence": [{
                "requirement": "touchy_man must remain the same person in p1 and p2",
                "observed": "p1 shows an older heavy square-faced man in a crewneck while p2 shows a younger slim soft-faced man in a collared sweater",
                "location": "touchy_man in left p1 and listener in right p2",
                "kind": "BROKEN_SUBJECT_CONTINUITY"
            }],
            "literal_writing_present": false,
            "literal_evidence": [],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "The repeated subject is visibly substituted"
        }))
        .expect("broken subject continuity review must decode");
    }
    if reading == "borderless" || reading == "torn" || reading == "breakout" {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "literal_writing_present": false,
            "literal_evidence": [],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "page_frame": reading.to_uppercase(),
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("page-frame recall review must decode");
    }
    if let Some(reading) = reading.strip_prefix("unrelated:") {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [{
                "reading": reading,
                "location": "school gate pillar",
                "kind": "UNRELATED"
            }],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "The writing is unrelated to the hidden answer"
        }))
        .expect("unrelated recall review must decode");
    }
    if reading.is_empty() || reading == "un un" {
        return serde_json::from_value(json!({
            "decision": "ALLOW",
            "evidence": [],
            "fidelity_inspected": true,
            "zoom_inspected": true,
            "reason": "No answer-bearing writing is visible"
        }))
        .expect("allow recall review must decode");
    }
    serde_json::from_value(json!({
        "decision": "REJECT",
        "evidence": [{
            "reading": reading,
            "location": "upper panel",
            "kind": "FOCUS"
        }],
        "reason": "The hidden answer is legible"
    }))
    .expect("reject recall review must decode")
}

/// Scripted literal-writing judge for renderer tests.
#[derive(Clone, Debug)]
struct ScriptedText {
    values: Rc<RefCell<VecDeque<TextReview>>>,
    images: Rc<RefCell<Vec<Vec<u8>>>>,
    gate: TextReviewGate,
}

impl ScriptedText {
    /// Create one scripted direct text judge.
    fn new(values: &[&str]) -> Self {
        Self {
            values: Rc::new(RefCell::new(
                values
                    .iter()
                    .map(|value| text_review(value, TextReviewGate::LlmJudge))
                    .collect(),
            )),
            images: Rc::new(RefCell::new(Vec::new())),
            gate: TextReviewGate::LlmJudge,
        }
    }

    /// Create one scripted OCR text judge.
    fn ocr(values: &[&str]) -> Self {
        Self {
            values: Rc::new(RefCell::new(
                values
                    .iter()
                    .map(|value| text_review(value, TextReviewGate::Ocr))
                    .collect(),
            )),
            images: Rc::new(RefCell::new(Vec::new())),
            gate: TextReviewGate::Ocr,
        }
    }
}

impl TextJudge for ScriptedText {
    /// Return the direct LLM route used by this test judge.
    fn gate(&self) -> TextReviewGate {
        self.gate
    }

    /// Return the next scripted literal-writing verdict.
    fn review(&self, image: &[u8], _grayscale: &GrayImage) -> Result<TextReview> {
        self.images.borrow_mut().push(image.to_vec());
        self.values
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("text judge ran out of scripted verdicts"))
    }
}

fn text_review(reading: &str, gate: TextReviewGate) -> TextReview {
    let (decision, evidence, reason) = if reading.is_empty() {
        (
            "ALLOW",
            json!([]),
            "No literal writing or numerals are visible",
        )
    } else {
        (
            "REJECT",
            json!([{
                "reading": reading,
                "location": "center sign",
                "kind": "WRITING"
            }]),
            "Literal writing is visible",
        )
    };
    serde_json::from_value(json!({
        "gate": match gate {
            TextReviewGate::Ocr => "OCR",
            TextReviewGate::LlmJudge => "LLM_JUDGE",
        },
        "decision": decision,
        "evidence": evidence,
        "reason": reason
    }))
    .expect("scripted text review must decode")
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
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        let Some(image) = self.values.borrow_mut().pop_front() else {
            bail!("image source ran out of scripted images");
        };
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image).write_to(&mut cursor, ImageFormat::Png)?;
        Ok(cursor.into_inner())
    }
}

#[derive(Clone, Debug)]
struct FixedBytes {
    bytes: Vec<u8>,
}

impl FixedBytes {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
        }
    }
}

impl ImageSource for FixedBytes {
    /// Return one fixed encoded image payload.
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }
}

/// Image source that always reports one provider failure.
#[derive(Clone, Debug)]
struct FailingSource;

impl ImageSource for FailingSource {
    /// Return one provider failure without encoded image bytes.
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        bail!("provider rejected image request")
    }
}

/// Image recall judge that always reports one infrastructure failure.
#[derive(Clone, Debug)]
struct FailingRecall;

impl RecallJudge for FailingRecall {
    /// Return one image recall failure after the raw image has been captured.
    fn review(&self, _scene: &Value, _image: &[u8]) -> Result<RecallReview> {
        bail!("recall judge failed")
    }
}

/// Image recall judge that recovers after one infrastructure failure.
#[derive(Clone, Debug)]
struct RecoveringRecall {
    calls: Rc<RefCell<usize>>,
}

impl RecoveringRecall {
    /// Create one recall judge that fails once and then allows the image.
    fn new() -> Self {
        Self {
            calls: Rc::new(RefCell::new(0)),
        }
    }
}

impl RecallJudge for RecoveringRecall {
    /// Fail the first review and allow every later review.
    fn review(&self, _scene: &Value, _image: &[u8]) -> Result<RecallReview> {
        let mut calls = self.calls.borrow_mut();
        *calls += 1;
        if *calls == 1 {
            bail!("recall judge failed once");
        }
        Ok(recall_review(""))
    }
}

/// Scripted image source that records every provider prompt.
#[derive(Clone, Debug)]
struct CapturingSource {
    values: Rc<RefCell<VecDeque<GrayImage>>>,
    prompts: Rc<RefCell<Vec<String>>>,
}

impl CapturingSource {
    /// Create one capturing source from scripted grayscale images.
    fn new(values: Vec<GrayImage>) -> Self {
        Self {
            values: Rc::new(RefCell::new(values.into())),
            prompts: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl ImageSource for CapturingSource {
    /// Record the provider prompt and return one scripted PNG payload.
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.prompts.borrow_mut().push(String::from(prompt));
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
    let (template, materialized) = match panels {
        1 => (
            "splash-1-v1",
            vec![json!({
                "id": "panel-0",
                "bounds": {"x": 32, "y": 32, "width": 960, "height": 960}
            })],
        ),
        2 => (
            "equal-split-vertical-2-v1",
            vec![
                json!({
                    "id": "panel-0",
                    "bounds": {"x": 32, "y": 32, "width": 448, "height": 960}
                }),
                json!({
                    "id": "panel-1",
                    "bounds": {"x": 544, "y": 32, "width": 448, "height": 960}
                }),
            ],
        ),
        _ => panic!("invariant: basic scene test supports one or two panels"),
    };
    json!({
        "manga_panel": {
            "meta": {
                "target_lang": target
            },
            "canvas": {
                "width": 1024,
                "height": 1024
            },
            "panel_layout": {
                "active_layout": {
                    "template_id": template
                }
            },
            "page_design": {
                "special_device": {
                    "kind": "none"
                }
            },
            "panels": materialized
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
            "template_id": if panels == 1 {
                "splash-1-v1"
            } else {
                "equal-split-vertical-2-v1"
            }
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
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] = json!("t-bottom-3-v1");
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

/// Create one plain three-panel diagonal strip scene.
fn diagonal_strip_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {
            "id": "p1",
            "bounds": {"x": 16, "y": 16, "width": 344, "height": 992},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[16, 16], [280, 16], [360, 1008], [16, 1008]]
            }
        },
        {
            "id": "p2",
            "bounds": {"x": 292, "y": 16, "width": 440, "height": 992},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[292, 16], [652, 16], [732, 1008], [372, 1008]]
            }
        },
        {
            "id": "p3",
            "bounds": {"x": 664, "y": 16, "width": 344, "height": 992},
            "frame": {
                "shape": "trapezoid",
                "polygon": [[664, 16], [1008, 16], [1008, 1008], [744, 1008]]
            }
        }
    ]);
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("diagonal-strip-3-v1");
    value["manga_panel"]["page_design"]["special_device"] = json!({
        "kind": "none",
        "source_panel": "",
        "target_panel": "",
        "subject_id": ""
    });
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

/// Create one four-panel staggered grid with offset vertical dividers.
fn staggered_grid_layout_scene(target: &str) -> Value {
    let mut value = active_layout_scene(2, target);
    value["manga_panel"]["panels"] = json!([
        {
            "id": "p1",
            "bounds": {"x": 16, "y": 16, "width": 600, "height": 484},
            "frame": {"shape": "trapezoid", "polygon": [[16, 16], [616, 16], [600, 500], [16, 500]]}
        },
        {
            "id": "p2",
            "bounds": {"x": 628, "y": 16, "width": 380, "height": 484},
            "frame": {"shape": "trapezoid", "polygon": [[628, 16], [1008, 16], [1008, 500], [644, 500]]}
        },
        {
            "id": "p3",
            "bounds": {"x": 16, "y": 516, "width": 360, "height": 492},
            "frame": {"shape": "trapezoid", "polygon": [[16, 516], [376, 516], [360, 1008], [16, 1008]]}
        },
        {
            "id": "p4",
            "bounds": {"x": 388, "y": 516, "width": 620, "height": 492},
            "frame": {"shape": "trapezoid", "polygon": [[388, 516], [1008, 516], [1008, 1008], [404, 1008]]}
        }
    ]);
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("staggered-grid-4-v1");
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
    value["manga_panel"]["panel_layout"]["active_layout"]["template_id"] =
        json!("vertical-triptych-3-v1");
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
        json!("diagonal-split-2-v1");
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
            value["manga_panel"]["panels"][0]["scene"]["subjects"] = json!([{
                "id": "actor",
                "figure": "the same visible actor"
            }]);
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

/// Create one symmetric four-panel grid with aligned vertical dividers.
fn regular_grid_panels() -> GrayImage {
    let mut image = framed(64, 1);
    for y in 0..64 {
        for x in 31..33 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 31..33 {
        for x in 0..64 {
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

/// Create two inset regions with a shifted separator in one chosen diagonal direction.
fn inset_diagonal_panels(mirrored: bool) -> GrayImage {
    let mut image = GrayImage::from_pixel(128, 128, Luma([255]));
    for y in 8u32..120 {
        let progress = y.saturating_sub(8).saturating_mul(12) / 111;
        let separator = if mirrored {
            72u32.saturating_sub(progress)
        } else {
            60u32.saturating_add(progress)
        };
        for x in 8..separator {
            image.put_pixel(x, y, Luma([0]));
        }
        for x in separator.saturating_add(2)..120 {
            image.put_pixel(x, y, Luma([0]));
        }
    }
    image
}

/// Create two vertical panels plus one isolated center region of the requested width.
fn split_panels_with_center_region(width: u32) -> GrayImage {
    let mut image = GrayImage::from_pixel(128, 128, Luma([255]));
    let start = 64u32.saturating_sub(width / 2);
    for y in 1..127 {
        for x in 1..55 {
            image.put_pixel(x, y, Luma([0]));
        }
        for x in 73..127 {
            image.put_pixel(x, y, Luma([0]));
        }
        for x in start..start.saturating_add(width) {
            image.put_pixel(x, y, Luma([0]));
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

/// Create one diagonal strip with independently mirrored parallel separators.
fn diagonal_strip_panels(mirrored: [bool; 2]) -> GrayImage {
    let mut image = framed(128, 1);
    for y in 0u32..128 {
        let offset = y.saturating_mul(10) / 127;
        for (index, start) in [35u32, 82u32].into_iter().enumerate() {
            let x = if mirrored[index] {
                start.saturating_add(10).saturating_sub(offset)
            } else {
                start.saturating_add(offset)
            };
            for gutter in x..x.saturating_add(2).min(128) {
                image.put_pixel(gutter, y, Luma([255]));
            }
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

/// Open the first panel to exterior white while retaining visible local content.
fn opened_t_bottom_panels() -> GrayImage {
    let mut image = shifted_t_bottom_panels();
    for y in 1..15 {
        for x in 1..15 {
            image.put_pixel(x, y, Luma([255]));
        }
    }
    for y in 3..12 {
        for x in 3..6 {
            image.put_pixel(x, y, Luma([0]));
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

/// The renderer retries when direct image review finds the hidden answer.
#[test]
fn renderer_retries_when_recall_review_finds_the_hidden_answer() -> Result<()> {
    let recall = ScriptedRecall::new(&["слово", ""]);
    let reviewed = recall.images.clone();
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels(), rectangular_panels()]),
        2,
        recall,
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
            reviewed.borrow().iter().all(|image| {
                image::guess_format(image).is_ok_and(|format| format == ImageFormat::Png)
            }),
        ),
        (
            false,
            vec![(
                String::from("Rendering manga"),
                1,
                String::from(
                    "Recall judge rejected image: The hidden answer is legible: 'слово' at upper panel",
                ),
            )],
            true,
        ),
        "renderer did not judge the actual candidate image before accepting it"
    );
    Ok(())
}

/// A missing-subject finding scores the attempt but no longer burns it.
#[test]
fn renderer_ships_a_missing_subject_finding_with_a_scored_verdict() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["missing-required-subject"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            first["status"].as_str(),
            first["score"].as_u64(),
            first["penalties"]["fidelity"].as_u64(),
            recall["scene_fidelity_evidence"][0]["kind"].as_str(),
        ),
        (
            false,
            0,
            Some("accepted"),
            Some(85),
            Some(15),
            Some("MISSING_REQUIRED_SUBJECT"),
        ),
        "a missing-subject finding burned the attempt instead of scoring it"
    );
    Ok(())
}

/// A continuity finding scores the attempt but no longer burns it.
#[test]
fn renderer_ships_a_continuity_finding_with_a_scored_verdict() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["broken-subject-continuity"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            verdict["status"].as_str(),
            verdict["score"].as_u64(),
            verdict["penalties"]["fidelity"].as_u64(),
            recall["scene_fidelity_evidence"][0]["kind"].as_str(),
        ),
        (
            false,
            Some("accepted"),
            Some(80),
            Some(20),
            Some("BROKEN_SUBJECT_CONTINUITY"),
        ),
        "a continuity finding burned the attempt instead of scoring it"
    );
    Ok(())
}

/// The renderer ships a bled frame untouched instead of painting its margin over.
#[test]
fn renderer_ships_a_bled_frame_untouched() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(16, 16, Luma([0]))]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 2),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(&scene(1, "en"), &mut progress)
        .expect("a bled frame must ship as drawn, not be rejected");
    assert_eq!(
        (
            rendered.to_luma8().get_pixel(0, 0)[0],
            rendered.to_luma8().get_pixel(8, 8)[0],
            progress.retries.len(),
        ),
        (0, 0, 0),
        "renderer painted over a bled frame instead of shipping it untouched"
    );
}

/// The renderer rejects a multi-panel frame when no gutter appears.
#[test]
fn renderer_rejects_a_multi_panel_frame_when_no_gutter_appears() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![framed(16, 1)]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&scene(2, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from(
            "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
        ),
        "renderer no longer rejects a multi panel frame when no gutter appears"
    );
}

/// The renderer accepts expressive geometry while retaining recall and border validation.
#[test]
fn renderer_accepts_expressive_geometry_while_retaining_recall_and_border_validation() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            crossed_t_bottom_panels(),
            GrayImage::from_pixel(32, 32, Luma([0])),
            crossed_t_bottom_panels(),
        ]),
        3,
        ScriptedRecall::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), "crossing"),
            &mut progress,
        )
        .expect("expressive layout must render after recall and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from(
                        "Recall judge rejected image: The hidden answer is legible: 'word' at upper panel",
                    ),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("quality score 60/100: found 1 panel region for 3 planned panels"),
                ),
            ],
        ),
        "expressive geometry bypasses recall or outer border validation"
    );
}

/// The renderer keeps straight-gutter validation for explicit ordinary layouts.
#[test]
fn renderer_rejects_explicit_ordinary_layouts_without_a_gutter() {
    let reasons = ["none", "master_view"].map(|device| {
        MangaRenderer::new(
            QueueSource::new(vec![framed(16, 1)]),
            1,
            ScriptedRecall::new(&[""]),
            BorderDetector::new(2, 6, 240, 1),
        )
        .render(&device_scene(2, "en", device), &mut Recorder::default())
        .expect_err("ordinary multi-panel layout without a gutter must be rejected")
        .to_string()
    });
    assert_eq!(
        reasons,
        [
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
            ),
        ],
        "ordinary page devices no longer require a straight gutter"
    );
}

/// Registry multi-panel geometry retains topology, recall, and outer-border checks.
#[test]
fn renderer_accepts_registry_geometry_while_retaining_recall_and_border_validation() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            rectangular_panels(),
            GrayImage::from_pixel(32, 32, Luma([0])),
            rectangular_panels(),
        ]),
        3,
        ScriptedRecall::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(&active_layout_scene(2, "en"), &mut progress)
        .expect("registry layout must render after recall and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from(
                        "Recall judge rejected image: The hidden answer is legible: 'word' at upper panel",
                    ),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("quality score 60/100: found 1 panel region for 2 planned panels"),
                ),
            ],
        ),
        "registry geometry bypasses topology, recall, or outer border validation"
    );
}

/// Registry validation rejects one splash when the selected layout requires multiple panels.
#[test]
fn renderer_rejects_registry_multi_panel_scene_rendered_as_one_splash() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![framed(32, 1)]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&active_layout_scene(2, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from(
            "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
        ),
        "registry renderer still accepts one splash for a multi panel selection"
    );
}

/// Semantic image review keeps unrelated visible writing without rejecting artwork.
#[test]
fn renderer_accepts_unrelated_visible_writing_for_registry_artwork() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["un un"]),
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
        "unrelated visible writing still rejects safe registry artwork"
    );
    Ok(())
}

/// Production geometry accepts the right topology when a model shifts one separator.
#[test]
fn renderer_accepts_shifted_separator_in_a_multi_panel_registry_page() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![shifted_t_bottom_panels()]),
        1,
        ScriptedRecall::new(&[""]),
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
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let rendered = renderer.render(&scene, &mut Recorder::default())?;
    assert!(
        !rendered.color().has_color(),
        "steeper slanted separator is still rejected despite retaining exact topology"
    );
    Ok(())
}

/// A plain slanted layout may mirror its decorative slope while preserving exact topology.
#[test]
fn renderer_accepts_a_mirrored_plain_slant_with_exact_topology() -> Result<()> {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![diagonal_strip_panels([true, true])]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let rendered = renderer.render(&diagonal_strip_layout_scene("en"), &mut Recorder::default())?;
    assert!(
        !rendered.color().has_color(),
        "globally mirrored diagonal strip is rejected despite preserving exact panel topology"
    );
    Ok(())
}

/// A V-shaped strip cannot masquerade as one globally mirrored diagonal layout.
#[test]
fn renderer_rejects_a_mixed_direction_diagonal_strip() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![diagonal_strip_panels([true, false])]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert!(
        renderer
            .render(&diagonal_strip_layout_scene("en"), &mut Recorder::default())
            .is_err(),
        "mixed-direction gutters were accepted as one globally mirrored diagonal strip"
    );
}

/// The emphasis rail requires its declared horizontal divider to retain a real slope.
#[test]
fn renderer_accepts_slanted_rail_and_rejects_straightened_rail() {
    let accepted = [true, false].map(|slanted| {
        MangaRenderer::new(
            QueueSource::new(vec![slanted_rail_panels(slanted)]),
            1,
            ScriptedRecall::new(&[""]),
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
        ScriptedRecall::new(&[""]),
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

/// Load one compact production topology regression fixture.
fn production_topology_fixture(name: &str) -> (Value, GrayImage) {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/topology-production");
    let scene = serde_json::from_slice::<Value>(
        std::fs::read(directory.join(format!("{name}.scene.json")))
            .expect("production topology scene must be readable")
            .as_slice(),
    )
    .expect("production topology scene must decode");
    let image = image::open(directory.join(format!("{name}.jpg")))
        .expect("production topology image must decode")
        .into_luma8();
    (scene, image)
}

/// Load one production image used to calibrate a locally declared topology.
fn production_topology_image(name: &str) -> GrayImage {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/topology-production")
        .join(format!("{name}.jpg"));
    image::open(path)
        .expect("production topology image must decode")
        .into_luma8()
}

/// Topology-only replay accepts an archived oblique rail with a reversed shallow slope.
#[test]
fn topology_accepts_archived_slanted_rail_with_shifted_divider() {
    let image = production_topology_image("slanted-rail-shifted");
    let regions = BorderDetector::new(6, 24, 240, 0).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(6, 24, 240, 0),
    )
    .render(&slanted_rail_layout_scene("en"), &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (3, true),
        "shifted production rail still fails despite retaining three oblique regions"
    );
}

/// Topology-only replay accepts archived staggered dividers around isolated centers.
#[test]
fn topology_accepts_archived_staggered_grid_with_shifted_dividers() {
    let image = production_topology_image("staggered-grid-shifted");
    let regions = BorderDetector::new(6, 24, 240, 0).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(6, 24, 240, 0),
    )
    .render(&staggered_grid_layout_scene("en"), &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (4, true),
        "shifted production staggered grid still fails despite retaining four regions"
    );
}

/// A staggered-grid declaration cannot accept a symmetric two-by-two grid.
#[test]
fn renderer_rejects_regular_grid_for_declared_staggered_grid() {
    let image = regular_grid_panels();
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&image);
    let rejected = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(&staggered_grid_layout_scene("en"), &mut Recorder::default())
    .is_err();
    assert_eq!(
        (regions, rejected),
        (4, true),
        "staggered-grid fallback still accepts aligned symmetric dividers"
    );
}

/// Topology-only replay accepts an archived crossing whose halo preserves three regions.
#[test]
fn topology_accepts_archived_crossing_with_closed_registered_regions() {
    let (scene, image) = production_topology_fixture("crossing-exact");
    let regions = BorderDetector::new(6, 24, 240, 0).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(6, 24, 240, 0),
    )
    .render(&scene, &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (3, true),
        "closed subject halo still makes a valid slanted crossing fail topology"
    );
}

/// Exact regions and a slanted divider cannot stand in for visible crossing content.
#[test]
fn renderer_rejects_slanted_layout_without_visible_crossing_content() {
    let image = shifted_slanted_t_bottom_panels();
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&image);
    let rejected = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(
        &slanted_crossing_layout_scene("en"),
        &mut Recorder::default(),
    )
    .is_err();
    assert_eq!(
        (regions, rejected),
        (3, true),
        "crossing gate still accepts an uninterrupted slanted gutter"
    );
}

/// Topology-only replay accepts an archived crossing that merges its declared pair.
#[test]
fn topology_accepts_archived_crossing_with_declared_merged_pair() {
    let (scene, image) = production_topology_fixture("crossing-merged");
    let regions = BorderDetector::new(6, 24, 240, 0).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(6, 24, 240, 0),
    )
    .render(&scene, &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (2, true),
        "declared merged crossing still loses its registered separator slope"
    );
}

/// Topology-only replay rejects an archived closed panel declared as open frame.
#[test]
fn topology_rejects_archived_closed_panel_declared_as_open_frame() {
    let (scene, image) = production_topology_fixture("open-frame");
    let regions = BorderDetector::new(6, 24, 240, 0).regions(&image);
    let rejected = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(6, 24, 240, 0),
    )
    .render(&scene, &mut Recorder::default())
    .is_err();
    assert_eq!(
        (regions, rejected),
        (3, true),
        "open-frame gate still accepts an ordinary closed production panel"
    );
}

/// An open source retains visible content while its center joins exterior white.
#[test]
fn renderer_accepts_visibly_open_frame_with_registered_companions() {
    let image = opened_t_bottom_panels();
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(
        &active_device_scene(t_bottom_layout_scene("en"), "open_frame"),
        &mut Recorder::default(),
    )
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (3, true),
        "visibly open source cannot retain its two closed companion regions"
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
        ScriptedRecall::new(&[""]),
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
        String::from(
            "Rejected after 1 attempts: quality score 60/100: found 2 panel regions for 3 planned panels"
        ),
        "open frame accepts a blank source as one semantic panel"
    );
}

/// Registry geometry rejects the right region count when it realizes a different layout.
#[test]
fn renderer_rejects_t_grid_for_a_declared_horizontal_triptych() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![upper_t_bottom_panels()]),
        1,
        ScriptedRecall::new(&[""]),
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
        String::from(
            "Rejected after 1 attempts: quality score 60/100: planned panels share one drawn region"
        ),
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
            ScriptedRecall::new(&[""]),
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

/// A shifted inset diagonal keeps its registered direction despite coordinate drift.
#[test]
fn renderer_accepts_shifted_inset_diagonal_with_declared_direction() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![inset_diagonal_panels(false)]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert!(
        renderer
            .render(&diagonal_layout_scene("en"), &mut Recorder::default())
            .is_ok(),
        "registry renderer still rejects a shifted inset diagonal with the declared direction"
    );
}

/// A shifted inset diagonal cannot reverse the registered separator direction.
#[test]
fn renderer_rejects_shifted_inset_diagonal_with_mirrored_direction() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![inset_diagonal_panels(true)]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert!(
        renderer
            .render(&diagonal_layout_scene("en"), &mut Recorder::default())
            .is_err(),
        "registry renderer accepts a shifted inset diagonal with a mirrored direction"
    );
}

/// A sub-two-percent isolated separator strip cannot become a semantic panel.
#[test]
fn renderer_ignores_sub_two_percent_separator_region() {
    let image = split_panels_with_center_region(2);
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&image);
    let accepted = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(&active_layout_scene(2, "en"), &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (regions, accepted),
        (2, true),
        "sub-two-percent separator strip still counts as an extra semantic panel"
    );
}

/// A registry-sized isolated region remains an undeclared semantic panel.
#[test]
fn renderer_rejects_registry_sized_extra_region() {
    let image = split_panels_with_center_region(11);
    let regions = BorderDetector::new(2, 6, 240, 1).regions(&image);
    let rejected = MangaRenderer::new(
        QueueSource::new(vec![image]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(&active_layout_scene(2, "en"), &mut Recorder::default())
    .is_err();
    assert_eq!(
        (regions, rejected),
        (3, true),
        "registry renderer ignores an undeclared registry-sized semantic region"
    );
}

/// The ordinary-layout fallback cannot satisfy a declared crossing device.
#[test]
fn renderer_rejects_shifted_diagonal_without_declared_crossing() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![inset_diagonal_panels(false)]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert!(
        renderer
            .render(
                &active_device_scene(diagonal_layout_scene("en"), "crossing"),
                &mut Recorder::default(),
            )
            .is_err(),
        "ordinary-layout fallback accepts two closed regions as a declared crossing"
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
            ScriptedRecall::new(&[""]),
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

/// Provider requests use bounded prose without inactive planning alternatives.
#[test]
fn renderer_sends_compiled_prose_without_planning_metadata() {
    let mut canonical = active_layout_scene(2, "en");
    canonical["manga_panel"]["meta"]["title"] =
        json!("EXACT_SENTENCE_MUST_NOT_REACH_IMAGE_PROVIDER");
    canonical["manga_panel"]["meta"]["description"] =
        json!("EXACT_SENTENCE_MUST_NOT_REACH_IMAGE_PROVIDER");
    canonical["manga_panel"]["meta"]["layout_selection"] = json!({
        "chosen_template_id": "equal-split-vertical-2-v1",
        "deterministic_slot": 1,
        "device_candidates": [{"scene_kind": "open_frame"}],
        "eligible_template_ids": ["equal-split-vertical-2-v1", "competing-layout-v1"],
        "ranked_candidates": [{"template_id": "competing-layout-v1"}],
        "scene_attempt_index": 2,
        "scene_features": {"panel_count": 2},
        "seed_source": "provider-test"
    });
    canonical["manga_panel"]["panel_layout"]["active_permissions"] = json!({"open_frame": false});
    canonical["manga_panel"]["panel_layout"]["conditional_permissions"] =
        json!({"open_frame": "inactive alternative"});
    canonical["manga_panel"]["panel_layout"]["permissions_from"] =
        json!("page_design.special_device.kind");
    canonical["manga_panel"]["page_design"] = json!({
        "camera_arc": {"strategy": "push_in"},
        "special_device": {"kind": "none", "source_panel": "", "target_panel": "", "subject_id": ""}
    });
    canonical["manga_panel"]["semantic_spine"] = json!({"literal_event": "one selected event"});
    canonical["manga_panel"]["rendering_rules"] = json!({"outer_border": "16px_pure_white"});
    canonical["manga_panel"]["panels"][0]["scene"] = json!({"camera": {"shot_scale": "wide"}});
    let source = CapturingSource::new(vec![rectangular_panels()]);
    let captured = source.prompts.clone();
    MangaRenderer::new(
        source,
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(&canonical, &mut Recorder::default())
    .expect("compiled provider prompt must retain valid canonical geometry");
    let prompt = captured
        .borrow()
        .first()
        .cloned()
        .expect("image source must receive one provider prompt");
    assert_eq!(
        (
            captured.borrow().len(),
            [
                prompt.starts_with("Create a finished black-and-white manga page"),
                prompt.contains("two equal upright panels side by side"),
                prompt.contains("The first panel:"),
                !prompt.contains("EXACT_SENTENCE_MUST_NOT_REACH_IMAGE_PROVIDER"),
                !prompt.contains("chosen_template_id"),
                !prompt.contains("competing-layout-v1"),
                !prompt.contains("scene_attempt_index"),
                !prompt.contains("provider-test"),
                prompt.chars().all(|character| !character.is_ascii_digit()),
                (150..=250).contains(&prompt.split_whitespace().count()),
            ],
        ),
        (1, [true; 10]),
        "provider prose leaked planner metadata or lost the image-prompt contract"
    );
}

/// Local prompt failures consume no image-provider request.
#[test]
fn renderer_cannot_call_provider_before_prompt_compilation_succeeds() {
    let source = CapturingSource::new(Vec::new());
    let captured = source.prompts.clone();
    let failed = MangaRenderer::new(
        source,
        1,
        ScriptedRecall::new(&[]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .render(&json!({}), &mut Recorder::default())
    .is_err();
    assert_eq!(
        (failed, captured.borrow().len()),
        (true, 0),
        "an invalid local prompt consumed one image-provider request"
    );
}

/// Production validation archives every accepted and rejected raw image attempt.
#[test]
fn renderer_archives_registry_image_attempts_with_verdicts() -> Result<()> {
    let temporary = tempdir()?;
    let source = CapturingSource::new(vec![rectangular_panels(), rectangular_panels()]);
    let captured = source.prompts.clone();
    let text = ScriptedText::new(&["", ""]);
    let text_images = text.images.clone();
    let scene = active_layout_scene(2, "en");
    let renderer = MangaRenderer::new(
        source,
        2,
        ScriptedRecall::new(&["word", ""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(text)
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&scene, &mut progress)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let second = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.json"),
    )?)?;
    let first_recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    let second_recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.recall.json"),
    )?)?;
    let first_text = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.text.json"),
    )?)?;
    let second_text = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.text.json"),
    )?)?;
    let first_scene = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.scene.json"),
    )?)?;
    let second_scene = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.scene.json"),
    )?)?;
    let first_prompt = std::fs::read_to_string(temporary.path().join("attempt-0001.prompt.txt"))?;
    let second_prompt = std::fs::read_to_string(temporary.path().join("attempt-0002.prompt.txt"))?;
    let sent = captured.borrow().clone();
    assert_eq!(
        (
            [
                rendered.color().has_color(),
                temporary.path().join("attempt-0001.png").is_file(),
                temporary.path().join("attempt-0002.png").is_file(),
            ],
            [
                first["scene"].as_str(),
                second["scene"].as_str(),
                first["prompt"].as_str(),
                second["prompt"].as_str(),
                first["recall"].as_str(),
                second["recall"].as_str(),
                first["text"].as_str(),
                second["text"].as_str(),
            ],
            [first_scene == scene, second_scene == scene],
            [first_prompt == sent[0], second_prompt == sent[1]],
            [
                first_recall["decision"].as_str(),
                first_recall["evidence"][0]["reading"].as_str(),
                second_recall["decision"].as_str(),
            ],
            [
                first_text["gate"].as_str(),
                first_text["decision"].as_str(),
                second_text["decision"].as_str(),
            ],
            text_images.borrow().len(),
            [
                first["status"].as_str(),
                first["category"].as_str(),
                first["reason"].as_str(),
                second["status"].as_str(),
                second["category"].as_str(),
            ],
        ),
        (
            [false, true, true],
            [
                Some("attempt-0001.scene.json"),
                Some("attempt-0002.scene.json"),
                Some("attempt-0001.prompt.txt"),
                Some("attempt-0002.prompt.txt"),
                Some("attempt-0001.recall.json"),
                Some("attempt-0002.recall.json"),
                Some("attempt-0001.text.json"),
                Some("attempt-0002.text.json"),
            ],
            [true; 2],
            [true; 2],
            [Some("REJECT"), Some("word"), Some("ALLOW")],
            [Some("LLM_JUDGE"), Some("ALLOW"), Some("ALLOW")],
            2,
            [
                Some("rejected"),
                Some("recall_text"),
                Some(
                    "Recall judge rejected image: The hidden answer is legible: 'word' at upper panel",
                ),
                Some("accepted"),
                Some("accepted"),
            ],
        ),
        "production image attempts or their validation verdicts were discarded"
    );
    Ok(())
}

/// Accepted attempt sidecars explicitly prove the scale-aware literal scan ran.
#[test]
fn renderer_archives_zoom_inspection_proof_on_an_accepted_attempt() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["zoom-safe"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(ScriptedText::new(&[""]))
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            recall["fidelity_inspected"].as_bool(),
            recall["zoom_inspected"].as_bool(),
            verdict["status"].as_str(),
            verdict["recall"].as_str(),
        ),
        (
            false,
            Some(true),
            Some(true),
            Some("accepted"),
            Some("attempt-0001.recall.json"),
        ),
        "accepted attempt lost explicit proof of dedicated fidelity or scale-aware literal review"
    );
    Ok(())
}

/// The final attempt ships the best archived non-blocked frame instead of failing.
#[test]
fn renderer_salvages_the_best_scored_attempt_after_the_final_rejection() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(32, 32, Luma([0]))]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            verdict["status"].as_str(),
            verdict["blocker"].as_bool(),
            verdict["score"].as_u64().is_some_and(|score| score < 100),
        ),
        (false, Some("salvaged"), Some(false), true),
        "the final rejected attempt was not salvaged from the archive"
    );
    Ok(())
}

/// Answer leakage stays a blocker that salvage never ships.
#[test]
fn renderer_never_salvages_an_answer_leaking_attempt() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["word"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    assert!(
        renderer
            .render(&active_layout_scene(2, "en"), &mut Recorder::default())
            .is_err(),
        "an answer-leaking attempt was salvaged into production"
    );
    Ok(())
}

/// A page with no frame anywhere is blocked when judge and perimeter agree.
#[test]
fn renderer_blocks_a_borderless_page_confirmed_by_its_inked_perimeter() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(16, 16, Luma([0]))]),
        1,
        ScriptedRecall::new(&["borderless"]),
        BorderDetector::new(2, 6, 240, 2),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let error = renderer
        .render(&scene(1, "en"), &mut Recorder::default())
        .unwrap_err()
        .to_string();
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            error.as_str(),
            verdict["category"].as_str(),
            verdict["blocker"].as_bool(),
            verdict["score"].as_u64(),
        ),
        (
            "Rejected after 1 attempts: No panel frame anywhere and ink reaches every page edge",
            Some("borderless"),
            Some(true),
            Some(0),
        ),
        "a fully borderless page escaped the frame blocker"
    );
    Ok(())
}

/// A mechanically present white margin vetoes a judged borderless verdict.
#[test]
fn renderer_keeps_a_framed_page_the_judge_wrongly_calls_borderless() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![framed(16, 4)]),
        1,
        ScriptedRecall::new(&["borderless"]),
        BorderDetector::new(2, 6, 240, 2),
    );
    assert!(
        renderer
            .render(&scene(1, "en"), &mut Recorder::default())
            .is_ok(),
        "a judged borderless verdict blocked a page whose white margin mechanically exists"
    );
}

/// A terminal judge failure still ships the best archived frame.
#[test]
fn renderer_salvages_the_archive_after_a_terminal_judge_failure() -> Result<()> {
    let temporary = tempdir()?;
    let first = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(32, 32, Luma([0]))]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let _ = first.render(&active_layout_scene(2, "en"), &mut Recorder::default());
    let second = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&[]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    let rendered = second.render(&active_layout_scene(2, "en"), &mut Recorder::default())?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (rendered.color().has_color(), verdict["status"].as_str()),
        (false, Some("salvaged")),
        "a terminal judge failure lost the archived frames instead of salvaging one"
    );
    Ok(())
}

/// A provider refusal — an exhausted image budget — still ships the archive.
#[test]
fn renderer_salvages_the_archive_when_the_provider_refuses() -> Result<()> {
    let temporary = tempdir()?;
    let first = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(32, 32, Luma([0]))]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let _ = first.render(&active_layout_scene(2, "en"), &mut Recorder::default());
    let second = MangaRenderer::new(
        QueueSource::new(vec![]),
        1,
        ScriptedRecall::new(&[]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    let rendered = second.render(&active_layout_scene(2, "en"), &mut Recorder::default())?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (rendered.color().has_color(), verdict["status"].as_str()),
        (false, Some("salvaged")),
        "an exhausted image budget failed the card despite salvageable archived frames"
    );
    Ok(())
}

/// The borderless blocker is never salvaged into production.
#[test]
fn renderer_never_salvages_a_borderless_attempt() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(16, 16, Luma([0]))]),
        1,
        ScriptedRecall::new(&["borderless"]),
        BorderDetector::new(2, 6, 240, 2),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    assert!(
        renderer
            .render(&scene(1, "en"), &mut Recorder::default())
            .is_err(),
        "a borderless attempt was salvaged into production"
    );
    Ok(())
}

/// Salvage prefers the archived frame closest to the registered topology.
#[test]
fn salvage_prefers_the_closest_topology_among_rejected_frames() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            GrayImage::from_pixel(32, 32, Luma([0])),
            shifted_t_bottom_panels(),
        ]),
        2,
        ScriptedRecall::new(&["", ""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .with_salvage();
    let rendered = renderer.render(
        &active_device_scene(t_bottom_layout_scene("en"), "crossing"),
        &mut Recorder::default(),
    )?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let second = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0002.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            first["status"].as_str(),
            first["score"].as_u64(),
            second["status"].as_str(),
            second["score"].as_u64(),
        ),
        (
            false,
            Some("rejected"),
            Some(60),
            Some("salvaged"),
            Some(76)
        ),
        "salvage shipped a frame farther from the registered topology"
    );
    Ok(())
}

/// A single detected writing finding scores the attempt but ships it anyway.
#[test]
fn renderer_ships_single_writing_finding_and_still_reviews_leakage() -> Result<()> {
    let text = ScriptedText::new(&["OPEN"]);
    let text_pending = text.values.clone();
    let recall = ScriptedRecall::new(&["zoom-safe"]);
    let recall_pending = recall.values.clone();
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        recall,
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(text)
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            text_pending.borrow().len(),
            recall_pending.borrow().len(),
            progress.retries.len(),
            verdict["status"].as_str(),
            verdict["score"].as_u64(),
            verdict["penalties"]["text"].as_u64(),
        ),
        (false, 0, 0, 0, Some("accepted"), Some(88), Some(12)),
        "a single writing finding burned the attempt instead of scoring it"
    );
    Ok(())
}

/// Stacked cosmetic findings ship immediately with their combined score.
#[test]
fn renderer_ships_stacked_cosmetic_findings_immediately() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["literal:rows of CJK-like pseudo-glyphs"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(ScriptedText::new(&["OPEN"]))
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "ko"), &mut progress)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            verdict["status"].as_str(),
            verdict["score"].as_u64(),
        ),
        (false, 0, Some("accepted"), Some(82)),
        "stacked cosmetic findings burned the attempt instead of shipping scored"
    );
    Ok(())
}

/// A torn frame line burns the attempt with its finding.
#[test]
fn renderer_retries_a_torn_frame_with_its_finding() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["torn"]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&active_layout_scene(2, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from(
            "Rejected after 1 attempts: quality score 76/100: generation artifact tears the panel frame"
        ),
        "a torn frame line escaped the frame-structure gate"
    );
}

/// A judged breakout forgives a mechanical topology mismatch and ships.
#[test]
fn renderer_ships_a_breakout_page_despite_a_topology_mismatch() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![GrayImage::from_pixel(32, 32, Luma([0]))]),
        1,
        ScriptedRecall::new(&["breakout"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "en"), &mut progress)?;
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            verdict["status"].as_str(),
            verdict["score"].as_u64(),
            verdict["penalties"]["topology"].as_u64(),
        ),
        (false, 0, Some("accepted"), Some(92), Some(8)),
        "a judged breakout page was rejected for its mechanical topology mismatch"
    );
    Ok(())
}

/// A transcribed reading the OCR gate missed scores the attempt but ships it.
#[test]
fn renderer_ships_transcribed_writing_the_first_text_gate_missed() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["unrelated:고등학교"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(ScriptedText::ocr(&[""]))
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "ko"), &mut progress)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            first["status"].as_str(),
            first["score"].as_u64(),
            first["penalties"]["literal"].as_u64(),
        ),
        (false, 0, Some("accepted"), Some(92), Some(8)),
        "a single transcribed reading burned the attempt instead of scoring it"
    );
    Ok(())
}

/// A grounded pseudo-writing finding scores the attempt but no longer burns it.
#[test]
fn renderer_ships_grounded_pseudo_writing_with_a_scored_verdict() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&["literal:rows of CJK-like pseudo-glyphs"]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(ScriptedText::ocr(&[""]))
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "ko"), &mut progress)?;
    let first_recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            first_recall["literal_evidence"][0]["kind"].as_str(),
            first["status"].as_str(),
            first["score"].as_u64(),
            first["penalties"]["literal"].as_u64(),
        ),
        (
            false,
            0,
            Some("PSEUDO_WRITING"),
            Some("accepted"),
            Some(94),
            Some(6)
        ),
        "a single pseudo-writing finding burned the attempt instead of scoring it"
    );
    Ok(())
}

/// A grounded technical-diagram finding scores the attempt but no longer burns it.
#[test]
fn renderer_ships_grounded_technical_diagram_with_a_scored_verdict() -> Result<()> {
    let temporary = tempdir()?;
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        ScriptedRecall::new(&[
            "technical:architectural floor plan with conventional room lines and symbols",
        ]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_text_judge(ScriptedText::ocr(&[""]))
    .with_attempt_archive(temporary.path().to_path_buf());
    let mut progress = Recorder::default();
    let rendered = renderer.render(&active_layout_scene(2, "hi"), &mut progress)?;
    let first_recall = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.recall.json"),
    )?)?;
    let first = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            rendered.color().has_color(),
            progress.retries.len(),
            first_recall["literal_evidence"][0]["kind"].as_str(),
            first["status"].as_str(),
            first["score"].as_u64(),
            first["penalties"]["literal"].as_u64(),
        ),
        (
            false,
            0,
            Some("TECHNICAL_DIAGRAM"),
            Some("accepted"),
            Some(90),
            Some(10),
        ),
        "a single technical-diagram finding burned the attempt instead of scoring it"
    );
    Ok(())
}

/// Provider failures archive their request and terminal verdict without inventing a raw image.
#[test]
fn renderer_archives_provider_failures_without_raw_images() -> Result<()> {
    let temporary = tempdir()?;
    let scene = active_layout_scene(2, "en");
    let failed = MangaRenderer::new(
        FailingSource,
        1,
        ScriptedRecall::new(&[]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .render(&scene, &mut Recorder::default())
    .is_err();
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    let archived_scene = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.scene.json"),
    )?)?;
    let prompt = std::fs::read_to_string(temporary.path().join("attempt-0001.prompt.txt"))?;
    let raw = std::fs::read_dir(temporary.path())?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .any(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("png" | "jpg" | "webp" | "gif" | "bin")
            )
        });
    assert_eq!(
        (
            failed,
            verdict["status"].as_str(),
            verdict["category"].as_str(),
            verdict["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("provider")),
            verdict["image"].is_null(),
            archived_scene == scene,
            prompt.starts_with("Create a finished black-and-white manga page"),
            raw,
        ),
        (
            true,
            Some("error"),
            Some("provider"),
            true,
            true,
            true,
            true,
            false
        ),
        "a launched provider failure was left without terminal request evidence"
    );
    Ok(())
}

/// Recall judge failures replace the captured image's pending verdict with an infrastructure error.
#[test]
fn renderer_archives_recall_judge_failures_after_raw_capture() -> Result<()> {
    let temporary = tempdir()?;
    let scene = active_layout_scene(2, "en");
    let failed = MangaRenderer::new(
        QueueSource::new(vec![rectangular_panels()]),
        1,
        FailingRecall,
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .render(&scene, &mut Recorder::default())
    .is_err();
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            failed,
            temporary.path().join("attempt-0001.png").is_file(),
            verdict["status"].as_str(),
            verdict["category"].as_str(),
            verdict["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("recall judge")),
            verdict["image"].as_str(),
            verdict["recall"].is_null(),
        ),
        (
            true,
            true,
            Some("error"),
            Some("recall_judge"),
            true,
            Some("attempt-0001.png"),
            true,
        ),
        "a recall judge failure was retried or left a production attempt pending"
    );
    Ok(())
}

/// A recovered recall review reuses its archived image instead of paying for another render.
#[test]
fn renderer_reuses_archived_image_after_recall_judge_failure() -> Result<()> {
    let temporary = tempdir()?;
    let scene = active_layout_scene(2, "en");
    let source = CapturingSource::new(vec![rectangular_panels()]);
    let recall = RecoveringRecall::new();
    let renderer = MangaRenderer::new(
        source.clone(),
        1,
        recall.clone(),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf());
    let first = renderer.render(&scene, &mut Recorder::default()).is_err();
    let second = renderer.render(&scene, &mut Recorder::default()).is_ok();
    let verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(
        temporary.path().join("attempt-0001.json"),
    )?)?;
    assert_eq!(
        (
            first,
            second,
            source.prompts.borrow().len(),
            *recall.calls.borrow(),
            verdict["status"].as_str(),
            verdict["recall"].as_str(),
            temporary.path().join("attempt-0002.json").exists(),
        ),
        (
            true,
            true,
            1,
            2,
            Some("accepted"),
            Some("attempt-0001.recall.json"),
            false,
        ),
        "recall recovery generated a replacement image or abandoned its immutable attempt"
    );
    Ok(())
}

/// A process restart reuses an image whose pending review never wrote a verdict.
#[test]
fn renderer_reuses_pending_archived_image_after_process_restart() -> Result<()> {
    let temporary = tempdir()?;
    let scene = active_layout_scene(2, "en");
    let source = CapturingSource::new(vec![rectangular_panels()]);
    let first = MangaRenderer::new(
        source.clone(),
        1,
        FailingRecall,
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .render(&scene, &mut Recorder::default())
    .is_err();
    let verdict_path = temporary.path().join("attempt-0001.json");
    let mut verdict = serde_json::from_str::<Value>(&std::fs::read_to_string(&verdict_path)?)?;
    verdict["status"] = json!("pending");
    verdict["category"] = json!("pending");
    verdict["reason"] = json!("");
    std::fs::write(&verdict_path, serde_json::to_vec_pretty(&verdict)?)?;
    let second = MangaRenderer::new(
        source.clone(),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    )
    .with_attempt_archive(temporary.path().to_path_buf())
    .render(&scene, &mut Recorder::default())
    .is_ok();
    assert_eq!(
        (first, second, source.prompts.borrow().len()),
        (true, true, 1),
        "a pending archived image was replaced after a process restart"
    );
    Ok(())
}

/// Color rejection runs before paid recall review and records its own retry reason.
#[test]
fn renderer_rejects_color_before_recall_review() {
    let recall = ScriptedRecall::new(&["recall must not run"]);
    let pending = recall.values.clone();
    let renderer = MangaRenderer::new(
        FixedBytes::new(include_bytes!("fixtures/monochrome/color-linger.jpg")),
        1,
        recall,
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let error = renderer
        .render(&active_layout_scene(1, "en"), &mut progress)
        .expect_err("colored image must be rejected");
    assert_eq!(
        (error.to_string(), pending.borrow().len(), progress.retries,),
        (
            String::from("Rejected after 1 attempts: Color detected"),
            1,
            vec![(
                String::from("Rendering manga"),
                1,
                String::from("Color detected"),
            )],
        ),
        "color validation no longer runs before paid recall review"
    );
}

/// Registry one-panel geometry retries an image that contains an internal gutter.
#[test]
fn renderer_retries_registry_one_panel_geometry_with_an_internal_gutter() -> Result<()> {
    let recall = ScriptedRecall::new(&["", ""]);
    let pending = recall.values.clone();
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![guttered(32, 1, 2), framed(32, 1)]),
        2,
        recall,
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
            pending.borrow().len(),
        ),
        (
            false,
            vec![(
                String::from("Rendering manga"),
                1,
                String::from("quality score 60/100: found 2 panel regions for 1 planned panel"),
            )],
            0,
        ),
        "structurally rejected artwork skipped the leakage review its scorecard requires"
    );
    Ok(())
}

/// Registry one-panel geometry rejects a diagonal split that has no straight gutter.
#[test]
fn renderer_rejects_registry_one_panel_geometry_split_diagonally() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![diagonal_panels()]),
        1,
        ScriptedRecall::new(&[""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    assert_eq!(
        renderer
            .render(&active_layout_scene(1, "en"), &mut Recorder::default())
            .unwrap_err()
            .to_string(),
        String::from(
            "Rejected after 1 attempts: quality score 60/100: found 2 panel regions for 1 planned panel"
        ),
        "registry renderer still accepts a diagonal diptych as one splash"
    );
}

/// Every structural device requires its complete materialized relation and valid topology.
#[test]
fn renderer_accepts_fully_materialized_structural_devices_with_valid_topology() {
    let accepted = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        let image = match device {
            "crossing" => crossed_t_bottom_panels(),
            "open_frame" => opened_t_bottom_panels(),
            _ => shifted_t_bottom_panels(),
        };
        MangaRenderer::new(
            QueueSource::new(vec![image]),
            1,
            ScriptedRecall::new(&[""]),
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
        ScriptedRecall::new(&[""]),
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
        String::from(
            "Rejected after 1 attempts: quality score 76/100: panel geometry misses the planned layout"
        ),
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
            ScriptedRecall::new(&[""]),
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
        ScriptedRecall::new(&[""]),
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
            ScriptedRecall::new(&[""]),
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
            ScriptedRecall::new(&[""]),
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
            ScriptedRecall::new(&[""]),
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

/// A crossing still rejects answer leakage and a missing outer frame before accepting connected panels.
#[test]
fn renderer_keeps_recall_and_outer_border_checks_for_crossing() {
    let renderer = MangaRenderer::new(
        QueueSource::new(vec![
            crossed_t_bottom_panels(),
            GrayImage::from_pixel(32, 32, Luma([0])),
            crossed_t_bottom_panels(),
        ]),
        3,
        ScriptedRecall::new(&["word", "", ""]),
        BorderDetector::new(2, 6, 240, 1),
    );
    let mut progress = Recorder::default();
    let rendered = renderer
        .render(
            &active_device_scene(t_bottom_layout_scene("en"), "crossing"),
            &mut progress,
        )
        .expect("valid crossing must render after recall and border retries");
    assert_eq!(
        (rendered.color().has_color(), progress.retries),
        (
            false,
            vec![
                (
                    String::from("Rendering manga"),
                    1,
                    String::from(
                        "Recall judge rejected image: The hidden answer is legible: 'word' at upper panel",
                    ),
                ),
                (
                    String::from("Rendering manga"),
                    2,
                    String::from("quality score 60/100: found 1 panel region for 3 planned panels"),
                ),
            ],
        ),
        "crossing bypasses recall or outer border validation"
    );
}

/// A structural device cannot erase more than its single declared panel relation.
#[test]
fn renderer_rejects_two_missing_regions_for_structural_devices() {
    let reasons = ["crossing", "overlap", "inset", "open_frame"].map(|device| {
        MangaRenderer::new(
            QueueSource::new(vec![framed(32, 1)]),
            1,
            ScriptedRecall::new(&[""]),
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
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 3 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 3 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 3 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 3 planned panels"
            ),
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
            ScriptedRecall::new(&[""]),
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
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
            ),
            String::from(
                "Rejected after 1 attempts: quality score 60/100: found 1 panel region for 2 planned panels"
            ),
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
        ScriptedRecall::new(&[""]),
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
        String::from(
            "Rejected after 1 attempts: quality score 60/100: found 2 panel regions for 1 planned panel"
        ),
        "one-panel device accepts a hidden split page"
    );
}
