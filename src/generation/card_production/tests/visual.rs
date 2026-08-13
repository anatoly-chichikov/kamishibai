use super::*;

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
fn a_rejected_picture_attempt_carries_the_renderer_verdict_and_its_frame() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("card", home.path());
    let renderer = production_renderer(
        CountingImageSource::new(image_bytes(GrayImage::from_pixel(16, 16, Luma([0])))),
        AcceptingRecall,
        BorderDetector::new(2, 6, 240, 2),
    )
    .with_attempt_archive(
        cache
            .filepath(IMAGE_ATTEMPTS_DIRECTORY)
            .expect("attempt archive path must resolve"),
    );
    let archived = archived_sequence(&cache);
    let rejected = renderer.render(&renderable_scene(), &mut NoopProgress);
    let attempt = judged(
        ArtifactAttempt::unmetered(rejected.map(|_| unreachable!("the border gate must reject"))),
        &cache,
        archived,
    );
    let fault = attempt
        .fault()
        .expect("a rejected picture must name the gate that rejected it");
    assert_eq!(
        (
            fault.category(),
            fault
                .artifact()
                .and_then(|path| path.file_name().and_then(|name| name.to_str())),
        ),
        ("border", Some("attempt-0001.png")),
        "a rejected picture attempt lost the renderer verdict or the frame it judged"
    );
}

#[derive(Clone, Copy, Debug)]
struct FailingRecall;

impl RecallJudge for FailingRecall {
    fn review(&self, _scene: &Value, _image: &[u8]) -> Result<RecallReview> {
        bail!("recall judge unavailable")
    }
}

#[test]
fn a_resumed_picture_rejection_carries_its_same_sequence_verdict_and_frame() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("card", home.path());
    let attempts = cache
        .filepath(IMAGE_ATTEMPTS_DIRECTORY)
        .expect("attempt archive path must resolve");
    let source = CountingImageSource::new(valid_image());
    let first = production_renderer(
        source.clone(),
        FailingRecall,
        BorderDetector::new(2, 6, 240, 2),
    )
    .with_attempt_archive(attempts.clone())
    .render(&renderable_scene(), &mut NoopProgress);
    let archived = archived_sequence(&cache);
    let rejected = production_renderer(
        source.clone(),
        AcceptingRecall,
        BorderDetector::new(2, 6, 240, 4),
    )
    .with_attempt_archive(attempts)
    .render(&renderable_scene(), &mut NoopProgress);
    let attempt = judged(
        ArtifactAttempt::unmetered(
            rejected.map(|_| unreachable!("the resumed image must fail the stricter border gate")),
        ),
        &cache,
        archived,
    );
    assert_eq!(
        (
            first.is_err(),
            attempt.fault().map(|fault| fault.category()),
            attempt.fault().and_then(|fault| {
                fault
                    .artifact()
                    .and_then(|path| path.file_name().and_then(|name| name.to_str()))
            }),
            source.calls(),
            archived_sequence(&cache),
        ),
        (true, Some("border"), Some("attempt-0001.png"), 1, 1),
        "a resumed picture rejection lost its rewritten verdict or immutable frame"
    );
}

#[test]
fn a_failure_before_the_provider_does_not_borrow_an_older_rejected_frame() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("card", home.path());
    let renderer = production_renderer(
        CountingImageSource::new(image_bytes(GrayImage::from_pixel(16, 16, Luma([0])))),
        AcceptingRecall,
        BorderDetector::new(2, 6, 240, 2),
    )
    .with_attempt_archive(
        cache
            .filepath(IMAGE_ATTEMPTS_DIRECTORY)
            .expect("attempt archive path must resolve"),
    );
    let _rejected = renderer.render(&renderable_scene(), &mut NoopProgress);
    let archived = archived_sequence(&cache);
    let attempt = judged(
        ArtifactAttempt::<ArtifactFile>::unmetered(Err(anyhow!("visual cache remained locked"))),
        &cache,
        archived,
    );
    assert!(
        attempt.fault().is_none(),
        "an attempt that never reached the provider was blamed on an older rejected picture"
    );
}

fn rejected_scene_attempt(reply: &str) -> ArtifactAttempt<ArtifactFile> {
    ArtifactAttempt::unmetered(Err(anyhow!("scene did not validate")
        .context(crate::gemini::RejectedReply::new("scene composer", reply))))
}

#[test]
fn a_rejected_scene_keeps_the_model_reply_in_the_shape_it_arrived_in() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("card", home.path());
    let structured = archived_reply(rejected_scene_attempt(r#"{"panels": []}"#), &cache);
    let prose = archived_reply(rejected_scene_attempt("I cannot draw that panel."), &cache);
    let names = [structured, prose].map(|attempt| {
        attempt
            .fault()
            .and_then(|fault| fault.artifact())
            .and_then(|path| path.file_name().and_then(|name| name.to_str()))
            .map(String::from)
    });
    assert_eq!(
        names,
        [
            Some(String::from("scene-0001.json")),
            Some(String::from("scene-0002.txt")),
        ],
        "an archived scene reply lost its shape or overwrote the previous one"
    );
}

#[test]
fn a_scene_failure_without_a_reply_archives_nothing() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("card", home.path());
    let attempt = archived_reply(
        ArtifactAttempt::<ArtifactFile>::unmetered(Err(anyhow!("visual cache remained locked"))),
        &cache,
    );
    assert!(
        attempt.fault().is_none() && !cache.path().join(IMAGE_ATTEMPTS_DIRECTORY).exists(),
        "a failure that never reached the model still wrote something into the archive"
    );
}
