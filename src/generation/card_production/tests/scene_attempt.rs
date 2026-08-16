use super::*;

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
fn cached_scene_does_not_move_recomposition_cursor_backwards() {
    let temporary = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("visual", temporary.path());
    fs::write(
        cache.filepath(SCENE_FILE).expect("scene path must resolve"),
        serde_json::to_vec(&serde_json::json!({
            "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 0}}}
        }))
        .expect("scene provenance must encode"),
    )
    .expect("scene provenance must be written");
    fs::write(
        cache
            .filepath(SCENE_ATTEMPT_FILE)
            .expect("cursor path must resolve"),
        serde_json::to_vec(&serde_json::json!({"scene_attempt_index": 4}))
            .expect("cursor must encode"),
    )
    .expect("recomposition cursor must be seeded");
    let cached = reserve_scene_attempt(&cache, Artifact::Scene, 0, false)
        .expect("cached scene must not rewind the durable cursor");
    let next = reserve_scene_attempt(&cache, Artifact::Picture, 0, true)
        .expect("next recomposition must advance the durable cursor");
    assert_eq!(
        (
            cached,
            next,
            load_scene_attempt(&cache).expect("cursor must decode")
        ),
        (0, 5, Some(5)),
        "cached scene rewound or skipped the durable recomposition cursor"
    );
}
