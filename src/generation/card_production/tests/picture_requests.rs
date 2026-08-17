use super::*;

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
            String::from("gemini-3.1-flash-image,gemini-3.7-flash"),
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
    for _ in 0..4 {
        let _ = first.image("compiled image prompt");
    }
    let restarted = RequestCountingImage::new(source.clone(), cache.clone());
    let beyond = restarted.image("compiled image prompt");
    assert_eq!(
        (
            beyond.is_err(),
            source.calls(),
            picture_requests(&cache),
            picture_series_requests(&cache),
        ),
        (true, 4, 4, 4),
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
        br#"{"schema":"kamishibai.picture-request-counter","version":1,"requests":4}"#,
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
        (true, 0, 4, 4),
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
