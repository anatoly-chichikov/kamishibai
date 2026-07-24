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
