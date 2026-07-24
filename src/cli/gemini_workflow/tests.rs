use std::cell::Cell;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use serde_json::Value;
use tempfile::TempDir;

use super::cost_accounting::*;
use super::picture_requests::*;
use super::visual_generation::*;
use super::*;
use crate::generation::Speaker;
use crate::generation::artifact_cache::{
    ILLUSTRATION_COST_FILE, IMAGE_ATTEMPTS_DIRECTORY, META_COST_FILE, PICTURE_REQUESTS_FILE,
    SCENE_FILE,
};
use crate::generation::manga::{RecallReview, Renderer, Translator};
use crate::session::{ArtifactCosts, CostRecord};

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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[test]
fn production_renderer_spends_one_image_call_per_artifact_attempt() {
    let source = CountingImageSource::new(image_bytes(GrayImage::from_pixel(16, 16, Luma([0]))));
    let renderer = production_renderer(
        source.clone(),
        RejectingRecall,
        BorderDetector::new(2, 6, 240, 2),
    );
    let result = renderer.render(&renderable_scene(), &mut NoopProgress);
    assert_eq!(
        (result.is_err(), source.calls()),
        (true, 1),
        "one outer artifact attempt multiplied into multiple image calls"
    );
}

#[test]
fn pre_provider_scene_cache_and_recomposition_failures_spend_no_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let scene_cache = Cache::new("scene", home.path());
    let cache_cache = Cache::new("cache", home.path());
    let recompose_cache = Cache::new("recompose", home.path());
    let scene_source = CountingImageSource::new(valid_image());
    let cache_source = CountingImageSource::new(valid_image());
    let recompose_source = CountingImageSource::new(valid_image());
    let scene = Illustration::new(
        scene_cache.clone(),
        FailingTranslator,
        MangaRenderer::new(
            RequestCountingImage::new(scene_source.clone(), scene_cache.clone()),
            1,
            AcceptingRecall,
            BorderDetector::new(2, 6, 240, 2),
        ),
    );
    let cached = Illustration::new(
        cache_cache.clone(),
        FailingTranslator,
        MangaRenderer::new(
            RequestCountingImage::new(cache_source.clone(), cache_cache.clone()),
            1,
            AcceptingRecall,
            BorderDetector::new(2, 6, 240, 2),
        ),
    );
    let recompose = Illustration::new(
        recompose_cache.clone(),
        FailingTranslator,
        MangaRenderer::new(
            RequestCountingImage::new(recompose_source.clone(), recompose_cache.clone()),
            1,
            AcceptingRecall,
            BorderDetector::new(2, 6, 240, 2),
        ),
    );
    let scene_result = scene.scene_only("sentence", "en", &mut NoopProgress);
    let cache_result = cached.picture_only("sentence", "en", &mut NoopProgress);
    let recompose_result =
        recompose.picture_with_recomposed_scene("sentence", "en", &mut NoopProgress);
    assert_eq!(
        (
            scene_result.is_err(),
            cache_result.is_err(),
            recompose_result.is_err(),
            scene_source.calls(),
            cache_source.calls(),
            recompose_source.calls(),
            picture_requests(&scene_cache),
            picture_requests(&cache_cache),
            picture_requests(&recompose_cache),
        ),
        (true, true, true, 0, 0, 0, 0, 0, 0),
        "a pre-provider failure consumed or recorded an image request"
    );
}

#[test]
fn pre_provider_recomposition_failure_falls_back_once_to_the_committed_scene() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let accounting = AccountingHealth::default();
    let fallback = Cell::new(0_u8);
    let mut progress = ();
    let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| Err(RecoveryFailure::Recomposition),
        |_| {
            fallback.set(fallback.get().saturating_add(1));
            reserve_picture_request(&cache)?;
            Ok("committed")
        },
    );
    assert_eq!(
        (result.ok(), fallback.get(), picture_requests(&cache)),
        (Some("committed"), 1, 1),
        "pre-provider recomposition failure did not produce exactly one committed-scene image"
    );
}

#[test]
fn recomposition_image_failure_never_falls_back_to_the_committed_scene() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let accounting = AccountingHealth::default();
    let fallback = Cell::new(0_u8);
    let mut progress = ();
    let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| {
            reserve_picture_request(&cache)?;
            Err(RecoveryFailure::Provider)
        },
        |_| {
            fallback.set(fallback.get().saturating_add(1));
            Ok("committed")
        },
    );
    assert_eq!(
        (result.err(), fallback.get(), picture_requests(&cache),),
        (Some(RecoveryFailure::Provider), 0, 1),
        "an image failure triggered an extra fallback provider call"
    );
}

#[test]
fn fallback_failure_before_the_provider_returns_the_recomposition_error() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let accounting = AccountingHealth::default();
    let fallback = Cell::new(0_u8);
    let mut progress = ();
    let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| Err(RecoveryFailure::Recomposition),
        |_| {
            fallback.set(fallback.get().saturating_add(1));
            Err(RecoveryFailure::Fallback)
        },
    );
    assert_eq!(
        (result.err(), fallback.get(), picture_requests(&cache)),
        (Some(RecoveryFailure::Recomposition), 1, 0),
        "a pre-provider fallback failure hid the original recomposition diagnosis"
    );
}

#[test]
fn fallback_image_failure_returns_the_fallback_provider_error() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let accounting = AccountingHealth::default();
    let mut progress = ();
    let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| Err(RecoveryFailure::Recomposition),
        |_| {
            reserve_picture_request(&cache)?;
            Err(RecoveryFailure::Provider)
        },
    );
    assert_eq!(
        (result.err(), picture_requests(&cache)),
        (Some(RecoveryFailure::Provider), 1),
        "a fallback image failure was replaced by a stale scene diagnosis"
    );
}

#[test]
fn cost_recording_failure_prevents_committed_scene_fallback() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let accounting = AccountingHealth::default();
    let costs = CostRecorder::guarded(
        Cache::failing("costs", home.path(), 0),
        Artifact::Scene,
        None,
        accounting.clone(),
    );
    let fallback = Cell::new(0_u8);
    let mut progress = ();
    let result: Result<&str> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| {
            costs
                .push(CostRecord::new(
                    "gemini-3.6-flash",
                    1,
                    100,
                    20,
                    120,
                    GenerationCost::from_nanos(300_000),
                ))
                .map_err(|error| error.context("scene composition request failed"))?;
            Ok("recomposed")
        },
        |_| {
            fallback.set(fallback.get().saturating_add(1));
            Ok("committed")
        },
    );
    assert_eq!(
        (
            result.is_err(),
            fallback.get(),
            picture_requests(&cache),
            accounting.failed(),
        ),
        (true, 0, 0, true),
        "durable cost failure was hidden by a committed-scene fallback"
    );
}

#[test]
fn request_recording_failure_prevents_committed_scene_fallback() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::failing("visual", home.path(), 0);
    let accounting = AccountingHealth::default();
    let fallback = Cell::new(0_u8);
    let source = CountingImageSource::new(valid_image());
    let image = RequestCountingImage::guarded(source.clone(), cache.clone(), accounting.clone());
    let mut progress = ();
    let result: Result<&str> = render_recomposition_with_fallback(
        &cache,
        &accounting,
        &mut progress,
        |_| {
            image.image("compiled image prompt")?;
            Ok("recomposed")
        },
        |_| {
            fallback.set(fallback.get().saturating_add(1));
            Ok("committed")
        },
    );
    assert_eq!(
        (
            result.is_err(),
            source.calls(),
            fallback.get(),
            picture_requests(&cache),
            accounting.failed(),
        ),
        (true, 0, 0, 0, true),
        "durable picture reservation failure was hidden by a committed-scene fallback"
    );
}

#[test]
fn transport_failure_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = FailingImageSource::new("transport failed");
    let image = RequestCountingImage::new(source.clone(), cache.clone());
    let result = image.image("compiled image prompt");
    assert_eq!(
        (result.is_err(), source.calls(), picture_requests(&cache)),
        (true, 1, 1),
        "a transport failure was not counted exactly once"
    );
}

#[test]
fn non_success_response_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = FailingImageSource::new("INVALID_ARGUMENT: request rejected");
    let image = RequestCountingImage::new(source.clone(), cache.clone());
    let result = image.image("compiled image prompt");
    assert_eq!(
        (result.is_err(), source.calls(), picture_requests(&cache)),
        (true, 1, 1),
        "a non-success provider response was not counted exactly once"
    );
}

#[test]
fn successful_response_without_usage_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
    let source = UsageFreeImageSource::new(costs, valid_image());
    let image = RequestCountingImage::new(source, cache.clone());
    let result = image.image("compiled image prompt");
    assert_eq!(
        (
            result.is_ok(),
            picture_requests(&cache),
            cache.exists(ILLUSTRATION_COST_FILE),
        ),
        (true, 1, false),
        "missing usage metadata erased the provider request or invented a cost record"
    );
}

#[test]
fn undecodable_response_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = CountingImageSource::new(b"not an image".to_vec());
    let renderer = MangaRenderer::new(
        RequestCountingImage::new(source.clone(), cache.clone()),
        1,
        AcceptingRecall,
        BorderDetector::new(2, 6, 240, 2),
    );
    let result = renderer.render(&renderable_scene(), &mut NoopProgress);
    assert_eq!(
        (result.is_err(), source.calls(), picture_requests(&cache)),
        (true, 1, 1),
        "an undecodable response was not counted exactly once"
    );
}

#[test]
fn validation_rejection_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = CountingImageSource::new(valid_image());
    let renderer = MangaRenderer::new(
        RequestCountingImage::new(source.clone(), cache.clone()),
        1,
        RejectingRecall,
        BorderDetector::new(2, 6, 240, 2),
    );
    let result = renderer.render(&renderable_scene(), &mut NoopProgress);
    assert_eq!(
        (result.is_err(), source.calls(), picture_requests(&cache)),
        (true, 1, 1),
        "a rejected image response was not counted exactly once"
    );
}

#[test]
fn accepted_response_spends_one_picture_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = CountingImageSource::new(valid_image());
    let renderer = MangaRenderer::new(
        RequestCountingImage::new(source.clone(), cache.clone()),
        1,
        AcceptingRecall,
        BorderDetector::new(2, 6, 240, 2),
    );
    let result = renderer.render(&renderable_scene(), &mut NoopProgress);
    assert_eq!(
        (result.is_ok(), source.calls(), picture_requests(&cache)),
        (true, 1, 1),
        "an accepted image response was not counted exactly once"
    );
}

#[test]
fn picture_cost_includes_recall_review_without_inflating_image_request_count() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
    let source = PaidImageSource {
        costs: costs.clone(),
        image: valid_image(),
    };
    let renderer = MangaRenderer::new(
        RequestCountingImage::new(source, cache.clone()),
        1,
        PaidRecall {
            costs: costs.clone(),
        },
        BorderDetector::new(2, 6, 240, 2),
    );
    let result = renderer.render(&renderable_scene(), &mut NoopProgress);
    let record = load_cost_record(&cache, Artifact::Picture)
        .expect("picture cost must decode")
        .expect("picture cost must exist");
    assert_eq!(
        (
            result.is_ok(),
            picture_requests(&cache),
            record.requests(),
            record.model().to_string(),
            record.cost(),
        ),
        (
            true,
            1,
            2,
            String::from("gemini-3.1-flash-image,gemini-3.5-flash-lite"),
            GenerationCost::from_nanos(950_000),
        ),
        "recall review cost was hidden from Picture or counted as another image generation"
    );
}

#[test]
fn picture_request_ceiling_survives_a_fresh_generator_instance() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    let source = FailingImageSource::new("transport failed");
    let first = RequestCountingImage::new(source.clone(), cache.clone());
    for _ in 0..3 {
        let _ = first.image("compiled image prompt");
    }
    let restarted = RequestCountingImage::new(source.clone(), cache.clone());
    let fourth = restarted.image("compiled image prompt");
    assert_eq!(
        (
            fourth.is_err(),
            source.calls(),
            picture_requests(&cache),
            picture_series_requests(&cache),
        ),
        (true, 3, 3, 3),
        "a fresh generator instance expanded the durable picture ceiling"
    );
}

#[test]
fn legacy_picture_counter_defaults_to_one_unfinished_series() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", home.path());
    fs::write(
        cache
            .filepath(PICTURE_REQUESTS_FILE)
            .expect("counter path must resolve"),
        br#"{"schema":"kamishibai.picture-request-counter","version":1,"requests":3}"#,
    )
    .expect("legacy counter must be written");
    let source = CountingImageSource::new(valid_image());
    let image = RequestCountingImage::new(source.clone(), cache.clone());
    let result = image.image("compiled image prompt");
    assert_eq!(
        (
            result.is_err(),
            source.calls(),
            picture_requests(&cache),
            picture_series_requests(&cache),
        ),
        (true, 0, 3, 3),
        "a legacy counter silently opened an unauthorized picture series"
    );
}

#[test]
fn picture_counter_write_failure_prevents_the_provider_call() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::failing("visual", home.path(), 0);
    let source = CountingImageSource::new(valid_image());
    let image = RequestCountingImage::new(source.clone(), cache.clone());
    let result = image.image("compiled image prompt");
    assert_eq!(
        (
            result.is_err(),
            source.calls(),
            picture_requests(&cache),
            cache.exists(PICTURE_REQUESTS_FILE),
        ),
        (true, 0, 0, false),
        "the provider was called after its durable request reservation failed"
    );
}

#[test]
fn two_recall_text_rejections_keep_the_third_attempt_on_the_current_scene() {
    let recovery = PictureRecovery::default();
    let path = Path::new("cards/local-rejections");
    let first = recovery
        .prepare(path, 0)
        .expect("first attempt must prepare");
    recovery
        .observe(path, 0, Some(LocalImageRejection::RecallText))
        .expect("first local rejection must record");
    let second = recovery
        .prepare(path, 1)
        .expect("second attempt must prepare");
    recovery
        .observe(path, 1, Some(LocalImageRejection::RecallText))
        .expect("second local rejection must record");
    let third = recovery
        .prepare(path, 2)
        .expect("third attempt must prepare");
    assert_eq!(
        (first, second, third),
        (false, false, false),
        "recall-text failures discarded a scene that could still render without text"
    );
}

#[test]
fn two_border_rejections_enable_third_attempt_recomposition() {
    let recovery = PictureRecovery::default();
    let path = Path::new("cards/repeated-border");
    recovery
        .prepare(path, 0)
        .expect("first attempt must prepare");
    recovery
        .observe(path, 0, Some(LocalImageRejection::Border))
        .expect("first border rejection must record");
    recovery.prepare(path, 1).expect("retry must prepare");
    recovery
        .observe(path, 1, Some(LocalImageRejection::Border))
        .expect("second border rejection must record");
    assert!(
        recovery
            .prepare(path, 2)
            .expect("third attempt must prepare"),
        "repeated border failures did not advance the third picture to a fresh scene"
    );
}

#[test]
fn two_color_rejections_keep_the_third_attempt_on_the_current_scene() {
    let recovery = PictureRecovery::default();
    let path = Path::new("cards/repeated-color");
    recovery
        .prepare(path, 0)
        .expect("first attempt must prepare");
    recovery
        .observe(path, 0, Some(LocalImageRejection::Color))
        .expect("first color rejection must record");
    recovery.prepare(path, 1).expect("retry must prepare");
    recovery
        .observe(path, 1, Some(LocalImageRejection::Color))
        .expect("second color rejection must record");
    assert!(
        !recovery
            .prepare(path, 2)
            .expect("third attempt must prepare"),
        "repeated color failures discarded a scene whose composition was not implicated"
    );
}

#[test]
fn mixed_border_then_ocr_rejections_keep_the_third_attempt_on_the_current_scene() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "border");
    write_rejection(temporary.path(), 2, "ocr");
    assert!(
        !PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("mixed local verdicts must decode"),
        "border then OCR discarded a scene that could still render cleanly"
    );
}

#[test]
fn mixed_topology_then_ocr_rejections_enable_third_attempt_recomposition() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "topology");
    write_rejection(temporary.path(), 2, "ocr");
    assert!(
        PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("mixed local verdicts must decode"),
        "topology evidence followed by OCR did not advance the third picture to a fresh scene"
    );
}

#[test]
fn two_topology_rejections_enable_third_attempt_recomposition() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "topology");
    write_rejection(temporary.path(), 2, "topology");
    assert!(
        PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("topology verdicts must decode"),
        "repeated topology rejections did not recompose the third picture attempt"
    );
}

#[test]
fn one_topology_rejection_keeps_the_second_attempt_on_the_current_scene() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "topology");
    assert!(
        !PictureRecovery::default()
            .prepare(temporary.path(), 1)
            .expect("topology verdict must decode"),
        "one noisy topology verdict discarded the scene before an image retry"
    );
}

#[test]
fn persisted_topology_rejection_keeps_the_second_picture_on_the_committed_scene_slot() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", temporary.path());
    let scene = serde_json::json!({
        "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 0}}}
    });
    fs::write(
        cache.filepath(SCENE_FILE).expect("scene path must resolve"),
        serde_json::to_vec(&scene).expect("scene provenance must encode"),
    )
    .expect("scene provenance must be written");
    let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
    fs::write(
        attempts.join("attempt-0001.scene.json"),
        serde_json::to_vec(&scene).expect("attempt scene must encode"),
    )
    .expect("attempt scene must be written");
    write_rejection(cache.path().as_path(), 1, "topology");
    let recover = PictureRecovery::default()
        .prepare(cache.path().as_path(), 1)
        .expect("persisted topology verdict must decode");
    let selected = reserve_scene_attempt(&cache, Artifact::Picture, 0, recover)
        .expect("second picture must retain the committed scene");
    assert_eq!(
        (
            recover,
            selected,
            load_scene_attempt(&cache).expect("cursor must decode")
        ),
        (false, 0, Some(0)),
        "a restarted second picture advanced after only one topology verdict"
    );
}

#[test]
fn provider_failure_does_not_count_as_a_local_rejection() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let recovery = PictureRecovery::default();
    recovery
        .prepare(temporary.path(), 0)
        .expect("first attempt must prepare");
    recovery
        .observe(temporary.path(), 0, Some(LocalImageRejection::Border))
        .expect("first local rejection must record");
    recovery
        .prepare(temporary.path(), 1)
        .expect("provider attempt must prepare");
    recovery
        .observe(temporary.path(), 1, None)
        .expect("provider failure must record");
    assert!(
        !recovery
            .prepare(temporary.path(), 2)
            .expect("third attempt must prepare"),
        "one provider failure was misclassified as a second local rejection"
    );
}

#[test]
fn persisted_decode_failure_does_not_count_as_a_local_rejection() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "border");
    write_verdict(temporary.path(), 2, "error", "transport_or_decode");
    assert!(
        !PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("persisted image outcomes must decode"),
        "a decode failure was misclassified as a second local rejection"
    );
}

#[test]
fn topology_then_ocr_recomposition_survives_a_fresh_generator_process() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "topology");
    write_rejection(temporary.path(), 2, "ocr");
    assert!(
        PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("persisted local verdicts must decode"),
        "a restarted generator forgot topology evidence before the third picture"
    );
}

#[test]
fn repeated_border_recomposition_survives_a_fresh_generator_process() {
    let temporary = TempDir::new().expect("tempdir must be created");
    write_rejection(temporary.path(), 1, "border");
    write_rejection(temporary.path(), 2, "border");
    assert!(
        PictureRecovery::default()
            .prepare(temporary.path(), 2)
            .expect("persisted border verdicts must decode"),
        "a restarted generator forgot repeated border evidence before the third picture"
    );
}

/// Persist one deterministic local image-rejection verdict for restart tests.
fn write_rejection(path: &Path, sequence: usize, category: &str) {
    write_verdict(path, sequence, "rejected", category);
}

/// Persist one deterministic image-attempt verdict for recovery tests.
fn write_verdict(path: &Path, sequence: usize, status: &str, category: &str) {
    let attempts = path.join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
    fs::write(
        attempts.join(format!("attempt-{sequence:04}.json")),
        serde_json::to_vec(&serde_json::json!({
            "sequence": sequence,
            "status": status,
            "category": category,
            "reason": "deterministic image-attempt verdict"
        }))
        .expect("attempt verdict must encode"),
    )
    .expect("attempt verdict must be written");
}

#[test]
fn malformed_persisted_verdict_does_not_invent_a_second_rejection() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let attempts = temporary.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
    fs::write(
        attempts.join("attempt-0001.json"),
        serde_json::to_vec(&serde_json::json!({
            "status": "rejected",
            "category": "ocr"
        }))
        .expect("attempt verdict must encode"),
    )
    .expect("attempt verdict must be written");
    fs::write(attempts.join("attempt-0002.json"), b"{").expect("broken verdict must be written");
    assert!(
        !PictureRecovery::default()
            .prepare(temporary.path(), 0)
            .expect("a broken verdict must degrade to one known rejection"),
        "a partial verdict invented a second local image rejection"
    );
}

#[test]
fn scene_recovery_advances_from_persisted_layout_attempt() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", temporary.path());
    fs::write(
        cache.filepath(SCENE_FILE).expect("scene path must resolve"),
        serde_json::to_vec(&serde_json::json!({
            "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 4}}}
        }))
        .expect("scene provenance must encode"),
    )
    .expect("scene provenance must be written");
    assert_eq!(
        scene_attempt_cursor(&cache, 0)
            .expect("scene provenance must decode")
            .committed,
        Some(4_u8),
        "a restarted recovery returned to an already failed layout slot"
    );
}

#[test]
fn rejected_recomposition_advances_beyond_the_archived_layout_attempt() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", temporary.path());
    fs::write(
        cache.filepath(SCENE_FILE).expect("scene path must resolve"),
        serde_json::to_vec(&serde_json::json!({
            "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 4}}}
        }))
        .expect("scene provenance must encode"),
    )
    .expect("scene provenance must be written");
    let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(attempts.as_path()).expect("attempt archive must be created");
    fs::write(
        attempts.join("attempt-0007.scene.json"),
        serde_json::to_vec(&serde_json::json!({
            "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 5}}}
        }))
        .expect("rejected scene provenance must encode"),
    )
    .expect("rejected scene provenance must be written");
    let cursor = scene_attempt_cursor(&cache, 0).expect("attempt archive must decode");
    assert_eq!(
        (
            cursor.has_rejected_recomposition(),
            cursor.current(0),
            cursor.next(0).expect("alternate slot must select"),
        ),
        (true, 4, 6),
        "regeneration repeated an already rejected scene alternate"
    );
}

#[test]
fn fresh_worker_advances_beyond_three_durably_reserved_scene_failures() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", temporary.path());
    let first = reserve_scene_attempt(&cache, Artifact::Scene, 0, false)
        .expect("first attempt must reserve");
    let second = reserve_scene_attempt(&cache, Artifact::Scene, 1, false)
        .expect("second attempt must reserve");
    let third = reserve_scene_attempt(&cache, Artifact::Scene, 2, false)
        .expect("third attempt must reserve");
    let restarted = reserve_scene_attempt(&cache, Artifact::Scene, 0, false)
        .expect("fresh worker must reserve");
    let stored = load_scene_attempt(&cache).expect("cursor must decode");
    assert_eq!(
        (first, second, third, restarted, stored),
        (0, 1, 2, 3, Some(3)),
        "a fresh worker reused a scene slot whose composer call already failed"
    );
}

#[test]
fn newly_committed_scene_is_rendered_before_old_rejections_recompose_again() {
    let cursor = SceneAttemptCursor {
        committed: Some(6),
        archived: Some(5),
        attempted: Some(6),
    };
    assert!(
        !cursor.recompose(true),
        "a crash-safe committed scene was skipped before its first image attempt"
    );
}

#[test]
fn picture_recovery_state_is_isolated_by_visual_cache_path() {
    let recovery = PictureRecovery::default();
    let first = Path::new("cards/first");
    let second = Path::new("cards/second");
    recovery.prepare(first, 0).expect("first card must prepare");
    recovery
        .observe(first, 0, Some(LocalImageRejection::Topology))
        .expect("first card outcome must record");
    recovery
        .prepare(first, 1)
        .expect("first retry must prepare");
    recovery
        .observe(first, 1, Some(LocalImageRejection::Topology))
        .expect("first retry outcome must record");
    recovery
        .prepare(second, 0)
        .expect("second card must prepare");
    recovery
        .observe(second, 0, Some(LocalImageRejection::Topology))
        .expect("second card outcome must record");
    assert_eq!(
        (
            recovery
                .prepare(first, 2)
                .expect("first third attempt must prepare"),
            recovery
                .prepare(second, 1)
                .expect("second retry must prepare"),
        ),
        (true, false),
        "one card's local rejection count contaminated another visual cache"
    );
}

#[test]
fn a_fresh_picture_tally_resets_stale_recovery_state() {
    let recovery = PictureRecovery::default();
    let path = Path::new("cards/rerolled");
    recovery
        .prepare(path, 0)
        .expect("first attempt must prepare");
    recovery
        .observe(path, 0, Some(LocalImageRejection::RecallText))
        .expect("first outcome must record");
    recovery.prepare(path, 1).expect("retry must prepare");
    recovery
        .observe(path, 1, Some(LocalImageRejection::RecallText))
        .expect("retry outcome must record");
    let reset = recovery.prepare(path, 0).expect("reroll must reset");
    recovery
        .observe(path, 0, Some(LocalImageRejection::RecallText))
        .expect("new first outcome must record");
    assert_eq!(
        (
            reset,
            recovery.prepare(path, 1).expect("new retry must prepare")
        ),
        (false, false),
        "a rerolled card inherited the previous picture series"
    );
}

#[test]
fn duplicate_visual_paths_hold_one_lock_without_deadlocking() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path())
        .visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .expect("visual cache must resolve");
    let guards = hold_visuals(vec![cache.clone(), cache], Duration::ZERO)
        .expect("duplicate visual paths must acquire one lock");
    assert_eq!(
        guards.len(),
        1,
        "duplicate visual paths acquired the same non-reentrant lock twice"
    );
}

#[test]
fn correction_observer_persists_the_exact_billed_request() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let record = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    persist_correction_cost(&cache, &record).expect("correction cost must persist");
    assert_eq!(
        load_cost_record(&cache, Artifact::Meta)
            .expect("meta cost must decode")
            .map(|stored| (stored.requests(), stored.cost())),
        Some((1, GenerationCost::from_nanos(300_000))),
        "correction observer discarded or inflated its exact request"
    );
}

#[test]
fn provider_observer_journals_session_spend_before_lifetime_sidecar_failure() {
    let home = TempDir::new().expect("tempdir must be created");
    let scope = SessionCostScope::for_run(home.path(), "fr-1", "created-a");
    scope.overlay(&[]).expect("session journal must seed");
    let recorder = CostRecorder::attributed(
        Cache::failing("cards/test", home.path(), 0),
        Artifact::Picture,
        Some(SessionCostAttribution::new(scope.clone(), 0)),
    );
    let result = recorder.push(CostRecord::new(
        "gemini-3.1-flash-image",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(700_000),
    ));
    assert_eq!(
        (
            result.is_err(),
            scope
                .absolute(0, ArtifactCosts::default())
                .expect("journal must remain readable")
                .cost(Artifact::Picture),
        ),
        (true, Some(GenerationCost::from_nanos(700_000))),
        "lifetime sidecar failure happened before session spend became durable"
    );
}

#[test]
fn correction_cost_waits_for_the_stable_meta_lease() {
    let Some(root) = std::env::var_os("KAMISHIBAI_CORRECTION_LOCK_ROOT") else {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let guard = cache
            .hold_root_stage(RootStage::Meta, Duration::ZERO)
            .expect("meta lease must be acquired");
        let mut child = Command::new(std::env::current_exe().expect("test binary must resolve"))
            .args([
                "cli::gemini_workflow::tests::correction_cost_waits_for_the_stable_meta_lease",
                "--exact",
                "--nocapture",
            ])
            .env("KAMISHIBAI_CORRECTION_LOCK_ROOT", home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cost writer child must spawn");
        std::thread::sleep(Duration::from_millis(100));
        let waited = child
            .try_wait()
            .expect("child state must be observable")
            .is_none()
            && !cache.exists(META_COST_FILE);
        drop(guard);
        let deadline = Instant::now() + Duration::from_secs(5);
        let succeeded = loop {
            if let Some(status) = child.try_wait().expect("child state must be observable") {
                break status.success();
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            (waited, succeeded, cache.exists(META_COST_FILE)),
            (true, true, true),
            "correction cost bypassed the stable meta lease or failed after it released"
        );
        return;
    };
    let cache = Cache::new("cards/test", PathBuf::from(root));
    let record = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let stored = persist_correction_cost(&cache, &record);
    assert!(
        stored.is_ok() && cache.exists(META_COST_FILE),
        "child failed to persist billed correction cost under the meta lease"
    );
}

#[test]
fn cached_artifacts_do_not_report_historical_cost() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let record = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    store_cost(&cache, Artifact::Sound, &record).expect("cost must persist");
    let costs = CostRecorder::new(cache, Artifact::Sound);
    assert_eq!(
        (
            costs.cumulative(true).expect("cache cost must settle"),
            costs.cumulative(false).expect("run cost must settle"),
        ),
        (None, None),
        "cache hits must not count historical Gemini cost as current spend"
    );
}

#[test]
fn fresh_artifacts_report_current_request_cost() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let record = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let costs = CostRecorder::new(cache, Artifact::Sound);
    costs.push(record).expect("cost must persist");
    assert_eq!(
        costs.cumulative(false).expect("cost must settle"),
        Some(GenerationCost::from_nanos(300_000)),
        "fresh Gemini requests must report their current spend"
    );
}

#[test]
fn a_new_run_does_not_inherit_historical_sidecar_cost() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let historical = CostRecord::new(
        "gemini-3.6-flash",
        2,
        200,
        40,
        240,
        GenerationCost::from_nanos(900_000),
    );
    let current = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    store_cost(&cache, Artifact::Sound, &historical).expect("historical cost must persist");
    let costs = CostRecorder::new(cache.clone(), Artifact::Sound);
    costs.push(current).expect("current cost must persist");
    assert_eq!(
        (
            costs.cumulative(false).expect("run cost must load"),
            load_cost(&cache, Artifact::Sound).expect("lifetime cost must load"),
        ),
        (
            Some(GenerationCost::from_nanos(300_000)),
            Some(GenerationCost::from_nanos(1_200_000)),
        ),
        "a fresh generation run inherited the card's lifetime sidecar spend"
    );
}

#[test]
fn fresh_artifacts_report_accumulated_retry_cost() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let first = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let second = CostRecord::new(
        "gemini-3.6-flash",
        1,
        40,
        10,
        50,
        GenerationCost::from_nanos(135_000),
    );
    let costs = CostRecorder::new(cache, Artifact::Sound);
    costs.push(first).expect("first cost must persist");
    costs.push(second).expect("retry cost must persist");
    assert_eq!(
        costs.cumulative(false).expect("retry cost must settle"),
        Some(GenerationCost::from_nanos(435_000)),
        "fresh retry success must report all successful Gemini requests for the artifact"
    );
}

#[test]
fn one_operation_reports_all_observed_request_cost() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let first = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let second = CostRecord::new(
        "gemini-3.6-flash",
        1,
        40,
        10,
        50,
        GenerationCost::from_nanos(135_000),
    );
    let costs = CostRecorder::new(cache, Artifact::Sound);
    costs.push(first).expect("first cost must persist");
    let first_cost = costs.cumulative(false).expect("first cost must settle");
    let unmetered = costs.cumulative(false).expect("cost must remain");
    costs.push(second).expect("second cost must persist");
    let second_cost = costs.cumulative(false).expect("second cost must settle");
    assert_eq!(
        (first_cost, unmetered, second_cost),
        (
            Some(GenerationCost::from_nanos(300_000)),
            Some(GenerationCost::from_nanos(300_000)),
            Some(GenerationCost::from_nanos(435_000)),
        ),
        "one provider operation did not return all observed request spend"
    );
}

#[test]
fn missing_usage_records_do_not_report_zero_costs() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let record = CostRecord::new("gemini-3.6-flash", 0, 0, 0, 0, GenerationCost::zero());
    let costs = CostRecorder::new(cache, Artifact::Sound);
    costs.push(record.clone()).expect("zero usage must settle");
    let fresh = costs.cumulative(false).expect("zero usage must settle");
    costs.push(record).expect("zero usage retry must settle");
    let retry = costs
        .cumulative(false)
        .expect("zero usage retry must settle");
    assert_eq!(
        (fresh, retry),
        (None, None),
        "missing Gemini usage metadata must leave the request cost absent"
    );
}

#[test]
fn recomposition_persists_scene_and_picture_spend_in_separate_sidecars() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/test", home.path());
    let scene = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let picture = CostRecord::new(
        "gemini-3.1-flash-image",
        1,
        40,
        10,
        50,
        GenerationCost::from_nanos(900_000),
    );
    let scene_costs = CostRecorder::new(cache.clone(), Artifact::Scene);
    let picture_costs = CostRecorder::new(cache.clone(), Artifact::Picture);
    scene_costs.push(scene).expect("scene cost must persist");
    picture_costs
        .push(picture)
        .expect("picture cost must persist");
    let costs = visual_costs(Artifact::Picture, false, &scene_costs, &picture_costs)
        .expect("visual costs must load");
    assert_eq!(
        (
            costs,
            load_cost_record(&cache, Artifact::Scene)
                .expect("scene cost must decode")
                .map(|record| (record.model().to_string(), record.requests())),
            load_cost_record(&cache, Artifact::Picture)
                .expect("picture cost must decode")
                .map(|record| (record.model().to_string(), record.requests())),
        ),
        (
            (
                Some(GenerationCost::from_nanos(900_000)),
                Some(GenerationCost::from_nanos(300_000)),
            ),
            Some((String::from("gemini-3.6-flash"), 1)),
            Some((String::from("gemini-3.1-flash-image"), 1)),
        ),
        "recomposition mixed the scene request into picture accounting"
    );
}

#[test]
fn metering_refuses_to_continue_after_cost_persistence_fails() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::failing("cards/test", home.path(), 0);
    let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
    let record = CostRecord::new(
        "gemini-3.1-flash-image",
        1,
        40,
        10,
        50,
        GenerationCost::from_nanos(900_000),
    );
    let result = costs.push(record);
    assert_eq!(
        (result.is_err(), cache.exists(ILLUSTRATION_COST_FILE)),
        (true, false),
        "a cost persistence failure was hidden from the provider boundary"
    );
}

#[test]
fn cost_persistence_failure_prevents_the_artifact_commit() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::failing("cards/test", home.path(), 0);
    let costs = CostRecorder::new(cache.clone(), Artifact::Sound);
    let audio = Audio::new(cache.clone(), "Read {text}", PersistingSpeaker { costs });
    let result = audio.generate("hello");
    assert_eq!(
        (result.is_err(), cache.exists(VOICE_FILE)),
        (true, false),
        "an audio artifact committed after its usage record failed to persist"
    );
}
