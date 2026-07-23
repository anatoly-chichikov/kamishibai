//! Tests for embedded Rust assets.

use kamishibai::generation::prompts as assets;

/// Embedded audio prompt stays aligned with the Python baseline asset.
#[test]
fn embedded_audio_prompt_matches_the_baseline_asset() {
    assert_eq!(
        assets::audio_prompt(),
        "Say in natural {language}: {text}",
        "embedded audio prompt drifted away from the baseline asset"
    );
}

/// Rendered audio prompt interpolates the target language.
#[test]
fn rendered_audio_prompt_inserts_the_target_language() {
    assert_eq!(
        assets::render_audio_prompt("Greek"),
        "Say in natural Greek: {text}",
        "rendered audio prompt did not interpolate the target language"
    );
}

/// Embedded manga template keeps the expected root object name.
#[test]
fn embedded_manga_template_keeps_the_panel_root() {
    assert!(
        assets::manga_template().contains("\"manga_panel\""),
        "embedded manga template lost the manga_panel root"
    );
}

/// Embedded manga policy is the production dynamic-layout specification.
#[test]
fn embedded_manga_template_keeps_the_dynamic_production_policy() -> anyhow::Result<()> {
    let template = serde_json::from_str::<serde_json::Value>(assets::manga_template())?;
    assert_eq!(
        (
            template["manga_panel"]["meta"]["spec_version"].as_str(),
            template["manga_panel"]["panel_layout"]["special_device_budget"].as_i64(),
            template["manga_panel"]["rendering_rules"]["finished_artwork"].as_str(),
        ),
        (
            Some("2.0.0"),
            Some(1),
            Some(
                "render only final finished manga artwork, never a blueprint, storyboard, wireframe, annotated plan, or layout diagram"
            ),
        ),
        "embedded manga template lost the production dynamic-layout policy"
    );
    Ok(())
}

/// Visual prompt policy exposes one stable SHA-256 cache revision.
#[test]
fn visual_revision_is_one_stable_sha256_digest() {
    assert_eq!(
        (assets::visual_revision(), assets::visual_revision().len()),
        (assets::visual_revision(), 64),
        "visual revision is not a stable SHA-256 digest"
    );
}

/// Production planning binds every camera change to narrative information and continuity.
#[test]
fn production_camera_plan_is_motivated_and_editable() -> anyhow::Result<()> {
    let features = include_str!("../assets/layout_features_prompt.txt");
    let composer = include_str!("../assets/layout_scene_prompt.txt");
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))?;
    assert_eq!(
        (
            features.contains("camera_arc"),
            features.contains("camera_motivation"),
            features.contains("information_gain"),
            features.contains("over_the_shoulder"),
            features.contains("match_on_action"),
            composer.contains("Never change framing merely for variety"),
            schema
                .pointer("/properties/panels/items/properties/scene/properties/camera/properties/viewpoint/enum")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| values.contains(&serde_json::json!("over_the_shoulder"))),
            schema
                .pointer("/properties/panels/items/properties/scene/properties/camera/properties/framing/enum")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| values.contains(&serde_json::json!("insert"))),
        ),
        (true, true, true, true, true, true, true, true),
        "production camera plan still permits unmotivated flat coverage"
    );
    Ok(())
}

/// Composer continuity stays within Gemini's supported structured-output depth.
#[test]
fn production_composer_flattens_wire_continuity_without_weakening_the_scene_contract()
-> anyhow::Result<()> {
    let prompt = include_str!("../assets/layout_scene_prompt.txt");
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))?;
    let continuity = schema
        .pointer("/properties/panels/items/properties/continuity/properties")
        .and_then(serde_json::Value::as_object)
        .expect("invariant: composer continuity schema must be an object");
    let required = schema
        .pointer("/properties/panels/items/properties/continuity/required")
        .and_then(serde_json::Value::as_array)
        .expect("invariant: composer continuity must require every wire field");
    let required = required
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        (
            [
                continuity.contains_key("eyeline_enabled"),
                continuity.contains_key("eyeline_looker_id"),
                continuity.contains_key("eyeline_target_anchor"),
                continuity.contains_key("eyeline_direction"),
                continuity.contains_key("match_on_action_enabled"),
                continuity.contains_key("match_on_action_subject_id"),
                continuity.contains_key("match_on_action_action"),
                !continuity.contains_key("eyeline"),
                !continuity.contains_key("match_on_action"),
                required.contains("eyeline_enabled"),
                required.contains("match_on_action_action"),
                prompt.contains("\"eyeline_enabled\": false"),
                prompt.contains("\"match_on_action_action\": \"\""),
            ],
            required.len(),
        ),
        ([true; 13], 11),
        "composer wire schema regained unsupported nested continuity objects"
    );
    Ok(())
}

/// Production registry builds use the stable picture-recovery visual revision.
#[test]
fn production_registry_uses_the_version_thirty_four_visual_revision() {
    assert_eq!(
        assets::visual_revision(),
        "78a3daa17035bd163e72ca62b15ad3bca1bb205963789d5f0918e8c92c9331db",
        "production registry visual revision drifted without a policy-version change"
    );
}

/// Production planning removes printed state cues before the image boundary.
#[test]
fn production_registry_prompts_require_unlabeled_physical_state() {
    let features = include_str!("../assets/layout_features_prompt.txt");
    let composer = include_str!("../assets/layout_scene_prompt.txt");
    let template = include_str!("../assets/manga_template.json");
    assert_eq!(
        (
            features.contains("Never put a printed state name"),
            composer.contains("Never put a printed state name"),
            template.contains("continuous content-free pure-white 16px band"),
            !template.contains("blank_hidden_or_icon_only"),
        ),
        (true, true, true, true),
        "production image boundary regained textual state cues or an ambiguous outer frame"
    );
}

/// Mechanical state changes stay physical instead of becoming arrows or control labels.
#[test]
fn production_image_boundary_forbids_control_state_glyphs() -> anyhow::Result<()> {
    let features = include_str!("../assets/layout_features_prompt.txt");
    let composer = include_str!("../assets/layout_scene_prompt.txt");
    let template =
        serde_json::from_str::<serde_json::Value>(include_str!("../assets/manga_template.json"))?;
    let controls = template
        .pointer("/manga_panel/rendering_rules/mechanical_controls")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert_eq!(
        (
            features.contains("one static unlabeled physical position per panel"),
            features.contains("Never request arrows, direction indicators"),
            composer.contains("one static unlabeled physical position per panel"),
            composer.contains("Never describe a mechanical control as mid-motion"),
            composer.contains("Never request arrows, direction indicators"),
            composer.contains("Set motion_treatment=none"),
            controls.contains("one static unlabeled physical position per panel"),
            controls.contains("Never draw arrows, direction indicators"),
            controls.contains("I/O, ON/OFF, zero/one labels"),
        ),
        (true, true, true, true, true, true, true, true, true),
        "production image boundary regained arrow-like mechanical state cues"
    );
    Ok(())
}

/// Production planning keeps sentence truth separate from cinematic coverage.
#[test]
fn production_registry_prompt_separates_semantic_beats_from_shots() {
    let prompt = include_str!("../assets/layout_features_prompt.txt");
    assert!(
        prompt.contains("semantic_beat_count")
            && prompt.contains("panel_count")
            && prompt.contains("source_support"),
        "registry prompt still forces cinematic coverage to invent semantic chronology"
    );
}

/// Production planning does not cap camera coverage at the semantic beat count.
#[test]
fn production_registry_prompt_allows_four_views_of_fewer_semantic_beats() {
    let prompt = include_str!("../assets/layout_features_prompt.txt");
    assert!(
        prompt.contains("semantic_beat_count does not cap panel_count")
            && prompt.contains("setting, operator or subject, mechanism in action"),
        "registry prompt still collapses a rich process into one shot per semantic beat"
    );
}

/// Production planning makes every candidate coverage count explicit and auditable.
#[test]
fn production_registry_prompt_requires_ordered_coverage_audit() {
    let prompt = include_str!("../assets/layout_features_prompt.txt");
    assert!(
        prompt.contains(
            "coverage_audit contains exactly four entries ordered by panel_count 1, 2, 3, 4"
        ) && prompt.contains("Every lower count is insufficient")
            && prompt.contains("Every higher count is redundant_or_unsupported")
            && prompt.contains("Evaluate the fourth candidate as seriously as the first three")
            && prompt.contains("a wide setting/operator view")
            && prompt.contains("the observable stable result"),
        "registry prompt does not force a grounded four-count coverage decision"
    );
}

/// Production planning splits compound anchors before dismissing a grounded fourth view.
#[test]
fn production_registry_prompt_audits_compound_and_non_textual_coverage() {
    let prompt = include_str!("../assets/layout_features_prompt.txt");
    assert!(
        prompt.contains("inspect every selected visible_anchor for compound coverage")
            && prompt.contains("A close evidence or mechanism detail is not redundant")
            && prompt
                .contains("an actor, a purposeful process, an obstacle or operating condition")
            && prompt.contains("Explicitly audit non-textual evidence")
            && prompt
                .contains("technicians open a jammed access plate to inspect the pipe beneath it")
            && prompt.contains("no invented pre-state or later repair")
            && prompt.contains("Do not propose text-bearing screens, gauges, calendars"),
        "registry prompt still collapses a grounded fourth view or relies on textual evidence"
    );
}

/// Production planning compares support-shot weight before dynamic geometry becomes eligible.
#[test]
fn production_registry_feature_prompt_compares_local_support_weight() {
    let prompt = include_str!("../assets/layout_features_prompt.txt");
    assert!(
        prompt.contains("compare the local narrative weight of every non-dominant shot")
            && prompt.contains("compact context or baseline")
            && prompt.contains("heavier action, mechanism, or evidence"),
        "registry feature planning never exposes the support-shot hierarchy needed by asymmetric routing"
    );
}

/// Production composition preserves unequal area through unequal cinematic staging.
#[test]
fn production_registry_scene_prompt_preserves_area_hierarchy() {
    let prompt = include_str!("../assets/layout_scene_prompt.txt");
    assert!(
        prompt.contains("Do not cancel unequal canonical panel areas")
            && prompt.contains("identical centered portraits")
            && prompt.contains("smaller support shot visually compact"),
        "registry scene composition can still flatten reviewed panel-area hierarchy"
    );
}

/// Production scene composition accepts every transition emitted by the feature planner.
#[test]
fn production_registry_composer_accepts_scene_to_scene_transitions() {
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))
    .expect("invariant: embedded layout scene schema must decode");
    let transitions = schema
        .pointer("/properties/panels/items/properties/transition_from_previous/enum")
        .and_then(serde_json::Value::as_array)
        .expect("invariant: composer transition enum must be an array");
    let prompt = include_str!("../assets/layout_scene_prompt.txt");
    assert_eq!(
        (
            transitions.contains(&serde_json::json!("scene_to_scene")),
            prompt.contains("subject_to_subject|scene_to_scene|aspect_to_aspect"),
        ),
        (true, true),
        "registry composer cannot express the planner's scene-to-scene transition"
    );
}

/// Production scene composition accepts contrast as a semantic visual relation.
#[test]
fn production_registry_composer_accepts_contrast_visual_relation() {
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))
    .expect("invariant: embedded layout scene schema must decode");
    let relations = schema
        .pointer("/properties/semantic_spine/properties/visual_relation/enum")
        .and_then(serde_json::Value::as_array)
        .expect("invariant: composer visual relation enum must be an array");
    let prompt = include_str!("../assets/layout_scene_prompt.txt");
    assert_eq!(
        (
            relations.contains(&serde_json::json!("contrast")),
            prompt.contains("opposition|contrast|burden"),
        ),
        (true, true),
        "registry composer rejects the model's supported contrast relation"
    );
}

/// Production composer schema prevents two observed stochastic wire failures.
#[test]
fn production_registry_composer_constrains_transition_and_expression() {
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))
    .expect("invariant: embedded layout scene schema must decode");
    let transitions = schema
        .pointer("/properties/panels/items/properties/transition_from_previous/enum")
        .and_then(serde_json::Value::as_array)
        .expect("invariant: composer transition enum must be an array");
    assert_eq!(
        (
            transitions.contains(&serde_json::json!("attention_shift")),
            schema
                .pointer("/properties/panels/items/properties/scene/properties/subjects/items/properties/expression/minLength")
                .and_then(serde_json::Value::as_u64),
        ),
        (false, Some(1)),
        "composer schema still permits one observed transition or expression failure"
    );
}

/// Production scene composition exposes the complete locally materialized single-card device wire.
#[test]
fn production_registry_composer_selects_operational_special_devices() {
    let schema = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/layout_scene_schema.json"
    ))
    .expect("invariant: embedded layout scene schema must decode");
    let kinds = schema
        .pointer("/properties/page_design/properties/special_device/properties/kind/enum")
        .and_then(serde_json::Value::as_array)
        .expect("invariant: composer device enum must be an array");
    let prompt = include_str!("../assets/layout_scene_prompt.txt");
    assert_eq!(
        (
            kinds,
            prompt.contains("Choose exactly one special device from device_candidates"),
            prompt.contains("locally qualified for automatic production selection"),
            prompt.contains("source_panel and target_panel use shot ids"),
            prompt.contains("The local materializer applies the selected device"),
            prompt.contains("source_panel is the parent/base shot")
                && !prompt.contains("source_panel is the larger parent shot"),
        ),
        (
            &vec![
                serde_json::json!("none"),
                serde_json::json!("crossing"),
                serde_json::json!("overlap"),
                serde_json::json!("inset"),
                serde_json::json!("open_frame"),
                serde_json::json!("master_view"),
                serde_json::json!("diagonal_release"),
            ],
            true,
            true,
            true,
            true,
            true,
        ),
        "registry composer still forces topology isolation instead of selecting operational devices"
    );
}

/// Production device selection is driven by one embedded versioned JSON catalog.
#[test]
fn production_registry_embeds_one_device_budget_and_honest_capability_statuses() {
    let registry = serde_json::from_str::<serde_json::Value>(include_str!(
        "../assets/device_registry_v3.json"
    ))
    .expect("invariant: embedded device registry must decode");
    let devices = registry["devices"]
        .as_array()
        .expect("invariant: device registry must contain devices");
    let statuses = devices
        .iter()
        .map(|value| {
            (
                value["scene_kind"]
                    .as_str()
                    .expect("invariant: scene kind must be a string"),
                (
                    value["capability_status"]
                        .as_str()
                        .expect("invariant: capability status must be a string"),
                    value["automatic_selection"]
                        .as_bool()
                        .expect("invariant: automatic selection must be a boolean"),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        (
            registry["schema"].as_str(),
            registry["version"].as_u64(),
            registry["selection_policy"]["maximum_devices"].as_u64(),
            statuses,
        ),
        (
            Some("kamishibai.dynamic-manga.operational-device-registry"),
            Some(3),
            Some(1),
            std::collections::BTreeMap::from([
                ("crossing", ("proven", true)),
                ("diagonal_release", ("qualification_required", false)),
                ("inset", ("qualification_required", false)),
                ("master_view", ("qualification_required", false)),
                ("none", ("qualified", true)),
                ("open_frame", ("qualification_required", false)),
                ("overlap", ("qualification_required", false)),
            ]),
        ),
        "device catalog hides budget or overstates an unqualified visual capability"
    );
}
