use super::*;

#[test]
fn two_recall_text_rejections_enable_third_attempt_recomposition() {
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
        (false, false, true),
        "repeated text or fidelity rejections kept re-rolling the same doomed scene"
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
