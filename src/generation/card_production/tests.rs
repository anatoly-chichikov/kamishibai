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
use super::invalidation::ArtifactGuards;
use super::picture_recovery::*;
use super::picture_requests::*;
use super::scene_attempt::*;
use super::visual::judged;
use super::*;
use crate::application::GenerationCostLedger;
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_COST_FILE, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, META_COST_FILE,
    PICTURE_REQUESTS_FILE, RootStage, SCENE_ATTEMPT_FILE, SCENE_COST_FILE, SCENE_FILE,
    VOICE_COST_FILE, VOICE_FILE,
};
use crate::generation::manga::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, RecallJudge, RecallReview, Renderer,
    Translator,
};
use crate::generation::visual_revision;
use crate::generation::{Audio, Speaker};
use crate::session::{
    Artifact, ArtifactCosts, ArtifactFile, AxisSet, CardCell, CardMetaCache, CostRecord,
    GenerationCost, Register, SentenceAxis, SentenceKind, SentenceLabelSelection, SentenceLabels,
    SentenceLevel,
};

#[test]
fn localized_meta_refresh_removes_legacy_audio_and_preserves_its_cost() {
    let directory = TempDir::new().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let term = "canard";
    let understanding = "a false newspaper story";
    let cache = CardMetaCache::new(directory.path());
    cache
        .store(
            term,
            understanding,
            &pair,
            &card_meta("The old sentence used canard"),
        )
        .expect("legacy meta seed must store");
    let cell = CardCell::new(directory.path(), &pair, term, understanding).cache();
    let meta_path = cell
        .filepath(crate::generation::artifact_cache::META_FILE)
        .expect("meta path must resolve");
    let mut legacy = serde_json::from_slice::<Value>(
        &fs::read(&meta_path).expect("meta seed must remain readable"),
    )
    .expect("meta seed must decode");
    legacy
        .as_object_mut()
        .expect("meta seed must be an object")
        .remove("policy");
    fs::write(
        &meta_path,
        serde_json::to_vec_pretty(&legacy).expect("legacy meta must encode"),
    )
    .expect("legacy meta must store");
    fs::write(
        cell.filepath(VOICE_FILE)
            .expect("legacy audio path must resolve"),
        b"old audio",
    )
    .expect("legacy audio must store");
    fs::write(
        cell.filepath(VOICE_COST_FILE)
            .expect("legacy audio cost path must resolve"),
        b"old cost",
    )
    .expect("legacy audio cost must store");
    let production = MetadataProduction::new(
        directory.path().to_path_buf(),
        GeminiAccess::console(),
        CostAccounting::new(None),
    );
    let file = production
        .store(
            term,
            understanding,
            &pair,
            &card_meta("The localized sentence uses canard"),
        )
        .expect("localized meta must replace legacy meta");
    let refreshed = cache
        .load(term, understanding, &pair)
        .expect("refreshed meta lookup must succeed")
        .expect("refreshed meta must exist");
    assert_eq!(
        (
            file.cached(),
            refreshed.target_sentence(),
            cell.exists(VOICE_FILE),
            cell.exists(VOICE_COST_FILE)
        ),
        (false, "The localized sentence uses canard", false, true),
        "localized meta refresh retained legacy audio, erased its cost, or failed to replace meta"
    );
}

#[test]
fn matching_cached_labels_are_pinned_locally_without_invalidating_media() {
    let directory = TempDir::new().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let term = "canard";
    let understanding = "a duck";
    let meta = labeled_meta(
        SentenceLevel::B1,
        SentenceKind::Question,
        AxisSet::default(),
    );
    CardMetaCache::new(directory.path())
        .store(term, understanding, &pair, &meta)
        .expect("matching meta must be seeded");
    let cell = CardCell::new(directory.path(), &pair, term, understanding).cache();
    let visual = cell
        .visual(visual_revision())
        .expect("visual revision must resolve");
    seed_refresh_files(&cell, &visual);
    let request = SentenceLabelSelection::empty()
        .choosing(SentenceAxis::Level, 2)
        .choosing(SentenceAxis::Type, 1);
    let production = MetadataProduction::new(
        directory.path().to_path_buf(),
        GeminiAccess::unavailable(),
        CostAccounting::new(None),
    );
    let attempt = production.generate(term, understanding, &pair, Some(&request), None);
    let generated = attempt
        .into_result()
        .expect("matching cached meta must not need Gemini")
        .0;
    let labels = generated
        .sentence_labels()
        .expect("locally reconciled meta must retain labels");
    assert_eq!(
        (
            labels.pinned().contains(SentenceAxis::Level),
            labels.pinned().contains(SentenceAxis::Type),
            cell.exists(VOICE_FILE),
            visual.exists(SCENE_FILE),
            visual.exists(ILLUSTRATION_FILE),
            visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
        ),
        (true, true, true, true, true, true),
        "matching cached labels called Gemini or invalidated reusable media"
    );
}

#[test]
fn failed_requested_refresh_keeps_the_old_meta_and_every_dependent_artifact() {
    let directory = TempDir::new().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let term = "canard";
    let understanding = "a duck";
    let old = card_meta("An approximately fulfilled a2 sentence").with_sentence_labels(
        SentenceLabels::new(
            Register::Neutral,
            SentenceLevel::A2,
            SentenceKind::Statement,
            AxisSet::from_axes([SentenceAxis::Level]),
            AxisSet::from_axes([SentenceAxis::Level]),
        ),
    );
    CardMetaCache::new(directory.path())
        .store(term, understanding, &pair, &old)
        .expect("old meta must be seeded");
    let cell = CardCell::new(directory.path(), &pair, term, understanding).cache();
    let visual = cell
        .visual(visual_revision())
        .expect("visual revision must resolve");
    seed_refresh_files(&cell, &visual);
    let request = SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 2);
    let production = MetadataProduction::new(
        directory.path().to_path_buf(),
        GeminiAccess::unavailable(),
        CostAccounting::new(None),
    );
    let attempt = production.generate(term, understanding, &pair, Some(&request), None);
    let retained = CardMetaCache::new(directory.path())
        .load(term, understanding, &pair)
        .expect("old meta must remain readable")
        .expect("old meta must remain cached");
    assert_eq!(
        (
            attempt.error().is_some(),
            retained.target_sentence(),
            cell.exists(VOICE_FILE),
            visual.exists(SCENE_FILE),
            visual.exists(ILLUSTRATION_FILE),
            visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
        ),
        (true, old.target_sentence(), true, true, true, true),
        "failed requested refresh deleted usable metadata or dependent artifacts"
    );
}

#[test]
fn successful_meta_refresh_clears_dependents_and_attempts_but_preserves_costs() {
    let directory = TempDir::new().expect("tempdir must be created");
    let pair = LanguagePair::new("fr", "en");
    let term = "canard";
    let understanding = "a duck";
    let old = labeled_meta(
        SentenceLevel::A2,
        SentenceKind::Statement,
        AxisSet::default(),
    );
    CardMetaCache::new(directory.path())
        .store(term, understanding, &pair, &old)
        .expect("old meta must be seeded");
    let cell = CardCell::new(directory.path(), &pair, term, understanding).cache();
    let visual = cell
        .visual(visual_revision())
        .expect("visual revision must resolve");
    seed_refresh_files(&cell, &visual);
    let production = MetadataProduction::new(
        directory.path().to_path_buf(),
        GeminiAccess::unavailable(),
        CostAccounting::new(None),
    );
    let replacement = labeled_meta(
        SentenceLevel::B1,
        SentenceKind::Question,
        AxisSet::from_axes([SentenceAxis::Level, SentenceAxis::Type]),
    );
    let _guards = ArtifactGuards::hold(&cell, &visual).expect("refresh locks must be acquired");
    production
        .replace_generated(&cell, &visual, term, understanding, &pair, &replacement)
        .expect("replacement transaction must commit");
    let stored = CardMetaCache::new(directory.path())
        .load(term, understanding, &pair)
        .expect("replacement meta must decode")
        .expect("replacement meta must exist");
    assert_eq!(
        (
            stored.target_sentence(),
            cell.exists(VOICE_FILE),
            visual.exists(SCENE_FILE),
            visual.exists(ILLUSTRATION_FILE),
            visual.exists(SCENE_ATTEMPT_FILE),
            visual.exists(PICTURE_REQUESTS_FILE),
            visual.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
            cell.exists(META_COST_FILE),
            cell.exists(VOICE_COST_FILE),
            visual.exists(SCENE_COST_FILE),
            visual.exists(ILLUSTRATION_COST_FILE),
        ),
        (
            replacement.target_sentence(),
            false,
            false,
            false,
            false,
            false,
            false,
            true,
            true,
            true,
            true,
        ),
        "metadata refresh retained stale dependents, erased costs, or missed the replacement"
    );
}

fn card_meta(sentence: &str) -> CardMeta {
    CardMeta::new(
        "ka.naʁ",
        "sample",
        "hoax",
        5,
        "A source sentence",
        "source",
        "A concise hint",
        "A concise context",
        sentence,
    )
}

fn labeled_meta(level: SentenceLevel, kind: SentenceKind, pinned: AxisSet) -> CardMeta {
    card_meta(format!("A {level:?} {kind:?} sentence").as_str()).with_sentence_labels(
        SentenceLabels::new(Register::Neutral, level, kind, pinned, AxisSet::default()),
    )
}

fn seed_refresh_files(cell: &Cache, visual: &Cache) {
    for (cache, filename) in [
        (cell, META_COST_FILE),
        (cell, VOICE_FILE),
        (cell, VOICE_COST_FILE),
        (visual, SCENE_FILE),
        (visual, SCENE_ATTEMPT_FILE),
        (visual, SCENE_COST_FILE),
        (visual, ILLUSTRATION_FILE),
        (visual, ILLUSTRATION_COST_FILE),
        (visual, PICTURE_REQUESTS_FILE),
    ] {
        fs::write(
            cache.filepath(filename).expect("fixture path must resolve"),
            b"fixture",
        )
        .expect("refresh fixture must be written");
    }
    let attempts = visual.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(&attempts).expect("attempt directory must be created");
    fs::write(attempts.join("attempt-0001.json"), b"{}").expect("attempt must be written");
}

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
