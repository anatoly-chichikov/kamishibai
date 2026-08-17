//! Deterministic natural-language prompts for the manga image model.

use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::HashSet;

const OPENING: &str = "Create a finished black-and-white manga page in high-contrast Indian ink, with crisp expressive linework and fine screentone shading; keep every visible mark strictly monochrome.";
const DOMINANT_HIERARCHY: &str = "Make unequal panel size express editorial emphasis, letting the dominant region carry the payoff while smaller regions provide only motivated visual progression.";
const BALANCED_HIERARCHY: &str = "Keep panel areas balanced so equal editorial emphasis remains clear throughout the motivated visual progression.";
const CLOSING: &str = "No logos, emblems, icons, symbols, pseudo-writing, glyphs. Badge mounts, plates, displays stay blank. Lights/hardware/contours remain physical; surfaces unlettered; gutters, borders, page-edge margins paper-white.";
const FILLERS: [&str; 4] = [
    "Keep the visual hierarchy immediate: silhouettes read cleanly, spatial layers stay distinct, and every pose supports the sentence rather than decorative spectacle.",
    "Use deliberate negative space and controlled screentone density so the eye follows the intended edit without losing the focal action.",
    "Preserve consistent faces, clothing, props, screen direction, and environment across the sequence.",
    "Let each required panel remain immediately legible as part of one coherent page.",
];

/// Compile one materialized scene into a bounded prose image prompt.
pub(crate) fn compile_image_prompt(scene: &Value) -> Result<String> {
    let panels = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
        .filter(|panels| (1..=4).contains(&panels.len()))
        .ok_or_else(|| anyhow!("image prompt requires one to four materialized panels"))?;
    let template = scene
        .pointer("/manga_panel/panel_layout/active_layout/template_id")
        .and_then(Value::as_str)
        .filter(|template| !template.is_empty())
        .ok_or_else(|| anyhow!("image prompt requires a materialized layout template"))?;
    let prompt = assembled(scene, panels, template, false)?;
    let words = prompt.split_whitespace().count();
    if (150..=250).contains(&words) {
        return Ok(prompt);
    }
    if words < 150 {
        return Err(anyhow!(
            "image prompt word count {words} fell outside the 150 to 250 word contract"
        ));
    }
    let prompt = assembled(scene, panels, template, true)?;
    let words = prompt.split_whitespace().count();
    if !(150..=250).contains(&words) {
        return Err(anyhow!(
            "image prompt word count {words} fell outside the 150 to 250 word contract"
        ));
    }
    Ok(prompt)
}

fn assembled(scene: &Value, panels: &[Value], template: &str, compressed: bool) -> Result<String> {
    let mut parts = vec![
        String::from(OPENING),
        format!(
            "The page has {}.",
            geometry(template)?.trim_end_matches('.')
        ),
        hierarchy(scene, panels, compressed)?,
    ];
    if let Some(device) = device(scene, panels)? {
        parts.push(device);
    }
    for (index, panel) in ordered(scene, panels)?.iter().enumerate() {
        parts.extend(panel_sentences(index, panel, panels.len(), compressed));
    }
    let closing_words = CLOSING.split_whitespace().count();
    for filler in FILLERS {
        if parts
            .iter()
            .map(|part| part.split_whitespace().count())
            .sum::<usize>()
            .saturating_add(closing_words)
            >= 150
        {
            break;
        }
        parts.push(String::from(filler));
    }
    parts.push(String::from(CLOSING));
    Ok(parts.join(" "))
}

fn geometry(template: &str) -> Result<&'static str> {
    let description = match template {
        "splash-1-v1" => Some("one uninterrupted full-page panel inside a continuous outer border"),
        "equal-split-vertical-2-v1" => {
            Some("two equal upright panels side by side, separated by one clean vertical gutter")
        }
        "equal-split-horizontal-2-v1" => {
            Some("two equal wide panels stacked top and bottom across one horizontal gutter")
        }
        "diagonal-split-2-v1" => Some(
            "one broader full-page-height panel on the left and one narrower full-page-height panel on the right, sharing one clean gutter slanting down toward the right",
        ),
        "diagonal-split-2-end-v1" => Some(
            "a narrow full-page-height setup panel on the left and a broader full-page-height payoff panel on the right, divided only by one gutter slanting down toward the left",
        ),
        "dominant-split-2-v1" => Some(
            "one broad full-page-height dominant panel on the left beside a narrow full-page-height reaction rail on the right across one straight vertical gutter",
        ),
        "dominant-split-2-end-v1" => Some(
            "one narrow full-page-height setup panel on the left beside one broad full-page-height payoff panel on the right across a straight vertical gutter, with no horizontal split",
        ),
        "orthogonal-grid-3-v1" | "vertical-triptych-3-v1" => Some(
            "three tall panels side by side, each spanning from the top edge to the bottom edge, separated only by two straight vertical gutters with no horizontal divider",
        ),
        "horizontal-triptych-3-v1" => Some(
            "three wide panels stacked from top to bottom, each spanning the full page width, separated only by two straight horizontal gutters with no vertical divider",
        ),
        "diagonal-strip-3-v1" => Some(
            "three adjacent full-page-height trapezoid panels read left to right, with a visibly broader middle panel between two narrower outer panels; two narrow parallel gutters slant down toward the right with no horizontal divider",
        ),
        "radial-y-3-v1" => Some(
            "three wedge panels meeting around a small paper-bright Y-shaped hub; read the upper-left wedge, then the upper-right wedge, then resolve in a wide but shallower triangular lower wedge",
        ),
        "dominant-rail-3-v1" => Some(
            "two compact support panels stacked on the left, read from top to bottom, before entering one dominant full-page-height panel on the right",
        ),
        "t-top-3-v1" => {
            Some("one broad dominant upper panel above two smaller side-by-side lower panels")
        }
        "t-bottom-3-v1" => {
            Some("two smaller side-by-side upper panels above one broad dominant lower panel")
        }
        "grid-2x2-4-v1" => Some(
            "exactly four balanced rectangular panels in two rows, two panels per row, divided by one straight horizontal gutter and aligned vertical gutters",
        ),
        "staggered-grid-4-v1" => Some(
            "exactly four trapezoid panels in two rows separated by one straight horizontal gutter; each row's internal gutter widens toward the bottom, with the upper gutter centered noticeably farther right than the lower gutter and the lower-right panel the largest payoff region",
        ),
        "blockage-rail-4-v1" => Some(
            "three compact support panels stacked on the left and read from top to bottom before entering one dominant full-page-height panel on the right",
        ),
        "expansion-stack-4-v1" => {
            Some("four full-width tiers that grow progressively taller toward the lower payoff")
        }
        "contraction-stack-4-v1" => {
            Some("four full-width tiers that contract progressively from the broad opening")
        }
        "diagonal-strip-4-v1" => Some(
            "four adjacent narrow full-page-height trapezoid panels with no horizontal dividers; all three narrow parallel gutters slant down toward the right from top to bottom",
        ),
        "radial-cross-4-v1" => Some(
            "four wedge panels orbiting a clean paper-bright central hub; read clockwise from the top wedge through the right and bottom wedges to the left wedge",
        ),
        "horizontal-strip-4-v1" => {
            Some("four equal tall panels arranged as a clean left-to-right strip")
        }
        "vertical-strip-4-v1" => {
            Some("four equal full-width panels stacked in a clean top-to-bottom strip")
        }
        "dominant-rail-3-rtl-v1" => Some(
            "two compact support panels stacked on the right, read from top to bottom, before entering one dominant full-page-height panel on the left",
        ),
        "blockage-rail-4-rtl-v1" => Some(
            "three compact support panels stacked on the right and read from top to bottom before entering one dominant full-page-height panel on the left",
        ),
        "parallel-tracks-4-v1" => Some(
            "four panels forming two vertical tracks with two stacked panels on each side; read down the left pair first and then down the right pair, while a wider central gutter keeps the tracks grouped",
        ),
        "inset-dominant-2-v1" => Some(
            "one dominant full-page panel read first, followed by one clearly nested detail inset in its upper-right area",
        ),
        "fan-3-v1" => Some(
            "three left-to-right wedge panels converging near one shared lower hub, with a narrow central wedge between two much broader outer wedges",
        ),
        "slanted-t-bottom-3-p2-v1" => Some(
            "one visibly smaller upper-left trapezoid and one visibly larger upper-right trapezoid divided by a gutter slanting down toward the left, above one broad dominant lower panel spanning the full page width across a straight horizontal gutter",
        ),
        "slanted-dominant-rail-3-p2-v1" => Some(
            "one narrow left rail containing a shorter upper support panel and a taller lower support panel, read top to bottom across a gutter slanting down toward the right, before entering one broad dominant full-page-height rectangular panel on the right across a straight vertical gutter",
        ),
        "diagonal-split-2-end-strong-v1" => Some(
            "one compressed full-page-height setup panel occupying about one third on the left and one dominant full-page-height payoff panel occupying about two thirds on the right, divided only by one gutter slanting down toward the left",
        ),
        _ => None,
    };
    description.ok_or_else(|| anyhow!("image prompt has unknown layout template '{template}'"))
}

fn hierarchy(scene: &Value, panels: &[Value], compressed: bool) -> Result<String> {
    let emphasis = emphasis(scene, panels)?;
    let Some(arc) = scene.pointer("/manga_panel/page_design/camera_arc") else {
        return Ok(String::from(match emphasis {
            Emphasis::Single => "Let the uninterrupted page carry one focused editorial beat.",
            Emphasis::Balanced => BALANCED_HIERARCHY,
            Emphasis::DominantLargest => DOMINANT_HIERARCHY,
            Emphasis::DominantSmaller => {
                "Preserve unequal panel areas while the smaller payoff gains editorial emphasis through its camera and action."
            }
            Emphasis::UnequalBalanced => {
                "Preserve unequal panel areas without inventing a dominant payoff; keep editorial weight balanced through camera and action."
            }
        }));
    };
    let progression = sanitized(
        arc.get("progression")
            .and_then(Value::as_str)
            .unwrap_or("motivated visual"),
        if compressed { 3 } else { 6 },
    );
    let emphasis = match emphasis {
        Emphasis::Single => "Keep one uninterrupted editorial focus",
        Emphasis::Balanced => "Keep panel areas balanced for equal editorial emphasis",
        Emphasis::DominantLargest => "Use the largest panel for editorial emphasis",
        Emphasis::DominantSmaller => {
            "Preserve unequal areas and emphasize the smaller payoff through its image"
        }
        Emphasis::UnequalBalanced => {
            "Preserve unequal areas while balancing editorial weight through each image"
        }
    };
    Ok(format!(
        "{emphasis} and a {progression} camera progression; motivate each cut by visible change."
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Emphasis {
    Single,
    Balanced,
    DominantLargest,
    DominantSmaller,
    UnequalBalanced,
}

fn emphasis(scene: &Value, panels: &[Value]) -> Result<Emphasis> {
    if panels.len() == 1 {
        return Ok(Emphasis::Single);
    }
    let areas = panels
        .iter()
        .map(panel_area)
        .collect::<Option<Vec<_>>>()
        .filter(|areas| areas.iter().all(|area| *area > 0))
        .ok_or_else(|| anyhow!("image prompt requires positive materialized panel geometry"))?;
    let dominant = scene
        .pointer("/manga_panel/page_design/dominant_panel")
        .and_then(Value::as_str)
        .filter(|panel| !panel.is_empty());
    if let Some(dominant) = dominant {
        let area = panels
            .iter()
            .position(|panel| panel.get("id").and_then(Value::as_str) == Some(dominant))
            .and_then(|index| areas.get(index))
            .copied()
            .ok_or_else(|| anyhow!("image prompt dominant panel is not materialized"))?;
        let maximum = areas
            .iter()
            .copied()
            .max()
            .ok_or_else(|| anyhow!("image prompt has no materialized panel area"))?;
        if area == maximum {
            return Ok(Emphasis::DominantLargest);
        }
        return Ok(Emphasis::DominantSmaller);
    }
    let minimum = areas
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("image prompt has no materialized panel area"))?;
    let maximum = areas
        .iter()
        .copied()
        .max()
        .ok_or_else(|| anyhow!("image prompt has no materialized panel area"))?;
    if maximum.saturating_sub(minimum) <= maximum / 10 {
        return Ok(Emphasis::Balanced);
    }
    Ok(Emphasis::UnequalBalanced)
}

fn panel_area(panel: &Value) -> Option<u128> {
    let polygon = panel.pointer("/frame/polygon").and_then(Value::as_array);
    if let Some(polygon) = polygon.filter(|points| points.len() >= 3) {
        let mut sum = 0i128;
        for index in 0..polygon.len() {
            let next = index.checked_add(1)?.checked_rem(polygon.len())?;
            let first = polygon.get(index)?.as_array()?;
            let second = polygon.get(next)?.as_array()?;
            let left = i128::from(first.first()?.as_i64()?);
            let top = i128::from(first.get(1)?.as_i64()?);
            let right = i128::from(second.first()?.as_i64()?);
            let bottom = i128::from(second.get(1)?.as_i64()?);
            let forward = left.checked_mul(bottom)?;
            let backward = right.checked_mul(top)?;
            sum = sum.checked_add(forward.checked_sub(backward)?)?;
        }
        return Some(sum.unsigned_abs());
    }
    let width = panel.pointer("/bounds/width")?.as_u64()?;
    let height = panel.pointer("/bounds/height")?.as_u64()?;
    u128::from(width)
        .checked_mul(u128::from(height))?
        .checked_mul(2)
}

fn ordered<'a>(scene: &'a Value, panels: &'a [Value]) -> Result<Vec<&'a Value>> {
    let Some(path) = scene
        .pointer("/manga_panel/page_design/reading_path")
        .and_then(Value::as_array)
    else {
        return Ok(panels.iter().collect());
    };
    if path.len() != panels.len() {
        return Err(anyhow!(
            "image prompt reading path does not cover every materialized panel"
        ));
    }
    let mut ids = HashSet::new();
    let mut ordered = Vec::with_capacity(path.len());
    for item in path {
        let id = item
            .as_str()
            .filter(|id| ids.insert(*id))
            .ok_or_else(|| anyhow!("image prompt reading path repeats or omits a panel"))?;
        let panel = panels
            .iter()
            .find(|panel| panel.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| anyhow!("image prompt reading path names an unknown panel"))?;
        ordered.push(panel);
    }
    Ok(ordered)
}

fn device(scene: &Value, panels: &[Value]) -> Result<Option<String>> {
    let root = scene
        .pointer("/manga_panel/page_design/special_device")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("image prompt requires a materialized special device"))?;
    let kind = root
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| !kind.is_empty())
        .ok_or_else(|| anyhow!("image prompt requires a materialized special device kind"))?;
    let prose = match kind {
        "none" => None,
        "crossing" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            let target = device_panel(root, "target_panel", scene, panels)?;
            let subject =
                device_subject(root, panels)?.unwrap_or_else(|| "One recurring subject".to_owned());
            Some(match (source, target) {
                (Some(source), Some(target)) => format!(
                    "{subject} alone crosses from the {source} panel into the {target} panel; every other figure stays contained."
                ),
                _ => format!(
                    "{subject} alone crosses one adjacent gutter while every other figure stays contained."
                ),
            })
        }
        "overlap" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            let target = device_panel(root, "target_panel", scene, panels)?;
            Some(match (source, target) {
                (Some(source), Some(target)) => format!(
                    "The {source} panel overlaps the {target} panel as foreground across one gutter; every other edge stays distinct."
                ),
                _ => "One foreground panel overlaps one adjacent panel across a single gutter; every other edge stays distinct.".to_owned(),
            })
        }
        "open_frame" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            Some(match source {
                Some(source) => format!(
                    "Open only the {source} panel border into the page; keep every other divider and the outer border intact."
                ),
                None => "Open only one panel border into the page; keep every other divider and the outer border intact.".to_owned(),
            })
        }
        "inset" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            let target = device_panel(root, "target_panel", scene, panels)?;
            Some(match (source, target) {
                (Some(source), Some(target)) => format!(
                    "Nest the {target} detail panel visibly inside the {source} parent panel."
                ),
                _ => "Nest one compact detail panel visibly inside one larger parent panel."
                    .to_owned(),
            })
        }
        "master_view" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            let target = device_panel(root, "target_panel", scene, panels)?;
            let subject =
                device_subject(root, panels)?.unwrap_or_else(|| "one recurring subject".to_owned());
            Some(match (source, target) {
                (Some(source), Some(target)) => format!(
                    "Carry {subject} through grounded phases from the {source} to the {target} panel within one continuous environment; keep every boundary visible."
                ),
                _ => format!(
                    "Carry {subject} through grounded phases across one continuous environment while keeping every panel boundary visible."
                ),
            })
        }
        "diagonal_release" => {
            let source = device_panel(root, "source_panel", scene, panels)?;
            let target = device_panel(root, "target_panel", scene, panels)?;
            Some(match (source, target) {
                (Some(source), Some(target)) => format!(
                    "Carry one supported directional release along the canonical slant from the {source} toward the {target} panel without changing dividers."
                ),
                _ => "Carry one supported directional release along the canonical slant across adjacent panels without changing dividers.".to_owned(),
            })
        }
        _ => return Err(anyhow!("image prompt has unknown special device '{kind}'")),
    };
    Ok(prose)
}

fn device_panel(
    device: &serde_json::Map<String, Value>,
    field: &str,
    scene: &Value,
    panels: &[Value],
) -> Result<Option<&'static str>> {
    let Some(id) = device
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let index = ordered(scene, panels)?
        .iter()
        .position(|panel| panel.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| anyhow!("image prompt special device names an unknown panel"))?;
    ["first", "second", "third", "fourth"]
        .get(index)
        .copied()
        .map(Some)
        .ok_or_else(|| anyhow!("image prompt special device leaves the reading path"))
}

fn device_subject(
    device: &serde_json::Map<String, Value>,
    panels: &[Value],
) -> Result<Option<String>> {
    let Some(subject) = device
        .get("subject_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let source = device
        .get("source_panel")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("image prompt special device subject lacks a source panel"))?;
    let figure = panels
        .iter()
        .find(|panel| panel.get("id").and_then(Value::as_str) == Some(source))
        .and_then(|panel| panel.pointer("/scene/subjects").and_then(Value::as_array))
        .and_then(|subjects| {
            subjects
                .iter()
                .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(subject))
        })
        .and_then(|subject| subject.get("figure").and_then(Value::as_str))
        .filter(|figure| !figure.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("image prompt special device subject is not visibly materialized")
        })?;
    Ok(Some(narrative(figure, 5)))
}

fn panel_sentences(
    index: usize,
    panel: &Value,
    panel_count: usize,
    compressed: bool,
) -> [String; 3] {
    let ordinal = ["first", "second", "third", "fourth"]
        .get(index)
        .copied()
        .unwrap_or("next");
    let scene = panel.get("scene").unwrap_or(panel);
    let (description_limit, motivation_limit, light_limit) = panel_limits(panel_count, compressed);
    let description = description(scene, description_limit);
    let camera = scene.get("camera").unwrap_or(&Value::Null);
    let scale = enum_words(camera, "shot_scale", "medium", 3);
    let viewpoint = viewpoint_words(camera);
    let framing = framing_words(camera);
    let angle = angle_words(camera);
    let depth = depth_words(camera, 3);
    let light = narrative(
        scene
            .get("lighting")
            .or_else(|| scene.get("mood"))
            .and_then(Value::as_str)
            .unwrap_or("controlled high-value contrast"),
        light_limit,
    );
    let motivation = narrative(
        panel
            .get("semantic_job")
            .or_else(|| panel.pointer("/shot_contract/camera_motivation"))
            .and_then(Value::as_str)
            .unwrap_or("the visible narrative change"),
        motivation_limit,
    );
    [
        format!("The {ordinal} panel: {description}."),
        format!("Use {scale} {viewpoint} {framing} framing from {angle}, with {depth}."),
        format!("Purpose: {motivation}; illumination: {light}."),
    ]
}

fn panel_limits(panels: usize, compressed: bool) -> (usize, usize, usize) {
    if compressed {
        return (5, 3, 2);
    }
    match panels {
        1 => (24, 14, 6),
        2 => (18, 14, 4),
        3 => (14, 14, 3),
        _ => (8, 8, 3),
    }
}

fn viewpoint_words(value: &Value) -> String {
    match value
        .get("viewpoint")
        .and_then(Value::as_str)
        .unwrap_or("objective")
    {
        "over_the_shoulder" => String::from("over-the-shoulder"),
        "point_of_view" => String::from("point-of-view"),
        other => sanitized(other, 2),
    }
}

fn framing_words(value: &Value) -> String {
    match value
        .get("framing")
        .and_then(Value::as_str)
        .unwrap_or("single")
    {
        "two_shot" => String::from("two-shot"),
        other => sanitized(other, 2),
    }
}

fn angle_words(value: &Value) -> String {
    match value
        .get("angle")
        .and_then(Value::as_str)
        .unwrap_or("eye_level")
    {
        "eye_level" => String::from("eye level"),
        "high" => String::from("a high angle"),
        "low" => String::from("a low angle"),
        "overhead" => String::from("overhead"),
        "dutch" => String::from("a Dutch angle"),
        other => sanitized(other, 3),
    }
}

fn description(scene: &Value, limit: usize) -> String {
    if let Some(description) = scene
        .get("description")
        .and_then(Value::as_str)
        .filter(|description| !description.trim().is_empty())
    {
        return narrative(description, limit);
    }
    let subject = scene
        .get("subjects")
        .and_then(Value::as_array)
        .and_then(|subjects| subjects.first());
    let visible = [
        subject
            .and_then(|item| item.get("figure"))
            .and_then(Value::as_str),
        subject
            .and_then(|item| item.get("pose"))
            .and_then(Value::as_str),
        subject
            .and_then(|item| item.get("expression"))
            .and_then(Value::as_str),
        scene
            .pointer("/environment/setting")
            .and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    narrative(visible.as_str(), limit)
}

fn enum_words(value: &Value, field: &str, fallback: &str, limit: usize) -> String {
    sanitized(
        value.get(field).and_then(Value::as_str).unwrap_or(fallback),
        limit,
    )
}

fn depth_words(value: &Value, limit: usize) -> String {
    match value
        .get("depth_plan")
        .and_then(Value::as_str)
        .unwrap_or("layered")
    {
        "deep" => String::from("deep focus"),
        "shallow" => String::from("shallow focus"),
        "layered" => String::from("layered depth"),
        "flat" => String::from("flat staging"),
        other => sanitized(other, limit),
    }
}

fn sanitized(value: &str, limit: usize) -> String {
    bounded(value, limit, false)
}

fn narrative(value: &str, limit: usize) -> String {
    bounded(value, limit, true)
}

fn bounded(value: &str, limit: usize, strict: bool) -> String {
    let words = tokens(value, strict);
    if words.len() <= limit {
        return finished(words);
    }
    if strict {
        let mut complete = Vec::new();
        for clause in value.split_inclusive(['.', '!', '?', ';']) {
            let words = tokens(clause, strict);
            if words.is_empty() || complete.len().saturating_add(words.len()) > limit {
                break;
            }
            complete.extend(words);
        }
        if !complete.is_empty() {
            return finished(complete);
        }
    }
    finished(words.into_iter().take(limit).collect())
}

fn tokens(value: &str, strict: bool) -> Vec<String> {
    let mut words = Vec::new();
    for source in value.split_whitespace() {
        let bare = source.trim_matches(|character: char| {
            !character.is_alphanumeric() && character != '-' && character != '_'
        });
        if bare.is_empty()
            || bare.chars().any(|character| character.is_ascii_digit())
            || bare.contains(['/', '\\', '#', '='])
        {
            continue;
        }
        if strict && (bare.contains('_') || schema_word(bare)) {
            continue;
        }
        for component in bare.split('_').filter(|component| !component.is_empty()) {
            words.push(normalized(component));
        }
    }
    words
}

fn finished(mut words: Vec<String>) -> String {
    while words.last().is_some_and(|word| {
        matches!(
            word.to_lowercase().as_str(),
            "a" | "an"
                | "and"
                | "against"
                | "as"
                | "at"
                | "because"
                | "behind"
                | "below"
                | "beside"
                | "by"
                | "for"
                | "from"
                | "in"
                | "into"
                | "near"
                | "of"
                | "on"
                | "or"
                | "over"
                | "than"
                | "that"
                | "the"
                | "through"
                | "to"
                | "toward"
                | "under"
                | "where"
                | "while"
                | "which"
                | "who"
                | "whose"
                | "with"
        )
    }) {
        words.pop();
    }
    if words.is_empty() {
        return String::from("controlled tonal contrast");
    }
    words.join(" ")
}

fn schema_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "x" | "y"
            | "width"
            | "height"
            | "bounds"
            | "coordinate"
            | "coordinates"
            | "polygon"
            | "z-index"
            | "subject-id"
            | "shot-id"
            | "panel-id"
    )
}

fn normalized(word: &str) -> String {
    word.split('-')
        .map(|part| tonal(part.to_lowercase().as_str()).unwrap_or(part))
        .collect::<Vec<_>>()
        .join("-")
}

fn tonal(word: &str) -> Option<&'static str> {
    match word {
        "warm" | "gold" | "golden" | "amber" | "orange" | "red" | "yellow" | "blond" | "blonde"
        | "pink" | "rose" | "scarlet" | "crimson" | "copper" | "bronze" | "ochre" => {
            Some("bright-value")
        }
        "cool" | "blue" | "green" | "purple" | "violet" | "cyan" | "magenta" | "teal"
        | "turquoise" => Some("restrained-value"),
        "black" | "dark" | "brown" | "navy" | "indigo" | "maroon" | "burgundy" => {
            Some("deep-shadow")
        }
        "white" | "pale" | "cream" | "beige" | "tan" => Some("bright-highlight"),
        "gray" | "grey" | "silver" | "sepia" => Some("neutral-midtone"),
        "color" | "colour" | "colored" | "coloured" | "multicolor" | "multicolour" => Some("tonal"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::{CLOSING, Emphasis, compile_image_prompt, emphasis};

    fn production_scene() -> Value {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/topology-production/crossing-exact.scene.json"
        ))
        .expect("production scene fixture must parse")
    }

    fn minimal_scene(template: &str, count: usize) -> Value {
        let panels = (0..count)
            .map(|index| {
                json!({
                    "id": format!("p{}", index + 1),
                    "bounds": {"width": 100, "height": 100},
                    "scene": {
                        "description": "A blacksmith carries goldfish past greenhouse and brownie",
                        "camera": {
                            "shot_scale": "medium_close",
                            "viewpoint": "objective",
                            "angle": "eye_level",
                            "depth_plan": "layered"
                        },
                        "lighting": "warm golden light against a cool blue background"
                    }
                })
            })
            .collect::<Vec<_>>();
        json!({
            "manga_panel": {
                "panel_layout": {"active_layout": {"template_id": template}},
                "page_design": {"special_device": {"kind": "none"}},
                "panels": panels
            }
        })
    }

    fn registry_scene(template: &str) -> Value {
        let registry =
            serde_json::from_str::<Value>(include_str!("../../../assets/layout_registry_v2.json"))
                .expect("layout registry must parse");
        let template = registry["templates"]
            .as_array()
            .expect("layout templates must be an array")
            .iter()
            .find(|candidate| candidate["template_id"].as_str() == Some(template))
            .expect("requested layout template must exist");
        let panels = template["panels"]
            .as_array()
            .expect("layout panels must be an array")
            .iter()
            .enumerate()
            .map(|(index, panel)| {
                json!({
                    "id": format!("p{}", index + 1),
                    "bounds": panel["bounds"],
                    "frame": {"polygon": panel["polygon"]},
                    "scene": {
                        "description": "One grounded subject performs a visible action",
                        "camera": {
                            "shot_scale": "medium",
                            "viewpoint": "objective",
                            "framing": "single",
                            "angle": "eye_level",
                            "depth_plan": "layered"
                        },
                        "lighting": "controlled tonal contrast"
                    }
                })
            })
            .collect::<Vec<_>>();
        let dominant = template["dominant_index"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .map(|index| format!("p{}", index + 1))
            .unwrap_or_default();
        json!({
            "manga_panel": {
                "panel_layout": {
                    "active_layout": {"template_id": template["template_id"]}
                },
                "page_design": {
                    "dominant_panel": dominant,
                    "special_device": {"kind": "none"}
                },
                "panels": panels
            }
        })
    }

    /// Production prose stays bounded and excludes provider-facing structure.
    #[test]
    fn production_prompt_is_bounded_narrative_without_schema_tokens() {
        let prompt = compile_image_prompt(&production_scene())
            .expect("production prompt must compile from a valid scene");
        let words = prompt.split_whitespace().count();
        assert_eq!(
            (
                (150..=250).contains(&words),
                prompt.starts_with("Create a finished black-and-white manga page"),
                prompt.contains("gutter slanting"),
                prompt.ends_with(CLOSING),
                ["\"x\"", "\"y\"", "z_index", "p1", "woman_ze"]
                    .iter()
                    .all(|token| !prompt.contains(token)),
                prompt.contains("wide objective environment"),
                prompt.contains("physical struggle against the heavy wind"),
                prompt.chars().all(|character| !character.is_ascii_digit()),
            ),
            (true, true, true, true, true, true, true, true),
            "production prose leaked structure or lost cinematic and semantic information"
        );
    }

    /// The production fixture freezes the exact deterministic provider prompt.
    #[test]
    fn production_prompt_has_one_frozen_digest() {
        let prompt = compile_image_prompt(&production_scene())
            .expect("production prompt must compile from a valid scene");
        let digest = Sha256::digest(prompt.as_bytes()).iter().fold(
            String::with_capacity(64),
            |mut value, byte| {
                use std::fmt::Write as _;
                write!(&mut value, "{byte:02x}")
                    .expect("invariant: writing hexadecimal bytes to a string cannot fail");
                value
            },
        );
        assert_eq!(
            digest, "4aa8a2f9c25ccec6a861f610fda6191775c15fef6075ce4ea64a877ece7abd83",
            "production image prose changed without an explicit revision review"
        );
    }

    #[test]
    fn production_prompt_closes_the_vehicle_symbol_boundary_exactly() {
        let prompt = compile_image_prompt(&production_scene())
            .expect("production prompt must compile from a valid scene");
        assert_eq!(
            (
                prompt.ends_with(
                    "No logos, emblems, icons, symbols, pseudo-writing, glyphs. Badge mounts, plates, displays stay blank. Lights/hardware/contours remain physical; surfaces unlettered; gutters, borders, page-edge margins paper-white.",
                ),
                (150..=250).contains(&prompt.split_whitespace().count()),
            ),
            (true, true),
            "production image prompt lost its exact vehicle symbol prohibition or word budget"
        );
    }

    /// Scene color language becomes monochrome value language deterministically.
    #[test]
    fn color_language_is_normalized_into_tonal_values() {
        let scene = minimal_scene("splash-1-v1", 1);
        let first = compile_image_prompt(&scene).expect("first prompt must compile");
        let second = compile_image_prompt(&scene).expect("second prompt must compile");
        let lowercase = first.to_lowercase();
        assert_eq!(
            (
                first == second,
                ["warm", "golden", "cool", "blue"]
                    .iter()
                    .all(|word| !lowercase.contains(word)),
                lowercase.contains("value"),
                lowercase.contains(
                    "medium close objective single framing from eye level, with layered depth",
                ),
                ["blacksmith", "goldfish", "greenhouse", "brownie"]
                    .iter()
                    .all(|word| lowercase.contains(word)),
            ),
            (true, true, true, true, true),
            "color normalization became nondeterministic or leaked chromatic language"
        );
    }

    /// Hostile scene prose cannot expose construction vocabulary or color names.
    #[test]
    fn hostile_scene_prose_cannot_leak_schema_ids_or_color_vocabulary() {
        let mut scene = minimal_scene("splash-1-v1", 1);
        scene["manga_panel"]["panels"][0]["scene"]["description"] = json!(
            "x 16 y 16 width 774 height 992 bounds polygon z_index subject_id woman_ze sepia crimson"
        );
        scene["manga_panel"]["panels"][0]["scene"]["lighting"] =
            json!("pink rose copper bronze ochre navy indigo maroon burgundy light");
        let prompt = compile_image_prompt(&scene).expect("hostile prompt must compile safely");
        let words = prompt
            .split(|character: char| !character.is_alphanumeric() && character != '-')
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        assert_eq!(
            (
                [
                    "x", "y", "width", "height", "bounds", "polygon", "z", "index", "subject",
                    "id", "woman", "ze", "sepia", "crimson", "pink", "rose", "copper", "bronze",
                    "ochre", "navy", "indigo", "maroon", "burgundy",
                ]
                .iter()
                .all(|forbidden| !words.iter().any(|word| word == forbidden)),
                prompt.contains("value") || prompt.contains("shadow"),
            ),
            (true, true),
            "hostile prose leaked schema, identifier, or chromatic vocabulary"
        );
    }

    /// Every production registry template has a prose geometry description.
    #[test]
    fn every_registry_template_compiles_without_fallback_geometry() {
        let registry =
            serde_json::from_str::<Value>(include_str!("../../../assets/layout_registry_v2.json"))
                .expect("layout registry must parse");
        let compiled = registry["templates"]
            .as_array()
            .expect("layout templates must be an array")
            .iter()
            .map(|template| {
                let id = template["template_id"]
                    .as_str()
                    .expect("template id must be a string");
                let count = template["panels"]
                    .as_array()
                    .expect("template panels must be an array")
                    .len();
                compile_image_prompt(&minimal_scene(id, count))
                    .is_ok_and(|prompt| (150..=250).contains(&prompt.split_whitespace().count()))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            compiled,
            vec![true; 32],
            "a production layout lost its natural-language geometry"
        );
    }

    /// Every production device has explicit prose or an explicit no-op.
    #[test]
    fn every_registry_device_has_explicit_prompt_behavior() {
        let registry =
            serde_json::from_str::<Value>(include_str!("../../../assets/device_registry_v3.json"))
                .expect("device registry must parse");
        let compiled = registry["devices"]
            .as_array()
            .expect("device registry must contain devices")
            .iter()
            .map(|device| {
                let kind = device["scene_kind"]
                    .as_str()
                    .expect("device kind must be a string");
                let mut scene = minimal_scene("equal-split-vertical-2-v1", 2);
                let source = if kind == "none" { "" } else { "p1" };
                let target = if matches!(kind, "none" | "open_frame") {
                    ""
                } else {
                    "p2"
                };
                let subject = if matches!(kind, "crossing" | "master_view") {
                    "actor"
                } else {
                    ""
                };
                scene["manga_panel"]["page_design"]["special_device"] = json!({
                    "kind": kind,
                    "source_panel": source,
                    "target_panel": target,
                    "subject_id": subject
                });
                scene["manga_panel"]["panels"][0]["scene"]["subjects"] = json!([{
                    "id": "actor",
                    "figure": "the same courier in a deep-shadow coat"
                }]);
                compile_image_prompt(&scene).is_ok()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            compiled,
            vec![true; 7],
            "a production device silently lost its image-prompt behavior"
        );
    }

    /// Unknown named templates cannot silently degrade to generic geometry.
    #[test]
    fn unknown_named_template_cannot_use_generic_geometry() {
        let result = compile_image_prompt(&minimal_scene("unknown-production-layout", 2));
        assert!(
            result.is_err(),
            "an unknown named layout silently entered the image provider prompt"
        );
    }

    /// Missing materialized templates cannot silently use generic geometry.
    #[test]
    fn missing_template_cannot_use_generic_geometry() {
        let mut scene = minimal_scene("splash-1-v1", 1);
        scene["manga_panel"]["panel_layout"]["active_layout"] = json!({});
        assert!(
            compile_image_prompt(&scene).is_err(),
            "a missing materialized layout silently entered the image provider prompt"
        );
    }

    /// Unknown non-empty devices cannot silently disappear at the provider boundary.
    #[test]
    fn unknown_named_device_cannot_disappear_from_the_prompt() {
        let mut scene = minimal_scene("equal-split-vertical-2-v1", 2);
        scene["manga_panel"]["page_design"]["special_device"]["kind"] =
            json!("unknown-production-device");
        assert!(
            compile_image_prompt(&scene).is_err(),
            "an unknown named device silently entered the image provider prompt"
        );
    }

    /// Materialized device relations become ordinal prose without leaking identifiers.
    #[test]
    fn special_device_relations_keep_panels_and_visible_subjects() {
        let mut scene = minimal_scene("equal-split-vertical-2-v1", 2);
        scene["manga_panel"]["page_design"]["reading_path"] = json!(["p2", "p1"]);
        scene["manga_panel"]["page_design"]["special_device"] = json!({
            "kind": "crossing",
            "source_panel": "p1",
            "target_panel": "p2",
            "subject_id": "courier_ze"
        });
        scene["manga_panel"]["panels"][0]["scene"]["subjects"] = json!([{
            "id": "courier_ze",
            "figure": "Golden-jacketed courier"
        }]);
        let prompt = compile_image_prompt(&scene).expect("materialized crossing must compile");
        assert_eq!(
            (
                prompt.contains(
                    "bright-value-jacketed courier alone crosses from the second panel into the first panel"
                ),
                ["p1", "p2", "courier_ze", "golden"]
                    .iter()
                    .all(|value| !prompt.contains(value)),
            ),
            (true, true),
            "special-device prose lost its materialized relation or leaked identifiers"
        );
    }

    /// An explicit subject reference cannot degrade into a generic crossing subject.
    #[test]
    fn unknown_special_device_subject_cannot_enter_the_provider_prompt() {
        let mut scene = minimal_scene("equal-split-vertical-2-v1", 2);
        scene["manga_panel"]["page_design"]["special_device"] = json!({
            "kind": "crossing",
            "source_panel": "p1",
            "target_panel": "p2",
            "subject_id": "missing_actor"
        });
        assert!(
            compile_image_prompt(&scene).is_err(),
            "an unknown explicit device subject silently entered the image prompt"
        );
    }

    /// Equal layouts cannot receive contradictory dominant-region instructions.
    #[test]
    fn equal_layout_prompt_preserves_balanced_editorial_emphasis() {
        let prompt = compile_image_prompt(&minimal_scene("equal-split-vertical-2-v1", 2))
            .expect("equal split prompt must compile");
        assert_eq!(
            (
                prompt.contains("equal editorial emphasis"),
                prompt.contains("dominant region"),
            ),
            (true, false),
            "equal geometry received contradictory dominant-region prose"
        );
    }

    /// Materialized polygon areas keep hierarchy prose truthful for irregular layouts.
    #[test]
    fn polygon_areas_distinguish_single_balanced_and_compact_payoffs() {
        let cases = [
            ("splash-1-v1", Emphasis::Single),
            ("equal-split-vertical-2-v1", Emphasis::Balanced),
            ("grid-2x2-4-v1", Emphasis::Balanced),
            ("diagonal-strip-3-v1", Emphasis::DominantSmaller),
            ("radial-y-3-v1", Emphasis::DominantSmaller),
            ("fan-3-v1", Emphasis::UnequalBalanced),
            ("diagonal-strip-4-v1", Emphasis::DominantLargest),
        ];
        let actual = cases.map(|(template, _)| {
            let scene = registry_scene(template);
            let panels = scene["manga_panel"]["panels"]
                .as_array()
                .expect("materialized panels must be an array");
            emphasis(&scene, panels)
                .expect("canonical polygon areas must produce truthful emphasis")
        });
        assert_eq!(
            actual,
            cases.map(|(_, expected)| expected),
            "polygon-derived hierarchy contradicted canonical panel emphasis"
        );
    }

    /// Reading order and motivated camera information survive prose compilation.
    #[test]
    fn prompt_uses_reading_path_and_motivated_camera_progression() {
        let mut scene = production_scene();
        scene["manga_panel"]["page_design"]["reading_path"] = json!(["p3", "p1", "p2"]);
        let prompt = compile_image_prompt(&scene).expect("reordered prompt must compile");
        let close = prompt
            .find("bright-value close-up")
            .expect("payoff description must survive");
        let beach = prompt
            .find("expansive wide shot")
            .expect("establishing description must survive");
        assert_eq!(
            (
                close < beach,
                prompt.contains("wide to medium to close camera progression"),
                prompt.contains("Purpose: Focuses on her facial expression"),
            ),
            (true, true, true),
            "prose compilation lost reading order or motivated camera progression"
        );
    }

    /// Duplicate reading-path entries cannot silently omit one materialized panel.
    #[test]
    fn duplicate_reading_path_cannot_enter_the_provider_prompt() {
        let mut scene = production_scene();
        scene["manga_panel"]["page_design"]["reading_path"] = json!(["p1", "p1", "p3"]);
        assert!(
            compile_image_prompt(&scene).is_err(),
            "a duplicate reading path silently omitted one materialized panel"
        );
    }

    /// Missing materialized panel geometry cannot create fictional hierarchy prose.
    #[test]
    fn missing_panel_geometry_cannot_create_fictional_hierarchy() {
        let mut scene = minimal_scene("equal-split-vertical-2-v1", 2);
        scene["manga_panel"]["panels"][1]
            .as_object_mut()
            .expect("test panel must be an object")
            .remove("bounds");
        assert!(
            compile_image_prompt(&scene).is_err(),
            "missing panel geometry silently invented editorial hierarchy"
        );
    }

    /// Bounded scene clauses remain grammatical after deterministic truncation.
    #[test]
    fn prompt_cannot_leave_trailing_connector_fragments() {
        let prompt = compile_image_prompt(&minimal_scene("grid-2x2-4-v1", 4))
            .expect("four-panel prompt must compile");
        assert!(
            [
                " a.",
                " an.",
                " and.",
                " at.",
                " by.",
                " for.",
                " from.",
                " in.",
                " of.",
                " on.",
                " or.",
                " the.",
                " through.",
                " to.",
                " toward.",
                " while.",
                " with."
            ]
            .iter()
            .all(|fragment| !prompt.contains(fragment)),
            "bounded prose ended one clause on a dangling connector"
        );
    }

    /// Dense valid four-panel scenes remain inside the provider word budget.
    #[test]
    fn dense_four_panel_scene_cannot_overrun_the_prompt_budget() {
        let panels = (0..4)
            .map(|index| {
                json!({
                    "id": format!("p{}", index + 1),
                    "bounds": {"width": 100, "height": 100},
                    "semantic_job": "one two three four five six seven",
                    "scene": {
                        "description": "one two three four five six seven eight nine ten eleven",
                        "camera": {
                            "shot_scale": "medium_close",
                            "viewpoint": "objective",
                            "viewpoint_subject_id": "",
                            "framing": "environment",
                            "angle": "eye_level",
                            "depth_plan": "layered"
                        },
                        "lighting": "one two three four five"
                    }
                })
            })
            .collect::<Vec<_>>();
        let scene = json!({
            "manga_panel": {
                "panel_layout": {
                    "active_layout": {"template_id": "staggered-grid-4-v1"}
                },
                "page_design": {
                    "camera_arc": {
                        "progression": "one two three four five six seven",
                        "motivation": "one two three four five six seven eight"
                    },
                    "special_device": {"kind": "none"}
                },
                "panels": panels
            }
        });
        let prompt =
            compile_image_prompt(&scene).expect("dense valid prompt must fit the image contract");
        assert_eq!(
            (
                (150..=250).contains(&prompt.split_whitespace().count()),
                prompt.chars().all(|character| !character.is_ascii_digit()),
                prompt.ends_with(CLOSING),
            ),
            (true, true, true),
            "dense four-panel prose exceeded its bounded provider contract"
        );
    }
}
