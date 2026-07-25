use std::cell::Cell;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use serde_json::Value;
use tempfile::TempDir;

use super::attempt_archive::*;
use super::cost_accounting::*;
use super::picture_recovery::*;
use super::picture_requests::*;
use super::scene_attempt::*;
use super::visual::judged;
use super::*;
use crate::application::GenerationCostLedger;
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_COST_FILE, IMAGE_ATTEMPTS_DIRECTORY, META_COST_FILE, PICTURE_REQUESTS_FILE,
    RootStage, SCENE_FILE, VOICE_FILE,
};
use crate::generation::manga::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, RecallJudge, RecallReview, Renderer,
    Translator,
};
use crate::generation::{Audio, Speaker};
use crate::session::{Artifact, ArtifactCosts, ArtifactFile, CostRecord, GenerationCost};

#[derive(Clone, Default)]
struct RecordingLedger {
    costs: Arc<Mutex<Vec<ArtifactCosts>>>,
}

impl RecordingLedger {
    fn cost(&self, slot: usize, artifact: Artifact) -> Option<GenerationCost> {
        self.costs
            .lock()
            .expect("recording ledger lock must remain healthy")
            .get(slot)
            .and_then(|costs| costs.cost(artifact))
    }
}

impl GenerationCostLedger for RecordingLedger {
    fn charge(&self, slot: usize, artifact: Artifact, delta: GenerationCost) -> Result<()> {
        let mut costs = self
            .costs
            .lock()
            .map_err(|_| anyhow!("recording ledger lock is poisoned"))?;
        if costs.len() <= slot {
            costs.resize(slot.saturating_add(1), ArtifactCosts::default());
        }
        costs[slot] = costs[slot].charged(artifact, delta);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct CountingImageSource {
    calls: Rc<Cell<usize>>,
    image: Vec<u8>,
}

impl CountingImageSource {
    fn new(image: Vec<u8>) -> Self {
        Self {
            calls: Rc::new(Cell::new(0)),
            image,
        }
    }

    fn calls(&self) -> usize {
        self.calls.get()
    }
}

#[derive(Clone, Debug)]
struct FailingImageSource {
    calls: Rc<Cell<usize>>,
    error: &'static str,
}

impl FailingImageSource {
    fn new(error: &'static str) -> Self {
        Self {
            calls: Rc::new(Cell::new(0)),
            error,
        }
    }

    fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl ImageSource for FailingImageSource {
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        bail!(self.error)
    }
}

#[derive(Clone)]
struct UsageFreeImageSource {
    costs: CostRecorder,
    image: Vec<u8>,
}

impl UsageFreeImageSource {
    fn new(costs: CostRecorder, image: Vec<u8>) -> Self {
        Self { costs, image }
    }
}

impl ImageSource for UsageFreeImageSource {
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        self.costs.push(CostRecord::new(
            "gemini-3.1-flash-image",
            0,
            0,
            0,
            0,
            GenerationCost::zero(),
        ))?;
        Ok(self.image.clone())
    }
}

#[derive(Clone)]
struct PaidImageSource {
    costs: CostRecorder,
    image: Vec<u8>,
}

impl ImageSource for PaidImageSource {
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        self.costs.push(CostRecord::new(
            "gemini-3.1-flash-image",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(900_000),
        ))?;
        Ok(self.image.clone())
    }
}

impl ImageSource for CountingImageSource {
    fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.image.clone())
    }
}

#[derive(Clone, Copy, Debug)]
struct RejectingRecall;

impl RecallJudge for RejectingRecall {
    fn review(&self, _image: &[u8]) -> Result<RecallReview> {
        Ok(serde_json::from_value(serde_json::json!({
            "decision": "REJECT",
            "evidence": [{
                "reading": "ANSWER",
                "location": "center",
                "kind": "FOCUS"
            }],
            "reason": "The focus answer is visible"
        }))?)
    }
}

#[derive(Clone, Copy, Debug)]
struct AcceptingRecall;

impl RecallJudge for AcceptingRecall {
    fn review(&self, _image: &[u8]) -> Result<RecallReview> {
        Ok(serde_json::from_value(serde_json::json!({
            "decision": "ALLOW",
            "evidence": [],
            "reason": "No answer-bearing writing is visible"
        }))?)
    }
}

#[derive(Clone)]
struct PaidRecall {
    costs: CostRecorder,
}

impl RecallJudge for PaidRecall {
    fn review(&self, _image: &[u8]) -> Result<RecallReview> {
        self.costs.push(CostRecord::new(
            "gemini-3.5-flash-lite",
            1,
            400,
            25,
            425,
            GenerationCost::from_nanos(50_000),
        ))?;
        AcceptingRecall.review(&[])
    }
}

#[derive(Clone, Copy, Debug)]
struct FailingTranslator;

impl Translator for FailingTranslator {
    fn translate(&self, _sentence: &str, _target: &str) -> Result<Value> {
        bail!("scene composition failed")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum RecoveryFailure {
    #[error("recomposition failed")]
    Recomposition,
    #[error("fallback failed")]
    Fallback,
    #[error("image provider failed")]
    Provider,
    #[error("accounting failed")]
    Accounting,
}

impl From<anyhow::Error> for RecoveryFailure {
    fn from(_error: anyhow::Error) -> Self {
        Self::Accounting
    }
}

fn image_bytes(image: GrayImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("test image must encode");
    bytes.into_inner()
}

fn valid_image() -> Vec<u8> {
    let mut image = GrayImage::from_pixel(32, 32, Luma([255]));
    for y in 2..30 {
        for x in 2..30 {
            image.put_pixel(x, y, Luma([0]));
        }
    }
    image_bytes(image)
}

fn renderable_scene() -> Value {
    serde_json::json!({
        "manga_panel": {
            "canvas": {
                "width": 1024,
                "height": 1024
            },
            "panel_layout": {
                "active_layout": {
                    "template_id": "splash-1-v1"
                }
            },
            "page_design": {
                "special_device": {
                    "kind": "none"
                }
            },
            "panels": [{
                "id": "p1",
                "bounds": {"x": 16, "y": 16, "width": 992, "height": 992},
                "scene": {
                    "description": "One grounded subject performs a visible action",
                    "camera": {
                        "shot_scale": "medium",
                        "viewpoint": "objective",
                        "angle": "eye_level",
                        "depth_plan": "layered"
                    },
                    "lighting": "controlled high-value contrast"
                }
            }]
        }
    })
}

fn picture_requests(cache: &Cache) -> u32 {
    load_picture_request_counter(cache)
        .expect("picture request counter must decode")
        .requests
}

fn picture_series_requests(cache: &Cache) -> u32 {
    let counter = load_picture_request_counter(cache).expect("picture request counter must decode");
    counter.series_requests.unwrap_or(counter.requests)
}

#[derive(Clone)]
struct PersistingSpeaker {
    costs: CostRecorder,
}

impl Speaker for PersistingSpeaker {
    fn speech(&self, _prompt: &str, _text: &str) -> Result<Vec<u8>> {
        self.costs.push(CostRecord::new(
            "gemini-2.5-flash-preview-tts",
            1,
            20,
            40,
            60,
            GenerationCost::from_nanos(700_000),
        ))?;
        Ok(vec![0, 0])
    }
}

mod cost_accounting;
mod picture_recovery;
mod picture_requests;
mod scene_attempt;
mod visual;
