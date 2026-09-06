use super::*;

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
fn correction_observer_retains_both_author_and_phonetic_charges() {
    let home = TempDir::new().expect("tempdir must be created");
    let cache = Cache::new("cards/phonetic-cost", home.path());
    let recorder = CostRecorder::new(cache.clone(), Artifact::Meta);
    recorder
        .push_correction(CostRecord::new(
            "gemini-3.8-flash",
            1,
            100,
            50,
            150,
            GenerationCost::from_nanos(262_500),
        ))
        .expect("author cost must persist");
    recorder
        .push_correction(CostRecord::new(
            "gemini-3.8-flash",
            1,
            7,
            14,
            21,
            GenerationCost::from_nanos(57_750),
        ))
        .expect("phonetic cost must persist");
    assert_eq!(
        (
            recorder
                .current(false)
                .expect("operation cost must resolve"),
            load_cost_record(&cache, Artifact::Meta).expect("stored cost must decode"),
        ),
        (
            Some(GenerationCost::from_nanos(320_250)),
            Some(CostRecord::new(
                "gemini-3.8-flash",
                2,
                107,
                64,
                171,
                GenerationCost::from_nanos(320_250)
            )),
        ),
        "correction observer discarded a text pass or double-counted its operation aggregate"
    );
}

#[test]
fn provider_observer_journals_session_spend_before_lifetime_sidecar_failure() {
    let home = TempDir::new().expect("tempdir must be created");
    let ledger = RecordingLedger::default();
    let recorder = CostRecorder::attributed(
        Cache::failing("cards/test", home.path(), 0),
        Artifact::Picture,
        Some(SlotCostAttribution::new(Arc::new(ledger.clone()), 0)),
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
        (result.is_err(), ledger.cost(0, Artifact::Picture),),
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
        let test = format!(
            "{}::correction_cost_waits_for_the_stable_meta_lease",
            module_path!()
                .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
                .unwrap_or(module_path!())
        );
        let mut child = Command::new(std::env::current_exe().expect("test binary must resolve"))
            .args([test.as_str(), "--exact", "--nocapture"])
            .env("KAMISHIBAI_CORRECTION_LOCK_ROOT", home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cost writer child must spawn");
        let ready = home.path().join("correction-cost-ready");
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        let reached_lock = loop {
            if ready.exists() {
                break true;
            }
            if child
                .try_wait()
                .expect("child state must be observable")
                .is_some()
            {
                break false;
            }
            if Instant::now() >= ready_deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let waited = reached_lock
            && child
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
    let root = PathBuf::from(root);
    std::fs::write(root.join("correction-cost-ready"), b"ready")
        .expect("child readiness must persist");
    let cache = Cache::new("cards/test", root);
    let record = CostRecord::new(
        "gemini-3.6-flash",
        1,
        100,
        20,
        120,
        GenerationCost::from_nanos(300_000),
    );
    let stored = persist_correction_cost(&cache, &record);
    if stored.is_err() || !cache.exists(META_COST_FILE) {
        panic!("child failed to persist billed correction cost under the meta lease");
    }
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
