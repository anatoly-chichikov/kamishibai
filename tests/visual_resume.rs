//! Exercise explicit retry and pre-send pause through the public production boundary.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use kamishibai::application::{CardProduction, GenerationControl, GenerationRun};
use kamishibai::generation::artifact_cache::{
    Cache, PICTURE_REQUESTS_FILE, SCENE_ATTEMPT_FILE, SCENE_COST_FILE, SCENE_FILE,
};
use kamishibai::generation::{VisualFailure, VisualResult, restart_picture_request_series};
use kamishibai::models::{
    GenerationStage, Look, ModelOutput, ModelProvider, ModelReply, ModelRequest, Parts,
    PictureStyle, Resend,
};
use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardCell, CardDraft, CardMeta, CostRecord,
    GenerationCost, LanguagePair,
};
use serde_json::{Value, json};
use tempfile::tempdir;

struct Studio {
    paused: AtomicBool,
    stage: GenerationStage,
    sent: Mutex<Vec<(GenerationStage, String)>>,
}

impl Studio {
    fn new(paused: bool, stage: GenerationStage) -> Self {
        Self {
            paused: AtomicBool::new(paused),
            stage,
            sent: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<(GenerationStage, String)> {
        self.sent
            .lock()
            .expect("request log must remain readable")
            .clone()
    }
}

impl ModelProvider for Studio {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        let (stage, prompt) = match request {
            ModelRequest::Text { stage, prompt, .. } => (stage, prompt),
            ModelRequest::Image { prompt, .. } => (GenerationStage::Picture, prompt),
            ModelRequest::Speech { .. } => anyhow::bail!("unexpected speech request"),
        };
        self.sent
            .lock()
            .expect("request log must remain writable")
            .push((stage, prompt));
        let text = match stage {
            GenerationStage::Features => super::gemini::scene_features(),
            GenerationStage::Scene => super::gemini::dynamic_scene(),
            GenerationStage::Picture => anyhow::bail!("drawing request reached the model"),
            other => anyhow::bail!("unexpected stage {other:?}"),
        };
        Ok(ModelReply::new(
            ModelOutput::Text {
                text: text.to_string(),
                truncated: false,
            },
            CostRecord::new(
                "local-studio",
                1,
                73,
                19,
                92,
                GenerationCost::from_nanos(391),
            ),
        ))
    }

    fn generate_visual(&self, request: ModelRequest) -> VisualResult<ModelReply> {
        let stage = match &request {
            ModelRequest::Text { stage, .. } => *stage,
            ModelRequest::Image { .. } => GenerationStage::Picture,
            ModelRequest::Speech { .. } => GenerationStage::Speech,
        };
        if self.paused.load(Ordering::SeqCst) && stage == self.stage {
            return Err(VisualFailure::NotSent);
        }
        self.generate(request).map_err(Into::into)
    }
}

impl GenerationControl for Studio {
    fn allows(&self, _card: usize) -> bool {
        true
    }

    fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

struct Pausing<P> {
    model: Arc<P>,
    paused: AtomicBool,
}

impl<P: ModelProvider> ModelProvider for Pausing<P> {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        self.model.generate(request)
    }

    fn generate_visual(&self, request: ModelRequest) -> VisualResult<ModelReply> {
        let result = self.model.generate_visual(request);
        if matches!(result, Err(VisualFailure::NotSent)) {
            self.paused.store(true, Ordering::SeqCst);
        }
        result
    }
}

impl<P> GenerationControl for Pausing<P> {
    fn allows(&self, _card: usize) -> bool {
        true
    }

    fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

fn pausing<P>(model: Arc<P>) -> Arc<Pausing<P>> {
    Arc::new(Pausing {
        model,
        paused: AtomicBool::new(false),
    })
}

struct Ink;

impl PictureStyle for Ink {
    fn name(&self) -> &str {
        "plain-ink"
    }

    fn prompt(&self, _scene: &Value) -> Result<String> {
        Ok(String::from("one framed monochrome scene"))
    }
}

fn look(resend: Resend) -> Look {
    Look::new(Arc::new(Ink), None, resend)
}

fn workflow(
    root: &Path,
    studio: Arc<impl ModelProvider + 'static>,
    look: Look,
) -> impl CardProduction {
    kamishibai::models::workflow(
        studio,
        Parts::new(look, Arc::new(kamishibai::gemini::EmbeddedPrompts)),
        "resume-v1",
        root.join("cache"),
        root.join("output"),
        None,
    )
    .expect("local workflow must compose")
}

fn draft(root: &Path) -> CardDraft {
    let audio = root.join("audio.wav");
    std::fs::write(&audio, b"local audio").expect("cached audio must exist");
    let draft = CardDraft::new("mèo", "a cat", LanguagePair::new("VI", "EN")).with_meta(
        CardMeta::new(
            "mɛw",
            "kɔn mɛw ŋu",
            "a cat",
            5,
            "The cat sleeps on the windowsill",
            "cat",
            "A furry household animal",
            "A concrete animal noun",
            "Con mèo ngủ trên bậu cửa sổ",
        ),
        None,
    );
    let artifacts = CardArtifacts::from_parts(
        draft.artifacts().meta().clone(),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(ArtifactFile::new(
            "audio.wav",
            audio,
            "local",
            false,
        )),
    );
    draft.with_artifacts(artifacts)
}

fn cache(root: &Path, draft: &CardDraft) -> Cache {
    CardCell::for_draft(
        kamishibai::models::cache_root(&root.join("cache"), "resume-v1").unwrap(),
        draft,
    )
    .cache()
    .visual(kamishibai::generation::visual_revision())
    .unwrap()
}

fn write(cache: &Cache, name: &str, value: Value) {
    std::fs::write(
        cache.filepath(name).unwrap(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

fn read(cache: &Cache, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(cache.filepath(name).unwrap()).unwrap()).unwrap()
}

fn exhausted(cache: &Cache) {
    write(
        cache,
        PICTURE_REQUESTS_FILE,
        json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":11,"series_requests":4}),
    );
    write(
        cache,
        SCENE_FILE,
        json!({"manga_panel":{"meta":{"layout_selection":{"scene_attempt_index":2}}}}),
    );
}

#[test]
fn explicit_retry_cannot_leave_an_exhausted_picture_series_closed() {
    let root = tempdir().unwrap();
    let cache = Cache::new("manual", root.path());
    exhausted(&cache);
    look(Resend::Manual).retry(&cache).unwrap();
    assert_eq!(
        read(&cache, PICTURE_REQUESTS_FILE),
        json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":11,"series_requests":0}),
        "explicit retry left the durable series exhausted or erased lifetime usage"
    );
}

fn retried(restarts: u8, resend: Resend) -> (usize, Value) {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(false, GenerationStage::Picture));
    let look = look(resend);
    let workflow = workflow(root.path(), studio.clone(), look.clone());
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    exhausted(&cache);
    if restarts == 1 {
        restart_picture_request_series(&cache).unwrap();
    }
    look.retry(&cache).unwrap();
    if restarts == 2 {
        restart_picture_request_series(&cache).unwrap();
    }
    let _ = workflow.generate_picture_in(0, &draft);
    (studio.requests().len(), read(&cache, PICTURE_REQUESTS_FILE))
}

#[test]
fn explicit_retry_cannot_block_the_next_picture_request() {
    assert_eq!(
        retried(0, Resend::Manual),
        (
            1,
            json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":12,"series_requests":1})
        ),
        "explicit retry failed to send exactly one newly authorized picture request"
    );
}

#[test]
fn explicit_retry_cannot_depend_on_the_resend_policy() {
    assert_eq!(
        retried(0, Resend::Automatic),
        (
            1,
            json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":12,"series_requests":1})
        ),
        "automatic resend prevented an explicit new request series"
    );
}

#[test]
fn explicit_retry_and_series_restart_cannot_interfere_in_either_order() {
    assert_eq!(
        [retried(1, Resend::Manual), retried(2, Resend::Manual)],
        [
            (
                1,
                json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":12,"series_requests":1})
            ),
            (
                1,
                json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":12,"series_requests":1})
            )
        ],
        "calling both retry operations changed the next authorized request"
    );
}

fn resumed(
    pauses: usize,
    stage: GenerationStage,
) -> (u8, Value, Vec<(GenerationStage, String)>, Value) {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(true, stage));
    let control = pausing(studio.clone());
    let workflow = workflow(root.path(), control.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    let mut run = GenerationRun::new(vec![draft]).unwrap();
    for _ in 0..pauses {
        control.paused.store(false, Ordering::SeqCst);
        run.advance_in(&workflow, control.as_ref()).unwrap();
    }
    let spent = run.drafts()[0].artifacts().scene().tally().done();
    studio.paused.store(false, Ordering::SeqCst);
    control.paused.store(false, Ordering::SeqCst);
    let result = run
        .advance_in(&workflow, control.as_ref())
        .unwrap()
        .unwrap()
        .into_parts();
    if let Some(error) = result.3 {
        panic!("resumed scene failed: {error}");
    }
    (
        spent,
        read(&cache, SCENE_FILE),
        studio.requests(),
        read(&cache, SCENE_COST_FILE),
    )
}

#[test]
fn hundreds_of_unsent_pauses_cannot_exhaust_the_scene_index() {
    assert_eq!(
        resumed(273, GenerationStage::Features),
        resumed(0, GenerationStage::Features),
        "unsent pauses changed the attempt count, scene selection, requests, or spend"
    );
}

#[test]
fn unsent_pauses_cannot_shift_scene_selection_or_parser_strictness() {
    assert_eq!(
        resumed(3, GenerationStage::Features),
        resumed(0, GenerationStage::Features),
        "three unsent pauses moved the scene to a different layout or lenient attempt"
    );
}

#[test]
fn pausing_after_features_cannot_skip_repeating_the_unfinished_scene() {
    let (spent, scene, requests, cost) = resumed(273, GenerationStage::Scene);
    let (_, expected, uninterrupted, _) = resumed(0, GenerationStage::Scene);
    let cost: CostRecord = serde_json::from_value(cost).unwrap();
    assert_eq!(
        (spent, scene, requests, cost.cost().nanos()),
        (
            0,
            expected,
            std::iter::repeat_n(uninterrupted[0].clone(), 273)
                .chain(uninterrupted)
                .collect::<Vec<_>>(),
            275 * 391
        ),
        "pause reused unfinished features, changed scene selection, or lost paid requests"
    );
}

#[test]
fn changed_metadata_cannot_inherit_features_from_a_paused_scene() {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(true, GenerationStage::Scene));
    let workflow = workflow(root.path(), studio.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let paused = workflow.generate_scene_in(0, &draft);
    let changed = CardMeta::new(
        "mɛw",
        "kɔn mɛw",
        "a cat",
        5,
        "The cat wakes up under the chair",
        "cat",
        "A furry household animal",
        "A concrete animal noun",
        "Con mèo thức dậy dưới ghế",
    );
    workflow
        .store_card_meta(draft.term(), draft.understanding(), draft.pair(), &changed)
        .unwrap();
    studio.paused.store(false, Ordering::SeqCst);
    let resumed = workflow.generate_scene_in(0, &draft.with_meta(changed, None));
    let features = studio
        .requests()
        .into_iter()
        .filter(|(stage, _)| *stage == GenerationStage::Features)
        .map(|(_, prompt)| prompt)
        .collect::<Vec<_>>();
    assert_eq!(
        (
            paused.error().is_some(),
            resumed.error().is_none(),
            features.len(),
            features
                .last()
                .is_some_and(|prompt| prompt.contains("Con mèo thức dậy dưới ghế")
                    && !prompt.contains("Con mèo ngủ trên bậu cửa sổ"))
        ),
        (true, true, 2, true),
        "metadata replacement reused features for the previous sentence"
    );
}

struct FailingStudio {
    inner: Studio,
    crash: Option<std::path::PathBuf>,
}

impl ModelProvider for FailingStudio {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        let stage = match &request {
            ModelRequest::Text { stage, .. } => *stage,
            _ => anyhow::bail!("unexpected non-text request"),
        };
        let reply = self.inner.generate(request)?;
        if stage == self.inner.stage
            && let Some(root) = &self.crash
        {
            std::fs::write(
                root.join("sent-request.json"),
                serde_json::to_vec(&(
                    stage == GenerationStage::Features,
                    self.inner.requests().last().unwrap().1.clone(),
                ))
                .unwrap(),
            )
            .unwrap();
            std::process::exit(73);
        }
        if stage == GenerationStage::Scene {
            anyhow::bail!("local scene request failed after send");
        }
        Ok(reply)
    }

    fn generate_visual(&self, request: ModelRequest) -> VisualResult<ModelReply> {
        if self.inner.paused.load(Ordering::SeqCst)
            && matches!(
                &request,
                ModelRequest::Text {
                    stage: GenerationStage::Scene,
                    ..
                }
            )
        {
            return Err(VisualFailure::NotSent);
        }
        self.generate(request).map_err(Into::into)
    }
}

impl GenerationControl for FailingStudio {
    fn allows(&self, _card: usize) -> bool {
        true
    }

    fn paused(&self) -> bool {
        self.inner.paused.load(Ordering::SeqCst)
    }
}

#[test]
fn alternating_pauses_and_sent_failures_cannot_bypass_the_scene_ceiling() {
    let root = tempdir().unwrap();
    let studio = Arc::new(FailingStudio {
        inner: Studio::new(true, GenerationStage::Scene),
        crash: None,
    });
    let control = pausing(studio.clone());
    let workflow = workflow(root.path(), control.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    let mut run = GenerationRun::new(vec![draft]).unwrap();
    let mut observed = Vec::new();
    for _ in 0..4 {
        studio.inner.paused.store(true, Ordering::SeqCst);
        for _ in 0..273 {
            control.paused.store(false, Ordering::SeqCst);
            run.advance_in(&workflow, control.as_ref()).unwrap();
        }
        run = GenerationRun::new(run.into_drafts()).unwrap();
        studio.inner.paused.store(false, Ordering::SeqCst);
        control.paused.store(false, Ordering::SeqCst);
        run.advance_in(&workflow, control.as_ref()).unwrap();
        observed.push((
            run.drafts()[0].artifacts().scene().tally().done(),
            read(&cache, SCENE_ATTEMPT_FILE)["scene_attempt_index"].clone(),
        ));
    }
    let sent = studio
        .inner
        .requests()
        .into_iter()
        .filter(|(stage, _)| *stage == GenerationStage::Scene)
        .count();
    assert_eq!(
        (observed, sent, run.next().is_none()),
        (
            vec![(1, json!(0)), (2, json!(1)), (3, json!(2)), (4, json!(3))],
            4,
            true
        ),
        "pauses changed sent failure indices or bypassed the scene attempt ceiling"
    );
}

fn crashed(stage: GenerationStage) -> (bool, Value, Value) {
    let root = tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "visual_resume::process_death_during_a_request_cannot_reuse_its_scene_index",
            "--nocapture",
        ])
        .env("SCENE_CRASH_ROOT", root.path())
        .env(
            "SCENE_CRASH_FEATURES",
            (stage == GenerationStage::Features).to_string(),
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if status.code() != Some(73) {
                panic!("request process exited unexpectedly: {status}");
            }
            break;
        }
        if started.elapsed() > Duration::from_secs(30) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("request process exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    let sent: (bool, String) =
        serde_json::from_slice(&std::fs::read(root.path().join("sent-request.json")).unwrap())
            .unwrap();
    let before = read(&cache, SCENE_ATTEMPT_FILE)["scene_attempt_index"].clone();
    let studio = Arc::new(Studio::new(false, GenerationStage::Scene));
    let workflow = workflow(root.path(), studio, look(Resend::Automatic));
    let result = workflow.generate_scene_in(0, &draft);
    if let Some(error) = result.error() {
        panic!("restarted scene failed: {error}");
    }
    (
        sent.0 == (stage == GenerationStage::Features) && !sent.1.is_empty(),
        before,
        read(&cache, SCENE_FILE)["manga_panel"]["meta"]["layout_selection"]["scene_attempt_index"]
            .clone(),
    )
}

#[test]
fn process_death_during_a_request_cannot_reuse_its_scene_index() {
    if let Ok(root) = std::env::var("SCENE_CRASH_ROOT") {
        let root = Path::new(&root);
        let stage = if std::env::var("SCENE_CRASH_FEATURES").unwrap() == "true" {
            GenerationStage::Features
        } else {
            GenerationStage::Scene
        };
        let studio = Arc::new(FailingStudio {
            inner: Studio::new(false, stage),
            crash: Some(root.to_path_buf()),
        });
        let workflow = workflow(root, studio, look(Resend::Automatic));
        let _ = workflow.generate_scene_in(0, &draft(root));
        panic!("child never reached the request");
    }
    assert_eq!(
        [
            crashed(GenerationStage::Features),
            crashed(GenerationStage::Scene)
        ],
        [(true, json!(0), json!(1)), (true, json!(0), json!(1))],
        "restart reused a possibly sent request index or skipped more than one index"
    );
}

fn paused_checkpoint(root: &Path, stage: GenerationStage) {
    let studio = Arc::new(Studio::new(true, stage));
    let control = pausing(studio);
    let workflow = workflow(root, control.clone(), look(Resend::Automatic));
    let mut run = GenerationRun::new(vec![draft(root)]).unwrap();
    run.advance_in(&workflow, control.as_ref()).unwrap();
    std::fs::write(
        root.join("drafts.json"),
        serde_json::to_vec(run.drafts()).unwrap(),
    )
    .unwrap();
}

fn restarted(name: &str, stage: GenerationStage) -> (u8, Value, Vec<GenerationStage>, u64) {
    let root = tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("SCENE_RESTART_ROOT", root.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if !status.success() {
                panic!("checkpoint process failed: {status}");
            }
            break;
        }
        if started.elapsed() > Duration::from_secs(30) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("checkpoint process exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let drafts: Vec<CardDraft> =
        serde_json::from_slice(&std::fs::read(root.path().join("drafts.json")).unwrap()).unwrap();
    let cache = cache(root.path(), &drafts[0]);
    let mut run = GenerationRun::new(drafts).unwrap();
    let spent = run.drafts()[0].artifacts().scene().tally().done();
    let studio = Arc::new(Studio::new(false, stage));
    let workflow = workflow(root.path(), studio.clone(), look(Resend::Automatic));
    run.advance(&workflow).unwrap();
    let cost: CostRecord = serde_json::from_value(read(&cache, SCENE_COST_FILE)).unwrap();
    (
        spent,
        read(&cache, SCENE_FILE)["manga_panel"]["meta"]["layout_selection"]["scene_attempt_index"]
            .clone(),
        studio
            .requests()
            .into_iter()
            .map(|(stage, _)| stage)
            .collect(),
        cost.cost().nanos(),
    )
}

#[test]
fn process_restart_after_an_unsent_pause_cannot_skip_a_scene_index() {
    if let Ok(root) = std::env::var("SCENE_RESTART_ROOT") {
        paused_checkpoint(Path::new(&root), GenerationStage::Features);
        return;
    }
    assert_eq!(
        restarted(
            "visual_resume::process_restart_after_an_unsent_pause_cannot_skip_a_scene_index",
            GenerationStage::Features
        ),
        (
            0,
            json!(0),
            vec![GenerationStage::Features, GenerationStage::Scene],
            782
        ),
        "restart lost an unsent scene attempt"
    );
}

#[test]
fn process_restart_after_scene_pause_cannot_skip_features_or_change_the_index() {
    if let Ok(root) = std::env::var("SCENE_RESTART_ROOT") {
        paused_checkpoint(Path::new(&root), GenerationStage::Scene);
        return;
    }
    assert_eq!(
        restarted(
            "visual_resume::process_restart_after_scene_pause_cannot_skip_features_or_change_the_index",
            GenerationStage::Scene
        ),
        (
            0,
            json!(0),
            vec![GenerationStage::Features, GenerationStage::Scene],
            1173
        ),
        "restart reused incomplete features, lost their cost, or changed the scene index"
    );
}

#[test]
fn legacy_durable_cursor_and_request_series_cannot_break_generation() {
    let root = tempdir().unwrap();
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    std::fs::write(
        cache.filepath(SCENE_ATTEMPT_FILE).unwrap(),
        include_bytes!("fixtures/attempt-state/scene-attempt.json"),
    )
    .unwrap();
    std::fs::write(
        cache.filepath(PICTURE_REQUESTS_FILE).unwrap(),
        include_bytes!("fixtures/attempt-state/picture.requests.json"),
    )
    .unwrap();
    let studio = Arc::new(Studio::new(false, GenerationStage::Scene));
    let workflow = workflow(root.path(), studio.clone(), look(Resend::Automatic));
    let mut run = GenerationRun::new(vec![draft]).unwrap();
    run.advance(&workflow).unwrap();
    assert_eq!((read(&cache, SCENE_FILE)["manga_panel"]["meta"]["layout_selection"]["scene_attempt_index"].clone(), read(&cache, PICTURE_REQUESTS_FILE), studio.requests().len()), (json!(3), json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":7,"series_requests":3}), 2), "legacy cursor or bounded request series was not preserved");
}

#[test]
fn retry_cannot_keep_manual_resend_blocked_or_preserve_a_gentle_scene() {
    let root = tempdir().unwrap();
    let cache = Cache::new("manual", root.path());
    exhausted(&cache);
    write(
        &cache,
        "soft.json",
        json!({"stage":"picture","attempt":2,"terminal":false,"budget":{"remaining":3,"pictures":11,"failures":0}}),
    );
    write(
        &cache,
        SCENE_FILE,
        json!({"manga_panel":{"meta":{"layout_selection":{"scene_attempt_index":2,"scene_mode":"soft"}}}}),
    );
    std::fs::create_dir_all(cache.filepath("attempts").unwrap()).unwrap();
    write(
        &cache,
        "attempts/attempt-0001.json",
        json!({"status":"pending","category":"moderation","image":null}),
    );
    look(Resend::Manual).retry(&cache).unwrap();
    assert_eq!(
        (
            cache.exists(SCENE_FILE),
            cache.exists("soft.json"),
            read(&cache, "attempts/attempt-0001.json")["retry_requested"].clone(),
            read(&cache, PICTURE_REQUESTS_FILE)["series_requests"].clone()
        ),
        (false, false, json!(true), json!(0)),
        "explicit retry left the gentle scene or resend guard active"
    );
}

#[test]
fn retry_cannot_reject_legacy_counters_without_a_series_field() {
    let root = tempdir().unwrap();
    let cache = Cache::new("legacy", root.path());
    write(
        &cache,
        PICTURE_REQUESTS_FILE,
        json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":13}),
    );
    look(Resend::Manual).retry(&cache).unwrap();
    assert_eq!(
        read(&cache, PICTURE_REQUESTS_FILE),
        json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":13,"series_requests":0}),
        "retry rejected the old counter or discarded lifetime requests"
    );
}

#[test]
fn pausing_a_recomposed_picture_cannot_reuse_a_sent_scene_index() {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(true, GenerationStage::Picture));
    let control = pausing(studio.clone());
    let workflow = workflow(root.path(), control.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    write(
        &cache,
        SCENE_FILE,
        json!({"manga_panel":{"meta":{"layout_selection":{"scene_attempt_index":0}}}}),
    );
    write(&cache, SCENE_ATTEMPT_FILE, json!({"scene_attempt_index":1}));
    let artifacts = CardArtifacts::from_parts(
        draft.artifacts().meta().clone(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(ArtifactFile::new(
            SCENE_FILE,
            cache.filepath(SCENE_FILE).unwrap(),
            "local",
            false,
        )),
        ArtifactSlot::fresh(Artifact::Picture),
        draft.artifacts().sound().clone(),
    );
    let mut run = GenerationRun::new(vec![draft.with_artifacts(artifacts)]).unwrap();
    for _ in 0..7 {
        control.paused.store(false, Ordering::SeqCst);
        run.advance_in(&workflow, control.as_ref()).unwrap();
    }
    let spent = run.drafts()[0].artifacts().picture().tally().done();
    let requests = read(&cache, PICTURE_REQUESTS_FILE);
    studio.paused.store(false, Ordering::SeqCst);
    run.advance(&workflow).unwrap();
    assert_eq!(
        (
            spent,
            requests["requests"].clone(),
            read(&cache, SCENE_ATTEMPT_FILE)["scene_attempt_index"].clone(),
            studio
                .requests()
                .into_iter()
                .map(|(stage, _)| stage)
                .collect::<Vec<_>>()
        ),
        (
            0,
            json!(0),
            json!(9),
            std::iter::repeat_n(vec![GenerationStage::Features, GenerationStage::Scene,], 8)
                .flatten()
                .chain([GenerationStage::Picture])
                .collect::<Vec<_>>()
        ),
        "picture pause reused a sent scene index or spent an unsent picture attempt"
    );
}

#[test]
fn standing_picture_pause_cannot_buy_work_or_change_budgets_before_resume() {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(true, GenerationStage::Picture));
    let workflow = workflow(root.path(), studio.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    write(
        &cache,
        SCENE_FILE,
        json!({"manga_panel":{"meta":{"layout_selection":{"scene_attempt_index":0}}}}),
    );
    write(&cache, SCENE_ATTEMPT_FILE, json!({"scene_attempt_index":1}));
    let counter = json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":11,"series_requests":0});
    write(&cache, PICTURE_REQUESTS_FILE, counter.clone());
    let artifacts = CardArtifacts::from_parts(
        draft.artifacts().meta().clone(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(ArtifactFile::new(
            SCENE_FILE,
            cache.filepath(SCENE_FILE).unwrap(),
            "local",
            false,
        )),
        ArtifactSlot::fresh(Artifact::Picture),
        draft.artifacts().sound().clone(),
    );
    let mut run = GenerationRun::new(vec![draft.with_artifacts(artifacts)]).unwrap();
    let before = serde_json::to_value(run.drafts()).unwrap();
    let mut settled = 0;
    for _ in 0..273 {
        settled += usize::from(
            run.advance_in(&workflow, studio.as_ref())
                .unwrap()
                .is_some(),
        );
    }
    let paused = (
        settled,
        studio.requests().len(),
        serde_json::to_value(run.drafts()).unwrap() == before,
        read(&cache, SCENE_ATTEMPT_FILE),
        read(&cache, PICTURE_REQUESTS_FILE),
    );
    studio.paused.store(false, Ordering::SeqCst);
    let resumed = run.advance_in(&workflow, studio.as_ref()).unwrap();
    assert_eq!(
        (
            paused,
            resumed.map(|step| step.into_parts().1),
            studio
                .requests()
                .into_iter()
                .map(|(stage, _)| stage)
                .collect::<Vec<_>>(),
            read(&cache, SCENE_ATTEMPT_FILE),
            read(&cache, PICTURE_REQUESTS_FILE),
        ),
        (
            (0, 0, true, json!({"scene_attempt_index":1}), counter),
            Some(Artifact::Picture),
            vec![
                GenerationStage::Features,
                GenerationStage::Scene,
                GenerationStage::Picture
            ],
            json!({"scene_attempt_index":2}),
            json!({"schema":"kamishibai.picture-request-counter","version":1,"requests":12,"series_requests":1}),
        ),
        "standing pause bought work, changed a budget, or prevented the resumed picture request"
    );
}

#[test]
fn unsent_scene_without_a_pause_cannot_bypass_the_attempt_ceiling() {
    let root = tempdir().unwrap();
    let studio = Arc::new(Studio::new(true, GenerationStage::Scene));
    let workflow = workflow(root.path(), studio.clone(), look(Resend::Automatic));
    let draft = draft(root.path());
    let cache = cache(root.path(), &draft);
    let mut run = GenerationRun::new(vec![draft]).unwrap();
    let mut failures = 0;
    for _ in 0..19 {
        if let Some(step) = run.advance(&workflow).unwrap() {
            failures += usize::from(step.into_parts().3.is_some());
        }
    }
    assert_eq!(
        (
            failures,
            run.drafts()[0].artifacts().scene().tally().done(),
            run.next(),
            studio
                .requests()
                .into_iter()
                .map(|(stage, _)| stage)
                .collect::<Vec<_>>(),
            cache.exists(SCENE_ATTEMPT_FILE),
        ),
        (4, 4, None, vec![GenerationStage::Features; 4], false),
        "unpaused refusal avoided a failed attempt, exceeded the ceiling, or kept an unsent scene index"
    );
}
