//! Production manga-scene decoding and dynamic-layout validation.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::generation::layout::{LayoutSelection, materialize};
use crate::generation::manga_template;

use super::protocol::{enforce, unfence, validate};

const DYNAMIC_SPEC: &str = "2.0.0";
const PANEL_MIN: i64 = 16;
const PANEL_MAX: i64 = 1008;

#[derive(Clone, Copy, Debug)]
struct EditContinuity<'a> {
    axis_relation: &'a str,
    screen_direction: &'a str,
    eyeline: Eyeline<'a>,
}

#[derive(Clone, Copy, Debug)]
struct Eyeline<'a> {
    enabled: bool,
    looker: &'a str,
    target: &'a str,
    direction: &'a str,
}

/// Merge one registry-composer response and replace its topology with canonical geometry.
pub(super) fn compose(
    raw: &str,
    sentence: &str,
    target: &str,
    selection: &LayoutSelection,
) -> Result<Value> {
    let decoded = decode_composer(unfence(raw.trim()))?;
    let mut fields = decoded
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("registry scene composer must return one scene object"))?;
    normalize_panel_roles(&mut fields)?;
    normalize_continuity(&mut fields)?;
    let mut scene = serde_json::from_str::<Value>(manga_template())?;
    let root = scene
        .get_mut("manga_panel")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("manga template must contain a manga_panel object"))?;
    merge_dynamic(root, &mut fields)?;
    let meta = root
        .get_mut("meta")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("manga template metadata must be an object"))?;
    meta.insert(
        String::from("title"),
        Value::String(sentence.chars().take(60).collect()),
    );
    meta.insert(
        String::from("description"),
        Value::String(String::from(sentence)),
    );
    meta.insert(
        String::from("target_lang"),
        Value::String(target.to_ascii_lowercase()),
    );
    materialize(&mut scene, selection)?;
    normalize_camera_subjects(&mut scene)?;
    normalize_eyelines(&mut scene)?;
    normalize_match_on_action(&mut scene)?;
    normalize_subject_expressions(&mut scene)?;
    validate_dynamic(&scene)?;
    specialize(&mut scene)?;
    enforce(&mut scene);
    validate(&scene)?;
    Ok(scene)
}

fn decode_composer(raw: &str) -> Result<Value> {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => Ok(value),
        Err(error) if error.is_eof() => {
            let repaired = close_json(raw).ok_or_else(|| {
                anyhow!("registry scene composer returned irreparable truncated JSON")
            })?;
            serde_json::from_str::<Value>(repaired.as_str())
                .context("registry scene composer returned invalid JSON")
        }
        Err(error) => Err(anyhow!(error).context("registry scene composer returned invalid JSON")),
    }
}

fn close_json(raw: &str) -> Option<String> {
    let mut expected = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in raw.chars() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' => expected.push('}'),
            '[' => expected.push(']'),
            '}' | ']' if expected.pop() != Some(character) => return None,
            _ => {}
        }
    }
    if quoted || expected.is_empty() {
        return None;
    }
    let mut repaired = String::from(raw);
    repaired.extend(expected.into_iter().rev());
    Some(repaired)
}

fn normalize_continuity(fields: &mut Map<String, Value>) -> Result<()> {
    let panels = fields
        .get_mut("panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registry scene composer must return a panels array"))?;
    for panel in panels {
        let continuity = panel
            .get_mut("continuity")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("registry scene composer panel must contain continuity"))?;
        nest_continuity(
            continuity,
            "eyeline",
            &[
                ("eyeline_enabled", "enabled"),
                ("eyeline_looker_id", "looker_id"),
                ("eyeline_target_anchor", "target_anchor"),
                ("eyeline_direction", "direction"),
            ],
        )?;
        nest_continuity(
            continuity,
            "match_on_action",
            &[
                ("match_on_action_enabled", "enabled"),
                ("match_on_action_subject_id", "subject_id"),
                ("match_on_action_action", "action"),
            ],
        )?;
    }
    Ok(())
}

fn normalize_panel_roles(fields: &mut Map<String, Value>) -> Result<()> {
    let panels = fields
        .get_mut("panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registry scene composer must return a panels array"))?;
    for panel in panels {
        let root = panel
            .as_object_mut()
            .ok_or_else(|| anyhow!("registry scene composer panels must be objects"))?;
        if root.get("narrative_role").and_then(Value::as_str) == Some("payoff") {
            root.insert(
                String::from("narrative_role"),
                Value::String(String::from("peak")),
            );
        }
    }
    Ok(())
}

fn normalize_camera_subjects(scene: &mut Value) -> Result<()> {
    let policy = String::from(
        scene
            .pointer("/manga_panel/page_design/camera_arc/continuity/eyeline_policy")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("registered scene must contain an eyeline policy"))?,
    );
    let panels = scene
        .pointer_mut("/manga_panel/panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registered scene must contain panels"))?;
    for panel in panels {
        let framing = panel
            .pointer("/scene/camera/framing")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain camera framing"))?;
        let viewpoint = panel
            .pointer("/scene/camera/viewpoint")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain camera viewpoint"))?;
        let anchor = panel
            .pointer("/scene/camera/viewpoint_subject_id")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain a viewpoint subject id"))?;
        let eyeline = panel
            .pointer("/continuity/eyeline/looker_id")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain an eyeline looker id"))?;
        let visible = panel
            .pointer("/shot_contract/visible_anchor")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain a visible shot anchor"))?;
        let attention = panel
            .get("attentional_frame")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("registered panel must contain an attentional frame"))?;
        let subjects = panel
            .pointer_mut("/scene/subjects")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("registered panel must contain subjects"))?;
        if subjects.is_empty() && !matches!(attention.as_str(), "macro" | "amorphic") {
            add_camera_subject(
                subjects,
                "scene_anchor",
                visible.as_str(),
                "clearly visible as the declared focal subject",
            );
        }
        if viewpoint == "over_the_shoulder" && !anchor.is_empty() {
            add_camera_subject(
                subjects,
                anchor.as_str(),
                visible.as_str(),
                "foreground shoulder establishes the declared viewpoint without obscuring the focus",
            );
        }
        if policy != "not_applicable" && !eyeline.is_empty() {
            add_camera_subject(
                subjects,
                eyeline.as_str(),
                visible.as_str(),
                "clearly visible while executing the declared eyeline",
            );
        }
        let minimum = match framing.as_str() {
            "two_shot" => 2,
            "group" => 3,
            _ => 0,
        };
        let mut index = 1;
        while subjects.len() < minimum {
            let id = format!("camera_support_{index}");
            index += 1;
            add_camera_subject(
                subjects,
                id.as_str(),
                visible.as_str(),
                "clearly visible within the planned multi-subject composition",
            );
        }
    }
    Ok(())
}

fn add_camera_subject(subjects: &mut Vec<Value>, id: &str, anchor: &str, blocking: &str) {
    if subjects
        .iter()
        .any(|subject| subject.get("id").and_then(Value::as_str) == Some(id))
    {
        return;
    }
    subjects.push(json!({
        "id": id,
        "figure": format!("scene-supported subject from {anchor}"),
        "pose": "visibly participating in the planned composition",
        "expression": "scene-consistent visible state",
        "blocking": blocking
    }));
}

fn normalize_eyelines(scene: &mut Value) -> Result<()> {
    let policy = String::from(
        scene
            .pointer("/manga_panel/page_design/camera_arc/continuity/eyeline_policy")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("registered scene must contain an eyeline policy"))?,
    );
    let panels = scene
        .pointer_mut("/manga_panel/panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registered scene must contain panels"))?;
    let mut relations: BTreeMap<(String, String), String> = BTreeMap::new();
    for panel in panels {
        let eyeline = panel
            .pointer_mut("/continuity/eyeline")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("registered panel must contain eyeline continuity"))?;
        if policy == "not_applicable" {
            eyeline.insert(String::from("enabled"), Value::Bool(false));
            eyeline.insert(String::from("looker_id"), Value::String(String::new()));
            eyeline.insert(String::from("target_anchor"), Value::String(String::new()));
            eyeline.insert(
                String::from("direction"),
                Value::String(String::from("none")),
            );
            continue;
        }
        if policy != "matched" || eyeline.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let looker = eyeline
            .get("looker_id")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("enabled eyeline must name one looker"))?;
        let target = eyeline
            .get("target_anchor")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("enabled eyeline must name one target"))?;
        let current = eyeline
            .get("direction")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("enabled eyeline must name one direction"))?;
        let direct = (looker.clone(), target.clone());
        let reciprocal = (target, looker);
        let direction = relations
            .get(&direct)
            .cloned()
            .or_else(|| {
                relations
                    .get(&reciprocal)
                    .and_then(|direction| opposite_eyeline(direction))
                    .map(String::from)
            })
            .unwrap_or(current);
        eyeline.insert(String::from("direction"), Value::String(direction.clone()));
        relations.insert(direct, direction);
    }
    Ok(())
}

fn normalize_match_on_action(scene: &mut Value) -> Result<()> {
    let panels = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("registered scene must contain panels"))?;
    let disabled = panels
        .iter()
        .enumerate()
        .map(|(index, _)| Ok((index, !match_on_action_supported(panels, index)?)))
        .collect::<Result<Vec<_>>>()?;
    let panels = scene
        .pointer_mut("/manga_panel/panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registered scene must contain panels"))?;
    for (index, disable) in disabled {
        if disable {
            panels[index]["continuity"]["match_on_action"] =
                json!({"enabled": false, "subject_id": "", "action": ""});
        }
    }
    Ok(())
}

fn match_on_action_supported(panels: &[Value], index: usize) -> Result<bool> {
    let panel = panels
        .get(index)
        .ok_or_else(|| anyhow!("match-on-action panel index is unavailable"))?;
    let enabled = required_bool(
        panel,
        "/continuity/match_on_action/enabled",
        "panel continuity.match_on_action.enabled",
    )?;
    let subject = required_string(
        panel,
        "/continuity/match_on_action/subject_id",
        "panel continuity.match_on_action.subject_id",
    )?;
    let action = required_string(
        panel,
        "/continuity/match_on_action/action",
        "panel continuity.match_on_action.action",
    )?;
    if !enabled {
        return Ok(subject.is_empty() && action.is_empty());
    }
    let Some(previous) = index.checked_sub(1).and_then(|value| panels.get(value)) else {
        return Ok(false);
    };
    let shared = required_string(
        panel,
        "/continuity/shared_environment_id",
        "panel continuity.shared_environment_id",
    )?;
    let previous_shared = required_string(
        previous,
        "/continuity/shared_environment_id",
        "previous panel continuity.shared_environment_id",
    )?;
    let transition = required_string(
        panel,
        "/transition_from_previous",
        "panel transition_from_previous",
    )?;
    Ok(!subject.is_empty()
        && !action.is_empty()
        && !shared.is_empty()
        && shared == previous_shared
        && transition == "action_to_action"
        && has_subject(previous, subject)
        && has_subject(panel, subject))
}

fn normalize_subject_expressions(scene: &mut Value) -> Result<()> {
    let panels = scene
        .pointer_mut("/manga_panel/panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("registered scene must contain panels"))?;
    for panel in panels {
        let subjects = panel
            .pointer_mut("/scene/subjects")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("registered panel must contain subjects"))?;
        for subject in subjects {
            let root = subject
                .as_object_mut()
                .ok_or_else(|| anyhow!("registered panel subjects must be objects"))?;
            let expression = root
                .get("expression")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("registered panel subject must contain expression"))?;
            if expression.trim().is_empty() {
                root.insert(
                    String::from("expression"),
                    Value::String(String::from("scene-consistent visible state")),
                );
            }
        }
    }
    Ok(())
}

fn nest_continuity(
    continuity: &mut Map<String, Value>,
    target: &str,
    fields: &[(&str, &str)],
) -> Result<()> {
    let mut nested = Map::new();
    for (source, name) in fields {
        let value = continuity
            .remove(*source)
            .ok_or_else(|| anyhow!("registry scene composer continuity must contain '{source}'"))?;
        nested.insert(String::from(*name), value);
    }
    continuity.insert(String::from(target), Value::Object(nested));
    Ok(())
}

fn merge_dynamic(root: &mut Map<String, Value>, fields: &mut Map<String, Value>) -> Result<()> {
    for name in ["semantic_spine", "page_design", "panels"] {
        let value = fields
            .remove(name)
            .ok_or_else(|| anyhow!("dynamic scene object must contain '{name}'"))?;
        root.insert(String::from(name), value);
    }
    strip_agent_policy(root);
    Ok(())
}

fn strip_agent_policy(root: &mut Map<String, Value>) {
    if let Some(page) = root.get_mut("page_design").and_then(Value::as_object_mut) {
        page.remove("special_device_budget");
    }
}

fn validate_dynamic(scene: &Value) -> Result<()> {
    if scene
        .pointer("/manga_panel/meta/spec_version")
        .and_then(Value::as_str)
        != Some(DYNAMIC_SPEC)
    {
        return Ok(());
    }
    let root = scene
        .pointer("/manga_panel")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("dynamic scene must contain a manga_panel object"))?;
    if root
        .get("panel_layout")
        .and_then(Value::as_object)
        .and_then(|layout| layout.get("special_device_budget"))
        .and_then(Value::as_i64)
        != Some(1)
    {
        bail!("manga_panel.panel_layout.special_device_budget must equal 1");
    }
    validate_semantic(root)?;
    let panels = root
        .get("panels")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("dynamic scene must contain a panels array"))?;
    if !(1..=4).contains(&panels.len()) {
        bail!("dynamic scene must contain between 1 and 4 panels");
    }
    let ids = panel_ids(panels)?;
    for panel in panels {
        validate_panel(panel)?;
    }
    let page = root
        .get("page_design")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("dynamic scene must contain a page_design object"))?;
    let registered = page.get("layout").is_some_and(Value::is_object);
    validate_page(page, &ids, registered)?;
    let device = device(page, panels, &ids)?;
    validate_kind(panels, &device)?;
    validate_frames(panels, &ids, &device, registered)?;
    validate_crossing(panels, &ids, &device)?;
    validate_master(panels, &device)?;
    if registered {
        validate_camera_program(page, panels)?;
    }
    Ok(())
}

/// Validate one persisted scene against the current production scene contract.
pub(crate) fn validate_cached(scene: &Value) -> Result<()> {
    if scene
        .pointer("/manga_panel/meta/spec_version")
        .and_then(Value::as_str)
        != Some(DYNAMIC_SPEC)
    {
        bail!("cached scene must use production spec version {DYNAMIC_SPEC}");
    }
    validate_dynamic(scene)?;
    validate(scene)
}

fn validate_camera_program(page: &Map<String, Value>, panels: &[Value]) -> Result<()> {
    let arc = page
        .get("camera_arc")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("registered scene must contain a motivated camera arc"))?;
    for field in ["strategy", "progression", "motivation"] {
        required_nonempty_field(arc, field, &format!("page_design.camera_arc.{field}"))?;
    }
    let policy = arc
        .get("continuity")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("camera arc must contain a continuity policy"))?;
    let axis_mode = required_enum_field(
        policy,
        "axis_mode",
        "page_design.camera_arc.continuity.axis_mode",
        &[
            "not_applicable",
            "preserve",
            "reestablish",
            "deliberate_break",
        ],
    )?;
    let axis = required_field(policy, "axis", "page_design.camera_arc.continuity.axis")?;
    let screen_direction = required_enum_field(
        policy,
        "screen_direction",
        "page_design.camera_arc.continuity.screen_direction",
        &[
            "not_applicable",
            "stationary",
            "left_to_right",
            "right_to_left",
            "toward_camera",
            "away_from_camera",
            "converging",
            "diverging",
        ],
    )?;
    let eyeline_policy = required_enum_field(
        policy,
        "eyeline_policy",
        "page_design.camera_arc.continuity.eyeline_policy",
        &["not_applicable", "matched", "deliberately_broken"],
    )?;
    let mut relations = Vec::with_capacity(panels.len());
    let mut eyelines: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    let mut eyeline_break = false;
    let mut enabled_eyelines = 0;
    for (index, panel) in panels.iter().enumerate() {
        let id = required_string(panel, "/id", "panel id")?;
        validate_shot_contract(panel, id)?;
        validate_viewpoint(panel, id)?;
        let edit = validate_edit_continuity(panel, panels, index, id)?;
        if edit.screen_direction != screen_direction {
            bail!("panel '{id}' screen direction contradicts the scene-level camera plan");
        }
        relations.push(edit.axis_relation);
        if edit.eyeline.enabled {
            enabled_eyelines += 1;
            let key = (edit.eyeline.looker, edit.eyeline.target);
            eyeline_break |= eyelines
                .get(&key)
                .is_some_and(|direction| *direction != edit.eyeline.direction);
            eyeline_break |= eyelines
                .get(&(edit.eyeline.target, edit.eyeline.looker))
                .is_some_and(|direction| !opposite_eyelines(direction, edit.eyeline.direction));
            eyelines.entry(key).or_insert(edit.eyeline.direction);
        }
    }
    validate_axis_execution(axis_mode, axis, relations.as_slice())?;
    validate_eyeline_execution(eyeline_policy, enabled_eyelines, eyeline_break)?;
    Ok(())
}

fn validate_axis_execution(axis_mode: &str, axis: &str, relations: &[&str]) -> Result<()> {
    if (axis_mode == "not_applicable") != axis.is_empty() {
        bail!("camera axis must be named exactly when the scene-level plan uses one");
    }
    let first = relations
        .first()
        .copied()
        .ok_or_else(|| anyhow!("camera axis execution requires at least one panel"))?;
    let later = &relations[1..];
    let valid = match axis_mode {
        "not_applicable" => relations
            .iter()
            .all(|relation| *relation == "not_applicable"),
        "preserve" => first == "establish" && later.iter().all(|relation| *relation == "preserve"),
        "reestablish" => {
            first == "establish"
                && later
                    .iter()
                    .all(|relation| matches!(*relation, "preserve" | "reestablish"))
                && later.contains(&"reestablish")
        }
        "deliberate_break" => {
            first == "establish"
                && later
                    .iter()
                    .all(|relation| matches!(*relation, "preserve" | "deliberate_break"))
                && later.contains(&"deliberate_break")
        }
        _ => false,
    };
    if !valid {
        bail!("panel continuity contradicts the scene-level camera axis strategy");
    }
    Ok(())
}

fn validate_eyeline_execution(policy: &str, enabled: usize, broken: bool) -> Result<()> {
    let valid = match policy {
        "not_applicable" => enabled == 0,
        "matched" => enabled > 0 && !broken,
        "deliberately_broken" => enabled > 1 && broken,
        _ => false,
    };
    if !valid {
        bail!("panel eyelines contradict the scene-level camera eyeline policy");
    }
    Ok(())
}

fn opposite_eyelines(first: &str, second: &str) -> bool {
    opposite_eyeline(first) == Some(second)
}

fn opposite_eyeline(direction: &str) -> Option<&'static str> {
    match direction {
        "screen_left" => Some("screen_right"),
        "screen_right" => Some("screen_left"),
        "up" => Some("down"),
        "down" => Some("up"),
        "toward_camera" => Some("away_from_camera"),
        "away_from_camera" => Some("toward_camera"),
        _ => None,
    }
}

fn validate_shot_contract(panel: &Value, id: &str) -> Result<()> {
    let contract = panel
        .get("shot_contract")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("panel '{id}' must contain its immutable shot contract"))?;
    for (field, path) in [
        ("shot_scale", "/scene/camera/shot_scale"),
        ("viewpoint", "/scene/camera/viewpoint"),
        ("framing", "/scene/camera/framing"),
        ("angle", "/scene/camera/angle"),
        ("depth_plan", "/scene/camera/depth_plan"),
    ] {
        if contract.get(field).and_then(Value::as_str)
            != panel.pointer(path).and_then(Value::as_str)
        {
            bail!("panel '{id}' changed immutable shot-contract field '{field}'");
        }
    }
    for field in ["camera_motivation", "information_gain"] {
        required_nonempty_field(
            contract,
            field,
            &format!("panel '{id}' shot_contract.{field}"),
        )?;
    }
    Ok(())
}

fn validate_viewpoint(panel: &Value, id: &str) -> Result<()> {
    let viewpoint = required_enum(
        panel,
        "/scene/camera/viewpoint",
        &format!("panel '{id}' camera.viewpoint"),
        &[
            "objective",
            "over_the_shoulder",
            "point_of_view",
            "subjective",
        ],
    )?;
    let anchor = required_string(
        panel,
        "/scene/camera/viewpoint_subject_id",
        &format!("panel '{id}' camera.viewpoint_subject_id"),
    )?;
    let framing = required_enum(
        panel,
        "/scene/camera/framing",
        &format!("panel '{id}' camera.framing"),
        &[
            "environment",
            "single",
            "two_shot",
            "group",
            "insert",
            "cutaway",
        ],
    )?;
    let subjects = required_array(panel, "/scene/subjects", &format!("panel '{id}' subjects"))?;
    let subject_ids = subjects
        .iter()
        .filter_map(|subject| subject.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if (viewpoint == "objective" && !anchor.is_empty())
        || (viewpoint != "objective" && anchor.is_empty())
        || (viewpoint == "over_the_shoulder" && !subject_ids.contains(anchor))
        || (framing == "two_shot" && subjects.len() < 2)
        || (framing == "group" && subjects.len() < 3)
    {
        bail!("panel '{id}' camera viewpoint or framing lacks its required subject support");
    }
    if framing == "insert"
        && panel
            .get("shot_contract")
            .and_then(Value::as_object)
            .and_then(|value| value.get("role"))
            .and_then(Value::as_str)
            != Some("detail")
    {
        bail!("panel '{id}' insert framing must execute a detail shot");
    }
    Ok(())
}

fn validate_edit_continuity<'a>(
    panel: &'a Value,
    panels: &'a [Value],
    index: usize,
    id: &str,
) -> Result<EditContinuity<'a>> {
    let relation = required_enum(
        panel,
        "/continuity/axis_relation_from_previous",
        &format!("panel '{id}' continuity.axis_relation_from_previous"),
        &[
            "not_applicable",
            "establish",
            "preserve",
            "reestablish",
            "deliberate_break",
        ],
    )?;
    let direction = required_enum(
        panel,
        "/continuity/screen_direction",
        &format!("panel '{id}' continuity.screen_direction"),
        &[
            "not_applicable",
            "stationary",
            "left_to_right",
            "right_to_left",
            "toward_camera",
            "away_from_camera",
            "converging",
            "diverging",
        ],
    )?;
    let eyeline = validate_eyeline(panel, id)?;
    validate_match_on_action(panel, panels, index, id)?;
    if index == 0 && !matches!(relation, "not_applicable" | "establish") {
        bail!("first panel '{id}' cannot preserve or break an unestablished camera axis");
    }
    Ok(EditContinuity {
        axis_relation: relation,
        screen_direction: direction,
        eyeline,
    })
}

fn validate_eyeline<'a>(panel: &'a Value, id: &str) -> Result<Eyeline<'a>> {
    let enabled = required_bool(
        panel,
        "/continuity/eyeline/enabled",
        &format!("panel '{id}' continuity.eyeline.enabled"),
    )?;
    let looker = required_string(
        panel,
        "/continuity/eyeline/looker_id",
        &format!("panel '{id}' continuity.eyeline.looker_id"),
    )?;
    let target = required_string(
        panel,
        "/continuity/eyeline/target_anchor",
        &format!("panel '{id}' continuity.eyeline.target_anchor"),
    )?;
    let direction = required_enum(
        panel,
        "/continuity/eyeline/direction",
        &format!("panel '{id}' continuity.eyeline.direction"),
        &[
            "none",
            "screen_left",
            "screen_right",
            "up",
            "down",
            "toward_camera",
            "away_from_camera",
        ],
    )?;
    if (enabled && (looker.is_empty() || target.is_empty() || direction == "none"))
        || (!enabled && (!looker.is_empty() || !target.is_empty() || direction != "none"))
    {
        bail!("panel '{id}' eyeline fields disagree with their enabled flag");
    }
    if enabled && !has_subject(panel, looker) {
        bail!("panel '{id}' eyeline looker '{looker}' is not visible in the shot");
    }
    Ok(Eyeline {
        enabled,
        looker,
        target,
        direction,
    })
}

fn validate_match_on_action(panel: &Value, panels: &[Value], index: usize, id: &str) -> Result<()> {
    let enabled = required_bool(
        panel,
        "/continuity/match_on_action/enabled",
        &format!("panel '{id}' continuity.match_on_action.enabled"),
    )?;
    let subject = required_string(
        panel,
        "/continuity/match_on_action/subject_id",
        &format!("panel '{id}' continuity.match_on_action.subject_id"),
    )?;
    let action = required_string(
        panel,
        "/continuity/match_on_action/action",
        &format!("panel '{id}' continuity.match_on_action.action"),
    )?;
    if !enabled {
        if !subject.is_empty() || !action.is_empty() {
            bail!("panel '{id}' disabled match_on_action must not name an action");
        }
        return Ok(());
    }
    let previous = index
        .checked_sub(1)
        .and_then(|value| panels.get(value))
        .ok_or_else(|| anyhow!("first panel '{id}' cannot match action from a previous shot"))?;
    let shared = required_string(
        panel,
        "/continuity/shared_environment_id",
        &format!("panel '{id}' continuity.shared_environment_id"),
    )?;
    let previous_shared = required_string(
        previous,
        "/continuity/shared_environment_id",
        "previous panel continuity.shared_environment_id",
    )?;
    let subject_present = [previous, panel].iter().all(|value| {
        value
            .pointer("/scene/subjects")
            .and_then(Value::as_array)
            .is_some_and(|subjects| {
                subjects
                    .iter()
                    .any(|value| value.get("id").and_then(Value::as_str) == Some(subject))
            })
    });
    if subject.is_empty()
        || action.is_empty()
        || shared.is_empty()
        || shared != previous_shared
        || !subject_present
        || required_string(
            panel,
            "/transition_from_previous",
            "panel transition_from_previous",
        )? != "action_to_action"
    {
        bail!("panel '{id}' match_on_action lacks one continuous supported action");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Device<'a> {
    kind: &'a str,
    source: &'a str,
    target: &'a str,
    subject: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PanelBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

fn panel_ids(panels: &[Value]) -> Result<BTreeSet<&str>> {
    let mut ids = BTreeSet::new();
    for panel in panels {
        let id = required_string(panel, "/id", "panel id")?;
        if id.trim().is_empty() {
            bail!("panel id cannot be empty");
        }
        if !ids.insert(id) {
            bail!("panel id '{id}' is duplicated");
        }
    }
    Ok(ids)
}

fn validate_semantic(root: &Map<String, Value>) -> Result<()> {
    let spine = root
        .get("semantic_spine")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("dynamic scene must contain a semantic_spine object"))?;
    for field in [
        "literal_event",
        "semantic_focus",
        "emotional_relation",
        "memory_hook",
    ] {
        required_nonempty_field(spine, field, &format!("semantic_spine.{field}"))?;
    }
    let intensity = spine
        .get("intensity")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("semantic_spine.intensity must be an integer"))?;
    if !(1..=5).contains(&intensity) {
        bail!("semantic_spine.intensity must be between 1 and 5");
    }
    required_enum_field(
        spine,
        "visual_relation",
        "semantic_spine.visual_relation",
        &[
            "containment",
            "distance",
            "opposition",
            "contrast",
            "burden",
            "repetition",
            "threshold",
            "balance",
            "cause_effect",
            "approach",
            "avoidance",
            "other",
        ],
    )?;
    let metaphor = spine
        .get("metaphor")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("semantic_spine.metaphor must be an object"))?;
    required_enum_field(
        metaphor,
        "mode",
        "semantic_spine.metaphor.mode",
        &["none", "juxtaposition", "fusion"],
    )?;
    required_field(metaphor, "mapping", "semantic_spine.metaphor.mapping")?;
    required_field(
        metaphor,
        "literal_anchor",
        "semantic_spine.metaphor.literal_anchor",
    )?;
    Ok(())
}

fn validate_panel(panel: &Value) -> Result<()> {
    let id = required_string(panel, "/id", "panel id")?;
    validate_panel_schema(panel, id)?;
    let bounds = panel_bounds(panel, id)?;
    if bounds.left < PANEL_MIN
        || bounds.top < PANEL_MIN
        || bounds.right > PANEL_MAX
        || bounds.bottom > PANEL_MAX
    {
        bail!("panel '{id}' bounds must stay within 16..1008 with positive dimensions");
    }
    if panel.get("bleed").and_then(Value::as_bool) != Some(false) {
        bail!("panel '{id}' must keep bleed false");
    }
    Ok(())
}

fn validate_panel_schema(panel: &Value, id: &str) -> Result<()> {
    required_enum(
        panel,
        "/narrative_role",
        &format!("panel '{id}' narrative_role"),
        &["establisher", "initial", "peak", "release"],
    )?;
    required_nonempty_string(
        panel,
        "/semantic_job",
        &format!("panel '{id}' semantic_job"),
    )?;
    let attention = required_enum(
        panel,
        "/attentional_frame",
        &format!("panel '{id}' attentional_frame"),
        &["macro", "mono", "micro", "amorphic"],
    )?;
    required_enum(
        panel,
        "/narrative_weight",
        &format!("panel '{id}' narrative_weight"),
        &["primary", "secondary", "transition"],
    )?;
    let transitions = [
        "none",
        "moment_to_moment",
        "action_to_action",
        "subject_to_subject",
        "scene_to_scene",
        "aspect_to_aspect",
    ]
    .as_slice();
    required_enum(
        panel,
        "/transition_from_previous",
        &format!("panel '{id}' transition_from_previous"),
        transitions,
    )?;
    validate_continuity(panel, id)?;
    validate_scene(panel, id, attention)?;
    Ok(())
}

fn validate_continuity(panel: &Value, id: &str) -> Result<()> {
    required_string(
        panel,
        "/continuity/shared_environment_id",
        &format!("panel '{id}' continuity.shared_environment_id"),
    )?;
    required_string(
        panel,
        "/continuity/subject_phase",
        &format!("panel '{id}' continuity.subject_phase"),
    )?;
    required_bool(
        panel,
        "/continuity/breakout/enabled",
        &format!("panel '{id}' continuity.breakout.enabled"),
    )?;
    required_string(
        panel,
        "/continuity/breakout/subject_id",
        &format!("panel '{id}' continuity.breakout.subject_id"),
    )?;
    required_enum(
        panel,
        "/continuity/breakout/edge",
        &format!("panel '{id}' continuity.breakout.edge"),
        &["left", "right", "top", "bottom", "empty"],
    )?;
    required_string(
        panel,
        "/continuity/breakout/destination_panel",
        &format!("panel '{id}' continuity.breakout.destination_panel"),
    )?;
    Ok(())
}

fn validate_scene(panel: &Value, id: &str, attention: &str) -> Result<()> {
    required_nonempty_string(
        panel,
        "/scene/description",
        &format!("panel '{id}' scene.description"),
    )?;
    let subjects = required_array(
        panel,
        "/scene/subjects",
        &format!("panel '{id}' scene.subjects"),
    )?;
    if subjects.is_empty() && !matches!(attention, "macro" | "amorphic") {
        bail!("panel '{id}' can omit subjects only for a macro or amorphic frame");
    }
    let mut ids = BTreeSet::new();
    for subject in subjects {
        let object = subject
            .as_object()
            .ok_or_else(|| anyhow!("panel '{id}' subjects must be objects"))?;
        let subject_id =
            required_nonempty_field(object, "id", &format!("panel '{id}' subject id"))?;
        if !ids.insert(subject_id) {
            bail!("panel '{id}' subject id '{subject_id}' is duplicated");
        }
        for field in ["figure", "pose", "expression", "blocking"] {
            required_nonempty_field(object, field, &format!("panel '{id}' subject {field}"))?;
        }
    }
    let environment = required_object(
        panel,
        "/scene/environment",
        &format!("panel '{id}' scene.environment"),
    )?;
    required_nonempty_field(
        environment,
        "setting",
        &format!("panel '{id}' environment.setting"),
    )?;
    for field in ["foreground", "midground", "background"] {
        required_string_array_field(
            environment,
            field,
            &format!("panel '{id}' environment.{field}"),
        )?;
    }
    required_enum(
        panel,
        "/scene/camera/shot_scale",
        &format!("panel '{id}' camera.shot_scale"),
        &[
            "extreme_wide",
            "wide",
            "full",
            "medium",
            "medium_close",
            "close",
            "extreme_close",
        ],
    )?;
    required_enum(
        panel,
        "/scene/camera/angle",
        &format!("panel '{id}' camera.angle"),
        &["eye_level", "high", "low", "overhead", "dutch"],
    )?;
    required_nonempty_string(
        panel,
        "/scene/camera/focus",
        &format!("panel '{id}' camera.focus"),
    )?;
    required_enum(
        panel,
        "/scene/camera/depth_plan",
        &format!("panel '{id}' camera.depth_plan"),
        &["deep", "layered", "shallow", "flat"],
    )?;
    required_nonempty_string(
        panel,
        "/scene/camera/eye_flow_exit",
        &format!("panel '{id}' camera.eye_flow_exit"),
    )?;
    required_enum(
        panel,
        "/scene/motion_treatment",
        &format!("panel '{id}' scene.motion_treatment"),
        &["none", "pose_only", "speed_lines", "blur"],
    )?;
    required_nonempty_string(
        panel,
        "/scene/lighting",
        &format!("panel '{id}' scene.lighting"),
    )?;
    required_nonempty_string(panel, "/scene/mood", &format!("panel '{id}' scene.mood"))?;
    if required_string(
        panel,
        "/scene/text_in_frame",
        &format!("panel '{id}' scene.text_in_frame"),
    )? != "none"
    {
        bail!("panel '{id}' scene.text_in_frame must equal 'none'");
    }
    Ok(())
}

fn panel_bounds(panel: &Value, id: &str) -> Result<PanelBounds> {
    let x = required_integer(panel, "/bounds/x", id)?;
    let y = required_integer(panel, "/bounds/y", id)?;
    let width = required_integer(panel, "/bounds/width", id)?;
    let height = required_integer(panel, "/bounds/height", id)?;
    if width <= 0 || height <= 0 {
        bail!("panel '{id}' bounds must use positive dimensions");
    }
    let right = x
        .checked_add(width)
        .ok_or_else(|| anyhow!("panel '{id}' horizontal bounds overflow"))?;
    let bottom = y
        .checked_add(height)
        .ok_or_else(|| anyhow!("panel '{id}' vertical bounds overflow"))?;
    Ok(PanelBounds {
        left: x,
        top: y,
        right,
        bottom,
    })
}

fn validate_page(page: &Map<String, Value>, ids: &BTreeSet<&str>, registered: bool) -> Result<()> {
    let rhythms = [
        "single_tableau",
        "regular",
        "compression_to_release",
        "expansion",
        "contraction",
    ]
    .as_slice();
    required_enum_field(page, "rhythm", "page_design.rhythm", rhythms)?;
    validate_archetype(page, registered)?;
    let dominant = page
        .get("dominant_panel")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("page_design.dominant_panel must be a string"))?;
    if dominant.is_empty() && !registered {
        bail!("page_design.dominant_panel must be nonempty");
    }
    if !dominant.is_empty() && !ids.contains(dominant) {
        bail!("dominant panel '{dominant}' does not exist");
    }
    let path = page
        .get("reading_path")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("page_design.reading_path must be an array"))?;
    let mut read = BTreeSet::new();
    for value in path {
        let id = value
            .as_str()
            .filter(|item| !item.trim().is_empty())
            .ok_or_else(|| anyhow!("reading_path panel ids must be nonempty strings"))?;
        if !ids.contains(id) || !read.insert(id) {
            bail!("reading_path must contain every panel id exactly once");
        }
    }
    if path.len() != ids.len() || &read != ids {
        bail!("reading_path must contain every panel id exactly once");
    }
    required_nonempty_field(page, "eye_flow_summary", "page_design.eye_flow_summary")?;
    required_nonempty_field(
        page,
        "layout_rendering_directive",
        "page_design.layout_rendering_directive",
    )?;
    Ok(())
}

fn validate_archetype(page: &Map<String, Value>, registered: bool) -> Result<()> {
    if registered {
        let archetype = required_nonempty_field(page, "archetype", "page_design.archetype")?;
        let template = page
            .get("layout")
            .and_then(Value::as_object)
            .and_then(|layout| layout.get("template_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("page_design.layout.template_id must be a string"))?;
        if archetype != template {
            bail!("page_design.archetype must match the registered template id");
        }
        return Ok(());
    }
    required_enum_field(
        page,
        "archetype",
        "page_design.archetype",
        &[
            "single_splash",
            "calm_asymmetry",
            "dominant_with_inset",
            "master_view",
            "compression_release",
        ],
    )?;
    Ok(())
}

fn device<'a>(
    page: &'a Map<String, Value>,
    panels: &'a [Value],
    ids: &BTreeSet<&str>,
) -> Result<Device<'a>> {
    let value = page
        .get("special_device")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("page_design.special_device must be an object"))?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("special_device.kind must be a string"))?;
    if !matches!(
        kind,
        "none"
            | "single_splash"
            | "inset"
            | "master_view"
            | "crossing"
            | "overlap"
            | "diagonal_release"
            | "open_frame"
    ) {
        bail!("special_device.kind '{kind}' is unknown");
    }
    required_nonempty_field(value, "reason", "special_device.reason")?;
    let source = required_field(value, "source_panel", "special_device.source_panel")?;
    let target = required_field(value, "target_panel", "special_device.target_panel")?;
    let subject = required_field(value, "subject_id", "special_device.subject_id")?;
    validate_known_reference(source, ids, "special_device.source_panel")?;
    validate_known_reference(target, ids, "special_device.target_panel")?;
    if !source.is_empty() && source == target {
        bail!("special_device source and target panels cannot be the same");
    }
    if !subject.is_empty() {
        let exists = if source.is_empty() {
            panels.iter().any(|panel| has_subject(panel, subject))
        } else {
            panel_by_id(panels, source).is_some_and(|panel| has_subject(panel, subject))
        };
        if !exists {
            bail!("special_device subject '{subject}' does not exist in its source panel");
        }
    }
    Ok(Device {
        kind,
        source,
        target,
        subject,
    })
}

fn validate_kind(panels: &[Value], device: &Device<'_>) -> Result<()> {
    if device.kind == "single_splash" && panels.len() != 1 {
        bail!("single_splash requires exactly one panel");
    }
    if device.kind == "single_splash"
        && (!device.source.is_empty() || !device.target.is_empty() || !device.subject.is_empty())
    {
        bail!("single_splash requires empty source, target, and subject references");
    }
    if device.kind == "none"
        && (!device.source.is_empty() || !device.target.is_empty() || !device.subject.is_empty())
    {
        bail!("none special device requires empty source, target, and subject references");
    }
    if !matches!(device.kind, "master_view" | "crossing") && !device.subject.is_empty() {
        bail!("only master_view and crossing may carry a special-device subject reference");
    }
    Ok(())
}

fn validate_frames(
    panels: &[Value],
    ids: &BTreeSet<&str>,
    device: &Device<'_>,
    registered: bool,
) -> Result<()> {
    let mut insets = 0usize;
    let mut overlaps = 0usize;
    let mut trapezoids = Vec::new();
    let mut open_frames = Vec::new();
    for panel in panels {
        let id = required_string(panel, "/id", "panel id")?;
        let frame = panel
            .get("frame")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("panel '{id}' must contain a frame object"))?;
        required_nonempty_field(
            frame,
            "geometry_intent",
            &format!("panel '{id}' frame.geometry_intent"),
        )?;
        let z_index = frame_z(frame, id)?;
        let bounds = panel_bounds(panel, id)?;
        let shape = frame
            .get("shape")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("panel '{id}' frame.shape must be a string"))?;
        if !matches!(
            shape,
            "rectangle"
                | "wide_rectangle"
                | "tall_rectangle"
                | "trapezoid"
                | "inset"
                | "open_frame"
        ) {
            bail!("panel '{id}' frame shape '{shape}' is unknown");
        }
        let polygon = frame
            .get("polygon")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("panel '{id}' frame.polygon must be an array"))?;
        if shape == "trapezoid" {
            trapezoids.push(id);
            if polygon.len() != 4 || (!registered && device.kind != "diagonal_release") {
                bail!("panel '{id}' trapezoid requires four points and an enabled topology");
            }
            validate_polygon(polygon, bounds, id)?;
        } else if !polygon.is_empty() {
            bail!("panel '{id}' non-trapezoid frame must use an empty polygon");
        }
        let border = frame
            .get("border")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("panel '{id}' frame.border must be a string"))?;
        if !matches!(border, "solid" | "none") {
            bail!("panel '{id}' frame border '{border}' is unknown");
        }
        if shape == "open_frame" {
            open_frames.push(id);
            if border != "none" || device.kind != "open_frame" {
                bail!("panel '{id}' open_frame requires border none and open_frame device");
            }
        } else if border != "solid" {
            bail!("panel '{id}' non-open frame must use a solid border");
        }
        let parent = required_field(frame, "parent_panel", "frame.parent_panel")?;
        validate_panel_reference(parent, id, ids, "frame.parent_panel")?;
        if shape == "inset" {
            insets += 1;
            if parent.is_empty() || device.kind != "inset" {
                bail!("panel '{id}' inset requires a parent and inset device");
            }
            let parent_panel = panel_by_id(panels, parent)
                .ok_or_else(|| anyhow!("inset parent panel '{parent}' does not exist"))?;
            let parent_bounds = panel_bounds(parent_panel, parent)?;
            let parent_frame = parent_panel
                .get("frame")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("inset parent panel '{parent}' has no frame"))?;
            if !contains(parent_bounds, bounds) || z_index <= frame_z(parent_frame, parent)? {
                bail!("inset panel '{id}' must be contained in and layered above '{parent}'");
            }
            if device.source != parent || device.target != id {
                bail!("inset device source and target must match its parent and child panels");
            }
        } else if !parent.is_empty() {
            bail!("panel '{id}' parent_panel is reserved for inset frames");
        }
        let overlap = required_field(frame, "overlaps_panel", "frame.overlaps_panel")?;
        validate_panel_reference(overlap, id, ids, "frame.overlaps_panel")?;
        if !overlap.is_empty() {
            overlaps += 1;
            if device.kind != "overlap" {
                bail!("panel '{id}' overlaps_panel requires overlap device");
            }
            let target_panel = panel_by_id(panels, overlap)
                .ok_or_else(|| anyhow!("overlap target panel '{overlap}' does not exist"))?;
            let target_bounds = panel_bounds(target_panel, overlap)?;
            let target_frame = target_panel
                .get("frame")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("overlap target panel '{overlap}' has no frame"))?;
            if !intersects(bounds, target_bounds) || z_index <= frame_z(target_frame, overlap)? {
                bail!("overlap panel '{id}' must intersect and layer above '{overlap}'");
            }
            if device.source != id || device.target != overlap {
                bail!("overlap device source and target must match the overlapping panel relation");
            }
        }
    }
    require_device_count(device.kind, "inset", insets)?;
    require_device_count(device.kind, "overlap", overlaps)?;
    if !registered {
        require_diagonal_count(device.kind, trapezoids.len())?;
        validate_diagonal_references(device, trapezoids.as_slice())?;
    }
    require_device_count(device.kind, "open_frame", open_frames.len())?;
    validate_open_frame_references(device, open_frames.as_slice())?;
    Ok(())
}

fn validate_diagonal_references(device: &Device<'_>, panels: &[&str]) -> Result<()> {
    if device.kind != "diagonal_release" {
        return Ok(());
    }
    if !device.subject.is_empty() {
        bail!("diagonal_release requires an empty subject reference");
    }
    let valid = match panels {
        [panel] => device.source == *panel && device.target.is_empty(),
        [first, second] => {
            (device.source == *first && device.target == *second)
                || (device.source == *second && device.target == *first)
        }
        _ => false,
    };
    if !valid {
        bail!("diagonal_release references must identify its one or two trapezoid panels");
    }
    Ok(())
}

fn validate_open_frame_references(device: &Device<'_>, panels: &[&str]) -> Result<()> {
    if device.kind != "open_frame" {
        return Ok(());
    }
    if panels.first().copied() != Some(device.source)
        || !device.target.is_empty()
        || !device.subject.is_empty()
    {
        bail!("open_frame source must identify its borderless panel with no other references");
    }
    Ok(())
}

fn frame_z(frame: &Map<String, Value>, id: &str) -> Result<i64> {
    frame
        .get("z_index")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| anyhow!("panel '{id}' frame.z_index must be a nonnegative integer"))
}

fn validate_polygon(points: &[Value], bounds: PanelBounds, id: &str) -> Result<()> {
    for point in points {
        let coordinates = point
            .as_array()
            .filter(|items| items.len() == 2)
            .ok_or_else(|| anyhow!("panel '{id}' polygon points must be [x,y] pairs"))?;
        let x = coordinates[0]
            .as_i64()
            .ok_or_else(|| anyhow!("panel '{id}' polygon x must be an integer"))?;
        let y = coordinates[1]
            .as_i64()
            .ok_or_else(|| anyhow!("panel '{id}' polygon y must be an integer"))?;
        if x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom {
            bail!("panel '{id}' polygon point [{x},{y}] lies outside its bounds");
        }
    }
    Ok(())
}

fn contains(parent: PanelBounds, child: PanelBounds) -> bool {
    child.left >= parent.left
        && child.top >= parent.top
        && child.right <= parent.right
        && child.bottom <= parent.bottom
}

fn intersects(left: PanelBounds, right: PanelBounds) -> bool {
    left.left.max(right.left) < left.right.min(right.right)
        && left.top.max(right.top) < left.bottom.min(right.bottom)
}

fn require_device_count(kind: &str, expected: &str, count: usize) -> Result<()> {
    if kind == expected && count != 1 {
        bail!("special device '{expected}' requires exactly one matching panel relation");
    }
    if kind != expected && count != 0 {
        bail!("panel relation '{expected}' requires matching special device");
    }
    Ok(())
}

fn require_diagonal_count(kind: &str, count: usize) -> Result<()> {
    if kind == "diagonal_release" && !(1..=2).contains(&count) {
        bail!("special device 'diagonal_release' requires one or two trapezoid panels");
    }
    if kind != "diagonal_release" && count != 0 {
        bail!("trapezoid panels require diagonal_release special device");
    }
    Ok(())
}

fn validate_crossing(panels: &[Value], ids: &BTreeSet<&str>, device: &Device<'_>) -> Result<()> {
    let mut enabled = 0usize;
    for panel in panels {
        let id = required_string(panel, "/id", "panel id")?;
        let Some(breakout) = panel
            .pointer("/continuity/breakout")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let active = optional_bool(breakout, "enabled", "continuity.breakout.enabled")?;
        let subject = optional_string(breakout, "subject_id", "continuity.breakout.subject_id")?;
        let destination = optional_string(
            breakout,
            "destination_panel",
            "continuity.breakout.destination_panel",
        )?;
        validate_panel_reference(
            destination,
            id,
            ids,
            "continuity.breakout.destination_panel",
        )?;
        if !subject.is_empty() && !has_subject(panel, subject) {
            bail!("breakout subject '{subject}' does not exist in panel '{id}'");
        }
        if active {
            enabled += 1;
            if device.kind != "crossing" || subject.is_empty() || destination.is_empty() {
                bail!("enabled breakout requires crossing device, subject, and destination");
            }
            if device.source != id || device.target != destination || device.subject != subject {
                bail!("crossing device references contradict the enabled breakout");
            }
        }
    }
    require_device_count(device.kind, "crossing", enabled)?;
    Ok(())
}

fn validate_master(panels: &[Value], device: &Device<'_>) -> Result<()> {
    if device.kind != "master_view" {
        return Ok(());
    }
    if device.source.is_empty() || device.target.is_empty() || device.subject.is_empty() {
        bail!("master_view requires source, target, and subject references");
    }
    let source = panel_by_id(panels, device.source)
        .ok_or_else(|| anyhow!("master_view source panel does not exist"))?;
    let target = panel_by_id(panels, device.target)
        .ok_or_else(|| anyhow!("master_view target panel does not exist"))?;
    let environment = optional_pointer_string(
        source,
        "/continuity/shared_environment_id",
        "continuity.shared_environment_id",
    )?;
    let target_environment = optional_pointer_string(
        target,
        "/continuity/shared_environment_id",
        "continuity.shared_environment_id",
    )?;
    if environment.is_empty() || environment != target_environment {
        bail!("master_view source and target must share one nonempty environment id");
    }
    let mut phases = BTreeSet::new();
    let mut participants = 0usize;
    for panel in panels {
        let shared = optional_pointer_string(
            panel,
            "/continuity/shared_environment_id",
            "continuity.shared_environment_id",
        )?;
        if shared != environment {
            continue;
        }
        participants += 1;
        if !has_subject(panel, device.subject) {
            bail!("master_view subject must exist in every participating panel");
        }
        let phase = optional_pointer_string(
            panel,
            "/continuity/subject_phase",
            "continuity.subject_phase",
        )?;
        if phase.is_empty() || !phases.insert(phase) {
            bail!("master_view subject phases must be nonempty and distinct");
        }
    }
    if participants < 2 {
        bail!("master_view requires at least two panels sharing one nonempty environment id");
    }
    Ok(())
}

fn specialize(scene: &mut Value) -> Result<()> {
    if scene
        .pointer("/manga_panel/meta/spec_version")
        .and_then(Value::as_str)
        != Some(DYNAMIC_SPEC)
    {
        return Ok(());
    }
    let kind = scene
        .pointer("/manga_panel/page_design/special_device/kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("special_device.kind must be a string"))?;
    let active_layout = scene.pointer("/manga_panel/page_design/layout").cloned();
    let permissions = serde_json::json!({
        "inset": kind == "inset",
        "master_view": kind == "master_view",
        "crossing": kind == "crossing",
        "overlap": kind == "overlap",
        "diagonal_release": kind == "diagonal_release",
        "open_frame": kind == "open_frame"
    });
    let root = scene
        .pointer_mut("/manga_panel")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("dynamic scene must contain a manga_panel object"))?;
    strip_agent_policy(root);
    if !root.get("panel_layout").is_some_and(Value::is_object) {
        root.insert(String::from("panel_layout"), Value::Object(Map::new()));
    }
    let panel_layout = root
        .get_mut("panel_layout")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("manga_panel.panel_layout must be an object"))?;
    panel_layout.insert(String::from("active_permissions"), permissions);
    if let Some(layout) = active_layout {
        panel_layout.insert(String::from("active_layout"), layout);
    }
    Ok(())
}

fn required_string<'a>(value: &'a Value, pointer: &str, label: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{label} must be a string"))
}

fn required_nonempty_string<'a>(value: &'a Value, pointer: &str, label: &str) -> Result<&'a str> {
    required_string(value, pointer, label).and_then(|item| {
        if item.trim().is_empty() {
            bail!("{label} must be nonempty");
        }
        Ok(item)
    })
}

fn required_object<'a>(
    value: &'a Value,
    pointer: &str,
    label: &str,
) -> Result<&'a Map<String, Value>> {
    value
        .pointer(pointer)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("{label} must be an object"))
}

fn required_array<'a>(value: &'a Value, pointer: &str, label: &str) -> Result<&'a [Value]> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow!("{label} must be an array"))
}

fn required_bool(value: &Value, pointer: &str, label: &str) -> Result<bool> {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow!("{label} must be a boolean"))
}

fn required_enum<'a>(
    value: &'a Value,
    pointer: &str,
    label: &str,
    allowed: &[&str],
) -> Result<&'a str> {
    let item = required_string(value, pointer, label)?;
    if !allowed.contains(&item) {
        bail!("{label} value '{item}' is unknown");
    }
    Ok(item)
}

fn required_field<'a>(value: &'a Map<String, Value>, field: &str, label: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{label} must be a string"))
}

fn required_nonempty_field<'a>(
    value: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str> {
    required_field(value, field, label).and_then(|item| {
        if item.trim().is_empty() {
            bail!("{label} must be nonempty");
        }
        Ok(item)
    })
}

fn required_enum_field<'a>(
    value: &'a Map<String, Value>,
    field: &str,
    label: &str,
    allowed: &[&str],
) -> Result<&'a str> {
    let item = required_field(value, field, label)?;
    if !allowed.contains(&item) {
        bail!("{label} value '{item}' is unknown");
    }
    Ok(item)
}

fn required_string_array_field(value: &Map<String, Value>, field: &str, label: &str) -> Result<()> {
    let items = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{label} must be an array"))?;
    if items
        .iter()
        .any(|item| item.as_str().is_none_or(|text| text.trim().is_empty()))
    {
        bail!("{label} must contain only nonempty strings");
    }
    Ok(())
}

fn required_integer(value: &Value, pointer: &str, id: &str) -> Result<i64> {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("panel '{id}' bounds must contain integers"))
}

fn optional_string<'a>(value: &'a Map<String, Value>, key: &str, label: &str) -> Result<&'a str> {
    match value.get(key) {
        None => Ok(""),
        Some(Value::String(item)) => Ok(item),
        Some(_) => bail!("{label} must be a string"),
    }
}

fn optional_pointer_string<'a>(value: &'a Value, pointer: &str, label: &str) -> Result<&'a str> {
    match value.pointer(pointer) {
        None => Ok(""),
        Some(Value::String(item)) => Ok(item),
        Some(_) => bail!("{label} must be a string"),
    }
}

fn optional_bool(value: &Map<String, Value>, key: &str, label: &str) -> Result<bool> {
    match value.get(key) {
        None => Ok(false),
        Some(Value::Bool(item)) => Ok(*item),
        Some(_) => bail!("{label} must be a boolean"),
    }
}

fn validate_known_reference(reference: &str, ids: &BTreeSet<&str>, label: &str) -> Result<()> {
    if !reference.is_empty() && !ids.contains(reference) {
        bail!("{label} references unknown panel '{reference}'");
    }
    Ok(())
}

fn validate_panel_reference(
    reference: &str,
    current: &str,
    ids: &BTreeSet<&str>,
    label: &str,
) -> Result<()> {
    validate_known_reference(reference, ids, label)?;
    if !reference.is_empty() && reference == current {
        bail!("{label} cannot reference its own panel '{current}'");
    }
    Ok(())
}

fn panel_by_id<'a>(panels: &'a [Value], id: &str) -> Option<&'a Value> {
    panels
        .iter()
        .find(|panel| panel.get("id").and_then(Value::as_str) == Some(id))
}

fn has_subject(panel: &Value, subject: &str) -> bool {
    panel
        .pointer("/scene/subjects")
        .and_then(Value::as_array)
        .is_some_and(|subjects| {
            subjects
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(subject))
        })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        normalize_camera_subjects, normalize_eyelines, normalize_match_on_action,
        normalize_panel_roles, normalize_subject_expressions, validate_dynamic,
    };

    fn panel(id: &str, x: i64, y: i64, width: i64, height: i64) -> Value {
        json!({
            "id": id,
            "bounds": {"x": x, "y": y, "width": width, "height": height},
            "bleed": false,
            "frame": {
                "shape": "rectangle",
                "polygon": [],
                "border": "solid",
                "parent_panel": "",
                "overlaps_panel": "",
                "z_index": 0,
                "geometry_intent": "clear rectangular beat"
            },
            "narrative_role": "peak",
            "semantic_job": "show the visible action",
            "attentional_frame": "mono",
            "narrative_weight": "primary",
            "transition_from_previous": "none",
            "continuity": {
                "breakout": {"enabled": false, "subject_id": "", "edge": "empty", "destination_panel": ""},
                "shared_environment_id": "",
                "subject_phase": ""
            },
            "scene": {
                "description": "A traveler moves through a station",
                "subjects": [{
                    "id": "traveler",
                    "figure": "adult traveler",
                    "pose": "striding with one foot forward",
                    "expression": "focused",
                    "blocking": "centered against the route"
                }],
                "environment": {
                    "setting": "rail station",
                    "foreground": ["platform edge"],
                    "midground": ["traveler"],
                    "background": ["station columns"]
                },
                "camera": {
                    "shot_scale": "full",
                    "angle": "eye_level",
                    "focus": "traveler",
                    "depth_plan": "layered",
                    "eye_flow_exit": "right edge"
                },
                "motion_treatment": "pose_only",
                "lighting": "clear side light",
                "mood": "purposeful",
                "text_in_frame": "none"
            }
        })
    }

    fn frame(
        mut panel: Value,
        shape: &str,
        border: &str,
        parent: &str,
        overlap: &str,
        z_index: i64,
        polygon: Value,
    ) -> Value {
        panel["frame"] = json!({
            "shape": shape,
            "polygon": polygon,
            "border": border,
            "parent_panel": parent,
            "overlaps_panel": overlap,
            "z_index": z_index,
            "geometry_intent": "device-specific narrative geometry"
        });
        panel
    }

    fn continuity(mut panel: Value, environment: &str, phase: &str) -> Value {
        panel["continuity"]["shared_environment_id"] = json!(environment);
        panel["continuity"]["subject_phase"] = json!(phase);
        panel
    }

    fn crossing(mut panel: Value, destination: &str) -> Value {
        panel["continuity"]["breakout"] = json!({
            "enabled": true,
            "subject_id": "traveler",
            "edge": "right",
            "destination_panel": destination
        });
        panel
    }

    fn camera_panel(mut panel: Value, relation: &str, direction: &str) -> Value {
        panel["continuity"]["axis_relation_from_previous"] = json!(relation);
        panel["continuity"]["screen_direction"] = json!(direction);
        panel["continuity"]["eyeline"] = json!({
            "enabled": false,
            "looker_id": "",
            "target_anchor": "",
            "direction": "none"
        });
        panel["continuity"]["match_on_action"] = json!({
            "enabled": false,
            "subject_id": "",
            "action": ""
        });
        panel["scene"]["camera"]["viewpoint"] = json!("objective");
        panel["scene"]["camera"]["viewpoint_subject_id"] = json!("");
        panel["scene"]["camera"]["framing"] = json!("single");
        panel["shot_contract"] = json!({
            "role": "action",
            "shot_scale": "full",
            "viewpoint": "objective",
            "framing": "single",
            "angle": "eye_level",
            "depth_plan": "layered",
            "camera_motivation": "the movement remains legible across the cut",
            "information_gain": format!("{} advances through the station", panel["id"].as_str().expect("invariant: camera fixture panel id must be a string"))
        });
        panel
    }

    fn registered_camera_scene() -> Value {
        let first = camera_panel(
            panel("first", 16, 16, 480, 992),
            "establish",
            "left_to_right",
        );
        let second = camera_panel(
            panel("second", 528, 16, 480, 992),
            "preserve",
            "left_to_right",
        );
        let mut value = scene("none", "", "", "", vec![first, second]);
        value["manga_panel"]["page_design"]["layout"] =
            json!({"template_id": "compression_release"});
        value["manga_panel"]["page_design"]["camera_arc"] = json!({
            "strategy": "action_acceleration",
            "progression": "the movement advances across two compatible views",
            "motivation": "the second phase reveals completion of the same movement",
            "continuity": {
                "axis_mode": "preserve",
                "axis": "traveler to route",
                "screen_direction": "left_to_right",
                "eyeline_policy": "not_applicable"
            }
        });
        value
    }

    fn scene(kind: &str, source: &str, target: &str, subject: &str, panels: Vec<Value>) -> Value {
        let reading_path = panels
            .iter()
            .map(|panel| {
                panel["id"]
                    .as_str()
                    .expect("invariant: fixture panel id must be a string")
            })
            .collect::<Vec<_>>();
        let dominant_panel = reading_path
            .first()
            .expect("invariant: fixture must contain a panel");
        json!({
            "manga_panel": {
                "meta": {"spec_version": "2.0.0"},
                "panel_layout": {"special_device_budget": 1},
                "semantic_spine": {
                    "literal_event": "A traveler moves through a station",
                    "semantic_focus": "movement",
                    "emotional_relation": "purposeful",
                    "intensity": 3,
                    "visual_relation": "approach",
                    "memory_hook": "one forward step aligned with the platform",
                    "metaphor": {"mode": "none", "mapping": "", "literal_anchor": ""}
                },
                "page_design": {
                    "rhythm": "compression_to_release",
                    "archetype": "compression_release",
                    "dominant_panel": dominant_panel,
                    "reading_path": reading_path,
                    "eye_flow_summary": "follow the traveler along the platform",
                    "layout_rendering_directive": "arrange the panels in an immediate reading order",
                    "special_device": {
                        "kind": kind,
                        "reason": "the sentence requires this narrative relation",
                        "source_panel": source,
                        "target_panel": target,
                        "subject_id": subject
                    }
                },
                "panels": panels
            }
        })
    }

    fn diagonal_scene(source: &str, target: &str) -> Value {
        let left = frame(
            panel("tension", 16, 16, 480, 992),
            "trapezoid",
            "solid",
            "",
            "",
            0,
            json!([[16, 16], [496, 80], [448, 1008], [16, 1008]]),
        );
        let right = frame(
            panel("release", 528, 16, 480, 992),
            "trapezoid",
            "solid",
            "",
            "",
            0,
            json!([[576, 16], [1008, 16], [1008, 1008], [528, 944]]),
        );
        scene("diagonal_release", source, target, "", vec![left, right])
    }

    #[test]
    fn inset_scene_passes_dynamic_validation() {
        let parent = panel("establisher", 16, 16, 992, 992);
        let child = frame(
            panel("detail", 640, 96, 280, 280),
            "inset",
            "solid",
            "establisher",
            "",
            1,
            json!([]),
        );
        let scene = scene("inset", "establisher", "detail", "", vec![parent, child]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid inset scene failed dynamic validation"
        );
    }

    #[test]
    fn semantic_payload_cannot_be_omitted_from_dynamic_scene() {
        let mut scene = scene("none", "", "", "", vec![panel("peak", 16, 16, 992, 992)]);
        scene["manga_panel"]["semantic_spine"] = json!({});
        assert!(
            validate_dynamic(&scene).is_err(),
            "dynamic scene without its semantic payload unexpectedly passed validation"
        );
    }

    #[test]
    fn contrast_visual_relation_passes_dynamic_validation() {
        let mut scene = scene("none", "", "", "", vec![panel("peak", 16, 16, 992, 992)]);
        scene["manga_panel"]["semantic_spine"]["visual_relation"] = json!("contrast");
        assert!(
            validate_dynamic(&scene).is_ok(),
            "contrast visual relation failed dynamic validation"
        );
    }

    #[test]
    fn camera_cannot_be_omitted_from_dynamic_panel() {
        let mut scene = scene("none", "", "", "", vec![panel("peak", 16, 16, 992, 992)]);
        scene["manga_panel"]["panels"][0]["scene"]["camera"] = Value::Null;
        assert!(
            validate_dynamic(&scene).is_err(),
            "dynamic panel without camera direction unexpectedly passed validation"
        );
    }

    #[test]
    fn registered_camera_program_accepts_one_coherent_continuity_plan() {
        assert!(
            validate_dynamic(&registered_camera_scene()).is_ok(),
            "coherent registered camera program failed validation"
        );
    }

    #[test]
    fn registered_camera_program_cannot_claim_an_unexecuted_axis_preservation() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][0]["continuity"]["axis_relation_from_previous"] =
            json!("not_applicable");
        scene["manga_panel"]["panels"][1]["continuity"]["axis_relation_from_previous"] =
            json!("not_applicable");
        assert!(
            validate_dynamic(&scene).is_err(),
            "unexecuted scene-level axis preservation unexpectedly passed validation"
        );
    }

    #[test]
    fn registered_camera_program_rejects_screen_direction_drift() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][1]["continuity"]["screen_direction"] =
            json!("right_to_left");
        assert!(
            validate_dynamic(&scene).is_err(),
            "panel screen direction drifted away from the camera plan"
        );
    }

    #[test]
    fn registered_camera_program_cannot_claim_unused_matched_eyelines() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["page_design"]["camera_arc"]["continuity"]["eyeline_policy"] =
            json!("matched");
        assert!(
            validate_dynamic(&scene).is_err(),
            "matched eyeline policy passed without one executed eyeline"
        );
    }

    #[test]
    fn registered_eyeline_requires_a_visible_looker() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["page_design"]["camera_arc"]["continuity"]["eyeline_policy"] =
            json!("matched");
        scene["manga_panel"]["panels"][0]["continuity"]["eyeline"] = json!({
            "enabled": true,
            "looker_id": "ghost",
            "target_anchor": "traveler",
            "direction": "screen_right"
        });
        assert!(
            validate_dynamic(&scene).is_err(),
            "eyeline with an invisible looker unexpectedly passed validation"
        );
    }

    #[test]
    fn registered_matched_eyelines_reject_one_relationship_flipping_direction() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["page_design"]["camera_arc"]["continuity"]["eyeline_policy"] =
            json!("matched");
        scene["manga_panel"]["panels"][0]["continuity"]["eyeline"] = json!({
            "enabled": true,
            "looker_id": "traveler",
            "target_anchor": "station clock",
            "direction": "screen_right"
        });
        scene["manga_panel"]["panels"][1]["continuity"]["eyeline"] = json!({
            "enabled": true,
            "looker_id": "traveler",
            "target_anchor": "station clock",
            "direction": "screen_left"
        });
        assert!(
            validate_dynamic(&scene).is_err(),
            "one matched eyeline relationship flipped screen direction"
        );
    }

    #[test]
    fn registered_matched_eyelines_normalize_reciprocal_screen_direction() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["page_design"]["camera_arc"]["continuity"]["eyeline_policy"] =
            json!("matched");
        scene["manga_panel"]["panels"][0]["continuity"]["eyeline"] = json!({
            "enabled": true,
            "looker_id": "traveler",
            "target_anchor": "friend",
            "direction": "screen_right"
        });
        scene["manga_panel"]["panels"][1]["scene"]["subjects"][0]["id"] = json!("friend");
        scene["manga_panel"]["panels"][1]["continuity"]["eyeline"] = json!({
            "enabled": true,
            "looker_id": "friend",
            "target_anchor": "traveler",
            "direction": "screen_right"
        });
        normalize_eyelines(&mut scene).expect("matched eyelines must normalize");
        assert_eq!(
            (
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][1]["continuity"]["eyeline"]["direction"].as_str(),
            ),
            (true, Some("screen_left")),
            "reciprocal matched eyelines retained the same screen direction"
        );
    }

    #[test]
    fn unsupported_match_on_action_is_disabled_instead_of_rejecting_the_scene() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][1]["continuity"]["match_on_action"] = json!({
            "enabled": true,
            "subject_id": "traveler",
            "action": "continuing one step"
        });
        let rejected_before = validate_dynamic(&scene).is_err();
        normalize_match_on_action(&mut scene).expect("optional edit continuity must normalize");
        assert_eq!(
            (
                rejected_before,
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][1]["continuity"]["match_on_action"].clone(),
            ),
            (
                true,
                true,
                json!({"enabled": false, "subject_id": "", "action": ""}),
            ),
            "unsupported match-on-action kept rejecting an otherwise valid scene"
        );
    }

    #[test]
    fn supported_match_on_action_survives_normalization() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][0]["continuity"]["shared_environment_id"] = json!("station");
        scene["manga_panel"]["panels"][1]["continuity"]["shared_environment_id"] = json!("station");
        scene["manga_panel"]["panels"][1]["transition_from_previous"] = json!("action_to_action");
        scene["manga_panel"]["panels"][1]["continuity"]["match_on_action"] = json!({
            "enabled": true,
            "subject_id": "traveler",
            "action": "continuing one step"
        });
        normalize_match_on_action(&mut scene).expect("supported edit continuity must normalize");
        assert_eq!(
            (
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][1]["continuity"]["match_on_action"].clone(),
            ),
            (
                true,
                json!({
                    "enabled": true,
                    "subject_id": "traveler",
                    "action": "continuing one step"
                }),
            ),
            "supported match-on-action was erased during normalization"
        );
    }

    #[test]
    fn blank_subject_expression_gets_one_safe_visible_default() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][0]["scene"]["subjects"][0]["expression"] = json!("   ");
        let rejected_before = validate_dynamic(&scene).is_err();
        normalize_subject_expressions(&mut scene)
            .expect("blank visible expressions must normalize");
        assert_eq!(
            (
                rejected_before,
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][0]["scene"]["subjects"][0]["expression"].as_str(),
            ),
            (true, true, Some("scene-consistent visible state")),
            "blank subject expression kept rejecting an otherwise valid scene"
        );
    }

    #[test]
    fn registered_group_framing_materializes_its_missing_supporting_subjects() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][0]["scene"]["camera"]["framing"] = json!("group");
        scene["manga_panel"]["panels"][0]["shot_contract"]["framing"] = json!("group");
        scene["manga_panel"]["panels"][0]["shot_contract"]["visible_anchor"] =
            json!("travelers sharing the station route");
        scene["manga_panel"]["panels"][1]["shot_contract"]["visible_anchor"] =
            json!("the traveler completing the route");
        normalize_camera_subjects(&mut scene).expect("group support must materialize");
        assert_eq!(
            (
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][0]["scene"]["subjects"]
                    .as_array()
                    .map(Vec::len),
            ),
            (true, Some(3)),
            "group framing remained unsupported by its visible subject count"
        );
    }

    #[test]
    fn mono_panel_materializes_one_missing_visible_subject() {
        let mut scene = registered_camera_scene();
        scene["manga_panel"]["panels"][0]["scene"]["subjects"] = json!([]);
        scene["manga_panel"]["panels"][0]["shot_contract"]["visible_anchor"] =
            json!("the traveler crossing the station");
        scene["manga_panel"]["panels"][1]["shot_contract"]["visible_anchor"] =
            json!("the traveler completing the route");
        normalize_camera_subjects(&mut scene).expect("visible subject must materialize");
        assert_eq!(
            (
                validate_dynamic(&scene).is_ok(),
                scene["manga_panel"]["panels"][0]["scene"]["subjects"][0]["id"].as_str(),
            ),
            (true, Some("scene_anchor")),
            "a mono panel remained empty after its visible anchor was available"
        );
    }

    #[test]
    fn composer_payoff_role_normalizes_to_the_canonical_peak() {
        let mut fields = json!({"panels": [{"narrative_role": "payoff"}]})
            .as_object()
            .cloned()
            .expect("fixture fields must be an object");
        normalize_panel_roles(&mut fields).expect("payoff alias must normalize");
        assert_eq!(
            fields["panels"][0]["narrative_role"],
            json!("peak"),
            "the composer payoff alias survived canonicalization"
        );
    }

    #[test]
    fn description_cannot_be_omitted_from_dynamic_panel() {
        let mut scene = scene("none", "", "", "", vec![panel("peak", 16, 16, 992, 992)]);
        scene["manga_panel"]["panels"][0]["scene"]["description"] = Value::Null;
        assert!(
            validate_dynamic(&scene).is_err(),
            "dynamic panel without its visible description unexpectedly passed validation"
        );
    }

    #[test]
    fn macro_environmental_panel_can_omit_subjects() {
        let mut panel = panel("peak", 16, 16, 992, 992);
        panel["attentional_frame"] = json!("macro");
        panel["scene"]["subjects"] = json!([]);
        let scene = scene("none", "", "", "", vec![panel]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "macro environmental panel without subjects failed validation"
        );
    }

    #[test]
    fn mono_panel_cannot_omit_its_subjects() {
        let mut panel = panel("peak", 16, 16, 992, 992);
        panel["scene"]["subjects"] = json!([]);
        let scene = scene("none", "", "", "", vec![panel]);
        assert!(
            validate_dynamic(&scene).is_err(),
            "mono panel without subjects unexpectedly passed validation"
        );
    }

    #[test]
    fn selected_medium_close_shot_scale_passes_dynamic_validation() {
        let mut panel = panel("peak", 16, 16, 992, 992);
        panel["scene"]["camera"]["shot_scale"] = json!("medium_close");
        let scene = scene("none", "", "", "", vec![panel]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "selected medium-close cinematic shot failed dynamic validation"
        );
    }

    #[test]
    fn scene_to_scene_transition_passes_dynamic_validation() {
        let first = panel("departure", 16, 16, 480, 992);
        let mut second = panel("arrival", 528, 16, 480, 992);
        second["transition_from_previous"] = json!("scene_to_scene");
        let scene = scene("none", "", "", "", vec![first, second]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid scene-to-scene transition failed dynamic validation"
        );
    }

    #[test]
    fn amorphic_panel_can_omit_subjects() {
        let mut panel = panel("peak", 16, 16, 992, 992);
        panel["attentional_frame"] = json!("amorphic");
        panel["scene"]["subjects"] = json!([]);
        let scene = scene("none", "", "", "", vec![panel]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "amorphic environmental panel without subjects failed validation"
        );
    }

    #[test]
    fn overlap_scene_passes_dynamic_validation() {
        let back = panel("wide", 16, 16, 650, 720);
        let front = frame(
            panel("impact", 520, 500, 488, 508),
            "rectangle",
            "solid",
            "",
            "wide",
            1,
            json!([]),
        );
        let scene = scene("overlap", "impact", "wide", "", vec![back, front]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid overlap scene failed dynamic validation"
        );
    }

    #[test]
    fn master_view_scene_passes_dynamic_validation() {
        let arrival = continuity(panel("arrival", 16, 16, 992, 470), "stairs", "arrival");
        let climb = continuity(panel("climb", 16, 538, 992, 470), "stairs", "climb");
        let scene = scene(
            "master_view",
            "arrival",
            "climb",
            "traveler",
            vec![arrival, climb],
        );
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid master view scene failed dynamic validation"
        );
    }

    #[test]
    fn crossing_scene_passes_dynamic_validation() {
        let source = crossing(panel("launch", 16, 16, 480, 992), "landing");
        let target = panel("landing", 528, 16, 480, 992);
        let scene = scene(
            "crossing",
            "launch",
            "landing",
            "traveler",
            vec![source, target],
        );
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid crossing scene failed dynamic validation"
        );
    }

    #[test]
    fn diagonal_pair_passes_dynamic_validation() {
        let scene = diagonal_scene("tension", "release");
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid diagonal pair failed dynamic validation"
        );
    }

    #[test]
    fn single_diagonal_panel_passes_dynamic_validation() {
        let release = frame(
            panel("release", 16, 16, 992, 992),
            "trapezoid",
            "solid",
            "",
            "",
            0,
            json!([[16, 16], [1008, 96], [912, 1008], [16, 1008]]),
        );
        let scene = scene("diagonal_release", "release", "", "", vec![release]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid single diagonal panel failed dynamic validation"
        );
    }

    #[test]
    fn diagonal_references_must_name_the_trapezoid_panels() {
        let scene = diagonal_scene("release", "");
        assert!(
            validate_dynamic(&scene).is_err(),
            "diagonal release with incomplete trapezoid references unexpectedly passed validation"
        );
    }

    #[test]
    fn open_frame_scene_passes_dynamic_validation() {
        let setup = panel("setup", 16, 16, 992, 400);
        let release = frame(
            panel("release", 16, 448, 992, 560),
            "open_frame",
            "none",
            "",
            "",
            0,
            json!([]),
        );
        let scene = scene("open_frame", "release", "", "", vec![setup, release]);
        assert!(
            validate_dynamic(&scene).is_ok(),
            "valid open frame scene failed dynamic validation"
        );
    }

    #[test]
    fn open_frame_source_must_name_the_borderless_panel() {
        let setup = panel("setup", 16, 16, 992, 400);
        let release = frame(
            panel("release", 16, 448, 992, 560),
            "open_frame",
            "none",
            "",
            "",
            0,
            json!([]),
        );
        let scene = scene("open_frame", "setup", "", "", vec![setup, release]);
        assert!(
            validate_dynamic(&scene).is_err(),
            "open frame with a contradictory source unexpectedly passed validation"
        );
    }

    #[test]
    fn contradictory_overlap_references_cannot_pass_dynamic_validation() {
        let back = panel("wide", 16, 16, 650, 720);
        let front = frame(
            panel("impact", 520, 500, 488, 508),
            "rectangle",
            "solid",
            "",
            "wide",
            1,
            json!([]),
        );
        let scene = scene("overlap", "wide", "impact", "", vec![back, front]);
        assert!(
            validate_dynamic(&scene).is_err(),
            "contradictory overlap references unexpectedly passed validation"
        );
    }
}
