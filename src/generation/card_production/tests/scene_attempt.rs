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

#[test]
fn failed_reservation_cannot_skip_an_unwritten_scene_index() {
    let temporary = TempDir::new().unwrap();
    let cache = Cache::failing("visual", temporary.path(), 0);
    fs::write(
        cache.filepath(SCENE_ATTEMPT_FILE).unwrap(),
        br#"{"scene_attempt_index":6}"#,
    )
    .unwrap();
    let failed = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).is_err();
    let restored = Cache::new("visual", temporary.path());
    let attempt = reserve_scene_attempt(&restored, Artifact::Scene, 0, false).unwrap();
    assert_eq!(
        (failed, attempt, load_scene_attempt(&restored).unwrap()),
        (true, 7, Some(7)),
        "failed cursor write skipped a scene index before any request"
    );
}

#[test]
fn abandoned_reservation_cannot_reuse_a_possibly_sent_scene_index() {
    let temporary = TempDir::new().unwrap();
    let cache = Cache::new("visual", temporary.path());
    let first = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
    let restored = Cache::new("visual", temporary.path());
    let second = reserve_scene_attempt(&restored, Artifact::Scene, 0, false).unwrap();
    assert_eq!(
        (first, second, load_scene_attempt(&restored).unwrap()),
        (0, 1, Some(1)),
        "restart reused a reserved index without confirmation that its request was unsent"
    );
}

#[test]
fn settled_failure_cannot_reuse_the_previous_scene_index() {
    let temporary = TempDir::new().unwrap();
    let cache = Cache::new("visual", temporary.path());
    let first = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
    let second = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
    assert_eq!(
        (first, second),
        (0, 1),
        "settled failure reused the previous scene selection"
    );
}

struct PausedScene;

impl crate::generation::manga::Translator for PausedScene {
    fn translate(&self, _sentence: &str, _target: &str) -> Result<serde_json::Value> {
        Err(crate::generation::VisualFailure::NotSent.into_error())
    }

    fn translate_observed(
        &self,
        _sentence: &str,
        _target: &str,
    ) -> crate::generation::VisualResult<serde_json::Value> {
        Err(crate::generation::VisualFailure::NotSent)
    }
}

#[test]
fn unsent_scene_cannot_leave_its_reservation_on_an_existing_cursor() {
    use crate::generation::manga::Translator;
    let temporary = TempDir::new().unwrap();
    let cache = Cache::new("visual", temporary.path());
    fs::write(
        cache.filepath(SCENE_ATTEMPT_FILE).unwrap(),
        br#"{"scene_attempt_index":254}"#,
    )
    .unwrap();
    let mut observations = Vec::new();
    for _ in 0..273 {
        let reserved = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
        let paused = ReservedScene::new(PausedScene, cache.clone(), Some(254))
            .translate_observed("The cat sleeps", "EN");
        observations.push((
            reserved,
            matches!(paused, Err(crate::generation::VisualFailure::NotSent)),
            load_scene_attempt(&cache).unwrap(),
        ));
    }
    assert_eq!(
        observations,
        vec![(255, true, Some(254)); 273],
        "unsent attempts exhausted or rewound the existing cursor"
    );
}

#[test]
fn rollback_failure_cannot_report_a_successful_pause() {
    use crate::generation::manga::Translator;
    let temporary = TempDir::new().unwrap();
    let cache = Cache::failing("visual", temporary.path(), 1);
    fs::write(
        cache.filepath(SCENE_ATTEMPT_FILE).unwrap(),
        br#"{"scene_attempt_index":6}"#,
    )
    .unwrap();
    reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
    let paused = ReservedScene::new(PausedScene, cache.clone(), Some(6))
        .translate_observed("The cat sleeps", "EN");
    let restarted = reserve_scene_attempt(&cache, Artifact::Scene, 0, false).unwrap();
    assert_eq!(
        (
            matches!(paused, Err(crate::generation::VisualFailure::Failed(_))),
            restarted
        ),
        (true, 8),
        "failed rollback hid the storage error or reused a reserved scene index"
    );
}
