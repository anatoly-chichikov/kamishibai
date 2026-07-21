use std::rc::Rc;
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::anyhow;
use anyhow::{Result, bail};
use image::DynamicImage;
use serde_json::Value;
use serde_json::json;

use super::{BorderDetector, ImageSource, Progress, Renderer, SceneText};

/// Render one scene through Gemini and reject invalid manga images.
#[derive(Clone)]
pub struct MangaRenderer<D> {
    client: Rc<dyn ImageSource>,
    retries: usize,
    text: D,
    border: BorderDetector,
    attempts: Option<PathBuf>,
}

impl<D> MangaRenderer<D> {
    /// Create one validating manga renderer.
    pub fn new<C>(client: C, retries: usize, text: D, border: BorderDetector) -> Self
    where
        C: ImageSource + 'static,
    {
        Self {
            client: Rc::new(client),
            retries,
            text,
            border,
            attempts: None,
        }
    }

    /// Preserve every raw production image attempt and its validation verdict.
    pub fn with_attempt_archive(mut self, directory: PathBuf) -> Self {
        self.attempts = Some(directory);
        self
    }
}

impl<D> std::fmt::Debug for MangaRenderer<D>
where
    D: std::fmt::Debug,
{
    /// Render one stable debug view for test diagnostics.
    fn fmt(&self, item: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = item.debug_struct("MangaRenderer");
        debug
            .field("client", &"ImageSource")
            .field("retries", &self.retries)
            .field("text", &self.text)
            .field("border", &self.border);
        debug.field("attempts", &self.attempts);
        debug.finish()
    }
}

impl<D> Renderer for MangaRenderer<D>
where
    D: SceneText,
{
    /// Return one rendered image for the scene.
    fn render(&self, scene: &Value, progress: &mut dyn Progress) -> Result<DynamicImage> {
        let panels = scene
            .get("manga_panel")
            .and_then(|root| root.get("panels"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut reason = String::new();
        for attempt in 0..self.retries {
            let bytes = self.client.image(scene)?;
            let journal = self
                .attempts
                .as_deref()
                .map(|directory| AttemptJournal::capture(directory, bytes.as_slice()))
                .transpose()?;
            let decoded = image::load_from_memory(bytes.as_slice());
            let gray = match decoded {
                Ok(image) => image.into_luma8(),
                Err(error) => {
                    record_attempt(journal.as_ref(), "error", error.to_string().as_str())?;
                    return Err(error.into());
                }
            };
            let found = self.text.detected(scene, &gray)?;
            let text_rejected = significant_registry_text(found.as_str());
            if text_rejected {
                reason = format!("OCR detected text: '{found}'");
                record_attempt(journal.as_ref(), "rejected", reason.as_str())?;
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            let failed = self.border.borders(&gray);
            if !failed.is_empty() {
                reason = format!("White border missing on: {}", failed.join(", "));
                record_attempt(journal.as_ref(), "rejected", reason.as_str())?;
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            if has_active_layout(scene) {
                if panels == 1 && !registry_topology_matches(&self.border, scene, &gray, panels) {
                    reason = String::from("Unexpected internal gutter in one-panel layout");
                    record_attempt(journal.as_ref(), "rejected", reason.as_str())?;
                    progress.retry("Rendering manga", attempt + 1, reason.as_str());
                    continue;
                }
                if panels > 1 && !registry_topology_matches(&self.border, scene, &gray, panels) {
                    reason = String::from("Registered panel topology was not detected");
                    record_attempt(journal.as_ref(), "rejected", reason.as_str())?;
                    progress.retry("Rendering manga", attempt + 1, reason.as_str());
                    continue;
                }
            } else if requires_gutter(scene, panels) && !self.border.gutter(&gray) {
                reason = String::from("No white gutter found");
                record_attempt(journal.as_ref(), "rejected", reason.as_str())?;
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            if found.is_empty() {
                record_attempt(journal.as_ref(), "accepted", "")?;
            } else {
                record_attempt(
                    journal.as_ref(),
                    "accepted",
                    format!("Ignored low-signal OCR: '{found}'").as_str(),
                )?;
            }
            return Ok(DynamicImage::ImageLuma8(gray));
        }
        bail!("Rejected after {} attempts: {}", self.retries, reason);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttemptJournal {
    sequence: usize,
    image: String,
    verdict: PathBuf,
}

impl AttemptJournal {
    fn capture(directory: &Path, bytes: &[u8]) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let sequence = fs::read_dir(directory)?
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| {
                name.strip_prefix("attempt-")?
                    .strip_suffix(".json")?
                    .parse::<usize>()
                    .ok()
            })
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow!("registry image attempt sequence overflow"))?;
        let extension = match image::guess_format(bytes).ok() {
            Some(image::ImageFormat::Png) => "png",
            Some(image::ImageFormat::Jpeg) => "jpg",
            Some(image::ImageFormat::WebP) => "webp",
            Some(image::ImageFormat::Gif) => "gif",
            _ => "bin",
        };
        let image = format!("attempt-{sequence:04}.{extension}");
        fs::write(directory.join(image.as_str()), bytes)?;
        let journal = Self {
            sequence,
            image,
            verdict: directory.join(format!("attempt-{sequence:04}.json")),
        };
        journal.record("pending", "")?;
        Ok(journal)
    }

    fn record(&self, status: &str, reason: &str) -> Result<()> {
        fs::write(
            self.verdict.as_path(),
            serde_json::to_vec_pretty(&json!({
                "sequence": self.sequence,
                "image": self.image,
                "status": status,
                "reason": reason
            }))?,
        )?;
        Ok(())
    }
}

fn record_attempt(journal: Option<&AttemptJournal>, status: &str, reason: &str) -> Result<()> {
    if let Some(journal) = journal {
        journal.record(status, reason)?;
    }
    Ok(())
}

fn has_active_layout(scene: &Value) -> bool {
    scene
        .pointer("/manga_panel/panel_layout/active_layout/template_id")
        .and_then(Value::as_str)
        .is_some_and(|template| !template.is_empty())
}

fn registry_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    panels: usize,
) -> bool {
    let kind = scene
        .pointer("/manga_panel/page_design/special_device/kind")
        .and_then(Value::as_str);
    let Some(declared) = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
        .filter(|declared| declared.len() == panels)
    else {
        return false;
    };
    if !registry_device_materialized(scene, kind) {
        return false;
    }
    if kind == Some("open_frame") {
        return open_frame_topology_matches(border, scene, declared, image, panels);
    }
    if emphasis_layout(scene, kind).is_some() {
        if kind == Some("crossing") {
            return crossing_emphasis_topology_matches(border, scene, declared, image, panels);
        }
        return emphasis_topology_matches(border, scene, declared, image, panels);
    }
    if kind == Some("crossing") {
        return crossing_regions_match(border, scene, image);
    }
    strict_topology_matches(border, scene, declared, image, panels)
}

fn strict_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    let Some(witnesses) = panel_witnesses(scene, panels, image) else {
        return false;
    };
    let points = witnesses.iter().flatten().copied().collect::<Vec<_>>();
    let (regions, labels) = border.region_measure(image, points.as_slice());
    let Some(assignments) = witness_labels(witnesses.as_slice(), labels.as_slice()) else {
        return false;
    };
    let distinct = assignments.into_iter().collect::<BTreeSet<_>>();
    regions == expected && distinct.len() == expected
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmphasisLayout {
    SlantedBottom,
    SlantedRail,
    StrongDiagonal,
}

fn emphasis_layout(scene: &Value, kind: Option<&str>) -> Option<EmphasisLayout> {
    if !matches!(
        kind,
        Some("none" | "crossing" | "diagonal_release" | "master_view")
    ) {
        return None;
    }
    match scene
        .pointer("/manga_panel/panel_layout/active_layout/template_id")
        .and_then(Value::as_str)?
    {
        "slanted-t-bottom-3-p2-v1" => Some(EmphasisLayout::SlantedBottom),
        "slanted-dominant-rail-3-p2-v1" => Some(EmphasisLayout::SlantedRail),
        "diagonal-split-2-end-strong-v1" => Some(EmphasisLayout::StrongDiagonal),
        _ => None,
    }
}

fn open_frame_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    if strict_topology_matches(border, scene, panels, image, expected) {
        return true;
    }
    let source = scene
        .pointer("/manga_panel/page_design/special_device/source_panel")
        .and_then(Value::as_str);
    let Some(source_index) = source.and_then(|source| panel_index(panels, source)) else {
        return false;
    };
    let Some(centers) = panel_centers(scene, panels, image) else {
        return false;
    };
    let (regions, labels) = border.region_measure(image, centers.as_slice());
    let companions = labels
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != source_index)
        .map(|(_, label)| *label)
        .collect::<Option<Vec<_>>>();
    let Some(companions) = companions else {
        return false;
    };
    if companions.len().checked_add(1) != Some(expected) {
        return false;
    }
    let distinct = companions.into_iter().collect::<BTreeSet<_>>();
    let source_isolated = labels
        .get(source_index)
        .copied()
        .flatten()
        .is_some_and(|source| !distinct.contains(&source));
    regions >= distinct.len()
        && regions <= expected
        && distinct.len().checked_add(1) == Some(expected)
        && source_isolated
}

fn crossing_emphasis_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    let Some(proofs) = slant_proofs(scene, panels).filter(|proofs| proofs.len() == 1) else {
        return false;
    };
    let Some((regions, assignments)) = registry_assignments(border, scene, panels, image) else {
        return false;
    };
    if exact_emphasis_topology_matches(
        border,
        scene,
        image,
        expected,
        regions,
        assignments.as_slice(),
        proofs.as_slice(),
    ) {
        return true;
    }
    regions.checked_add(1) == Some(expected)
        && crossing_assignments_match(scene, panels, assignments.as_slice())
        && registry_slants_match(
            border,
            scene,
            image,
            assignments.as_slice(),
            proofs.as_slice(),
        )
}

fn emphasis_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    let Some(proofs) = slant_proofs(scene, panels) else {
        return false;
    };
    if proofs.len() != 1 {
        return false;
    }
    let Some((regions, assignments)) = registry_assignments(border, scene, panels, image) else {
        return false;
    };
    if exact_emphasis_topology_matches(
        border,
        scene,
        image,
        expected,
        regions,
        assignments.as_slice(),
        proofs.as_slice(),
    ) {
        return true;
    }
    if regions.checked_add(1) != Some(expected) {
        return false;
    }
    let Some(sealed) = seal_merged_pair_corridor(scene, panels, image, assignments.as_slice())
    else {
        return false;
    };
    let Some((regions, assignments)) = registry_assignments(border, scene, panels, &sealed) else {
        return false;
    };
    exact_emphasis_topology_matches(
        border,
        scene,
        &sealed,
        expected,
        regions,
        assignments.as_slice(),
        proofs.as_slice(),
    )
}

fn registry_assignments(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
) -> Option<(usize, Vec<usize>)> {
    let centers = panel_centers(scene, panels, image)?;
    let (regions, labels) = border.region_measure(image, centers.as_slice());
    Some((regions, labels.into_iter().collect::<Option<Vec<_>>>()?))
}

fn exact_emphasis_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    expected: usize,
    regions: usize,
    assignments: &[usize],
    proofs: &[SlantProof],
) -> bool {
    let distinct = assignments.iter().copied().collect::<BTreeSet<_>>();
    regions == expected
        && distinct.len() == expected
        && registry_slants_match(border, scene, image, assignments, proofs)
}

fn seal_merged_pair_corridor(
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    assignments: &[usize],
) -> Option<image::GrayImage> {
    let pair = merged_pair(assignments)?;
    let centers = panel_centers(scene, panels, image)?;
    let first = centers.get(pair.first).copied()?;
    let second = centers.get(pair.second).copied()?;
    let radius = image.width().min(image.height()).checked_div(16)?.max(2);
    let mut sealed = image.clone();
    for y in 0..image.height() {
        for x in 0..image.width() {
            if !inside_bisector_corridor((x, y), first, second, radius) {
                continue;
            }
            let left = x.saturating_sub(1);
            let top = y.saturating_sub(1);
            let right = x.saturating_add(1).min(image.width().saturating_sub(1));
            let bottom = y.saturating_add(1).min(image.height().saturating_sub(1));
            if (top..=bottom).any(|other_y| {
                (left..=right).any(|other_x| image.get_pixel(other_x, other_y)[0] >= 250)
            }) {
                sealed.put_pixel(x, y, image::Luma([255]));
            }
        }
    }
    Some(sealed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PanelPair {
    first: usize,
    second: usize,
}

fn merged_pair(assignments: &[usize]) -> Option<PanelPair> {
    let pairs = (0..assignments.len())
        .flat_map(|first| {
            (first.saturating_add(1)..assignments.len()).filter_map(move |second| {
                (assignments.get(first) == assignments.get(second))
                    .then_some(PanelPair { first, second })
            })
        })
        .collect::<Vec<_>>();
    (pairs.len() == 1).then(|| pairs[0])
}

fn inside_bisector_corridor(
    point: (u32, u32),
    first: (u32, u32),
    second: (u32, u32),
    radius: u32,
) -> bool {
    let x = i128::from(point.0);
    let y = i128::from(point.1);
    let first_x = i128::from(first.0);
    let first_y = i128::from(first.1);
    let second_x = i128::from(second.0);
    let second_y = i128::from(second.1);
    let delta_x = second_x - first_x;
    let delta_y = second_y - first_y;
    let offset = (2 * (x * delta_x + y * delta_y)
        - (second_x * second_x + second_y * second_y - first_x * first_x - first_y * first_y))
        .abs();
    let span = delta_x.abs() + delta_y.abs();
    offset <= 2 * i128::from(radius) * span
}

fn registry_device_materialized(scene: &Value, kind: Option<&str>) -> bool {
    match kind {
        Some("crossing") => crossing_materialized(scene),
        Some("overlap") => overlap_materialized(scene),
        Some("inset") => inset_materialized(scene),
        Some("open_frame") => open_frame_materialized(scene),
        Some("none" | "single_splash" | "master_view" | "diagonal_release" | "scene_to_scene")
        | None => true,
        Some(_) => false,
    }
}

fn crossing_materialized(scene: &Value) -> bool {
    let Some(device) = scene.pointer("/manga_panel/page_design/special_device") else {
        return false;
    };
    let Some(source) = device.get("source_panel").and_then(Value::as_str) else {
        return false;
    };
    let Some(target) = device.get("target_panel").and_then(Value::as_str) else {
        return false;
    };
    let Some(subject) = device.get("subject_id").and_then(Value::as_str) else {
        return false;
    };
    let Some(panels) = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
    else {
        return false;
    };
    !source.is_empty()
        && !target.is_empty()
        && source != target
        && !subject.is_empty()
        && panel(panels, target).is_some()
        && panels
            .iter()
            .filter(|panel| {
                panel
                    .pointer("/continuity/breakout/enabled")
                    .and_then(Value::as_bool)
                    == Some(true)
            })
            .count()
            == 1
        && panel(panels, source).is_some_and(|panel| {
            panel
                .pointer("/continuity/breakout/enabled")
                .and_then(Value::as_bool)
                == Some(true)
                && panel
                    .pointer("/continuity/breakout/subject_id")
                    .and_then(Value::as_str)
                    == Some(subject)
                && panel
                    .pointer("/continuity/breakout/destination_panel")
                    .and_then(Value::as_str)
                    == Some(target)
        })
}

fn overlap_materialized(scene: &Value) -> bool {
    let Some((panels, source, target)) = device_pair(scene) else {
        return false;
    };
    let Some(front) = panel(panels, source) else {
        return false;
    };
    let Some(back) = panel(panels, target) else {
        return false;
    };
    front
        .pointer("/frame/overlaps_panel")
        .and_then(Value::as_str)
        == Some(target)
        && frame_level(front) > frame_level(back)
        && registry_bounds(front)
            .is_some_and(|bounds| registry_bounds(back).is_some_and(|other| bounds.overlaps(other)))
}

fn inset_materialized(scene: &Value) -> bool {
    let Some((panels, source, target)) = device_pair(scene) else {
        return false;
    };
    let Some(parent) = panel(panels, source) else {
        return false;
    };
    let Some(child) = panel(panels, target) else {
        return false;
    };
    child.pointer("/frame/shape").and_then(Value::as_str) == Some("inset")
        && child.pointer("/frame/parent_panel").and_then(Value::as_str) == Some(source)
        && child.pointer("/frame/border").and_then(Value::as_str) == Some("solid")
        && frame_level(child) > frame_level(parent)
        && registry_bounds(parent).is_some_and(|bounds| {
            registry_bounds(child).is_some_and(|other| bounds.contains(other))
        })
}

fn open_frame_materialized(scene: &Value) -> bool {
    let Some(device) = scene.pointer("/manga_panel/page_design/special_device") else {
        return false;
    };
    let Some(source) = device.get("source_panel").and_then(Value::as_str) else {
        return false;
    };
    let Some(panels) = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
    else {
        return false;
    };
    !source.is_empty()
        && device.get("target_panel").and_then(Value::as_str) == Some("")
        && panels.iter().all(|panel| {
            let source_panel = panel.get("id").and_then(Value::as_str) == Some(source);
            let border = panel.pointer("/frame/border").and_then(Value::as_str);
            let shape = panel.pointer("/frame/shape").and_then(Value::as_str);
            if source_panel {
                border == Some("none") && shape == Some("open_frame")
            } else {
                border == Some("solid") && shape != Some("open_frame")
            }
        })
        && panel(panels, source).is_some()
}

fn crossing_regions_match(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
) -> bool {
    let Some((panels, _, _)) = device_pair(scene) else {
        return false;
    };
    let Some(witnesses) = panel_witnesses(scene, panels, image) else {
        return false;
    };
    let points = witnesses.iter().flatten().copied().collect::<Vec<_>>();
    let (regions, labels) = border.region_measure(image, points.as_slice());
    if regions.checked_add(1) != Some(panels.len()) {
        return false;
    }
    let Some(assignments) = witness_labels(witnesses.as_slice(), labels.as_slice()) else {
        return false;
    };
    crossing_assignments_match(scene, panels, assignments.as_slice())
}

fn crossing_assignments_match(scene: &Value, panels: &[Value], assignments: &[usize]) -> bool {
    let Some((_, source, target)) = device_pair(scene) else {
        return false;
    };
    let Some(source_index) = panel_index(panels, source) else {
        return false;
    };
    let Some(target_index) = panel_index(panels, target) else {
        return false;
    };
    let Some(shared) = assignments.get(source_index).copied() else {
        return false;
    };
    if assignments.get(target_index).copied() != Some(shared) {
        return false;
    }
    let others = assignments
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != source_index && *index != target_index)
        .map(|(_, label)| *label)
        .collect::<BTreeSet<_>>();
    assignments
        .iter()
        .enumerate()
        .all(|(index, label)| index == source_index || index == target_index || *label != shared)
        && others.len() == panels.len().saturating_sub(2)
}

fn panel_witnesses(
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
) -> Option<Vec<Vec<(u32, u32)>>> {
    let width = scene
        .pointer("/manga_panel/canvas/width")
        .and_then(Value::as_u64)?;
    let height = scene
        .pointer("/manga_panel/canvas/height")
        .and_then(Value::as_u64)?;
    panels
        .iter()
        .map(|panel| {
            panel_witness_anchors(panel)?
                .into_iter()
                .map(|(x, y)| {
                    Some((
                        scale_point(x, width, image.width())?,
                        scale_point(y, height, image.height())?,
                    ))
                })
                .collect()
        })
        .collect()
}

fn panel_centers(
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
) -> Option<Vec<(u32, u32)>> {
    let width = scene
        .pointer("/manga_panel/canvas/width")
        .and_then(Value::as_u64)?;
    let height = scene
        .pointer("/manga_panel/canvas/height")
        .and_then(Value::as_u64)?;
    panels
        .iter()
        .map(|panel| {
            let (x, y) = panel_center(panel)?;
            Some((
                scale_point(x, width, image.width())?,
                scale_point(y, height, image.height())?,
            ))
        })
        .collect()
}

fn panel_center(panel: &Value) -> Option<(u64, u64)> {
    let polygon = polygon_points(panel)?;
    if polygon.is_empty() {
        registry_bounds(panel)?.center()
    } else {
        polygon_center(polygon.as_slice())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoordinatePair {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VerticalRails {
    span: CoordinatePair,
    left: CoordinatePair,
    right: CoordinatePair,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HorizontalRails {
    span: CoordinatePair,
    top: CoordinatePair,
    bottom: CoordinatePair,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CanvasSize {
    width: u64,
    height: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegionPair {
    first: usize,
    second: usize,
}

fn registry_slants_match(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    assignments: &[usize],
    proofs: &[SlantProof],
) -> bool {
    let Some(width) = scene
        .pointer("/manga_panel/canvas/width")
        .and_then(Value::as_u64)
    else {
        return false;
    };
    let Some(height) = scene
        .pointer("/manga_panel/canvas/height")
        .and_then(Value::as_u64)
    else {
        return false;
    };
    let canvas = CanvasSize { width, height };
    proofs.iter().all(|proof| {
        let Some(first) = assignments.get(proof.panels.first).copied() else {
            return false;
        };
        let Some(second) = assignments.get(proof.panels.second).copied() else {
            return false;
        };
        slant_matches(border, image, canvas, *proof, RegionPair { first, second })
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlantAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SlantProof {
    axis: SlantAxis,
    panels: PanelPair,
    span: CoordinatePair,
    scan: CoordinatePair,
    direction: Ordering,
}

fn slant_proofs(scene: &Value, panels: &[Value]) -> Option<Vec<SlantProof>> {
    let width = scene
        .pointer("/manga_panel/canvas/width")
        .and_then(Value::as_u64)?;
    let height = scene
        .pointer("/manga_panel/canvas/height")
        .and_then(Value::as_u64)?;
    Some(
        (0..panels.len())
            .flat_map(|first| {
                (first.saturating_add(1)..panels.len()).filter_map(move |second| {
                    vertical_slant_proof(
                        &panels[first],
                        &panels[second],
                        width,
                        PanelPair { first, second },
                    )
                    .or_else(|| {
                        horizontal_slant_proof(
                            &panels[first],
                            &panels[second],
                            height,
                            PanelPair { first, second },
                        )
                    })
                })
            })
            .collect(),
    )
}

fn vertical_slant_proof(
    left: &Value,
    right: &Value,
    width: u64,
    panels: PanelPair,
) -> Option<SlantProof> {
    let left_rails = vertical_rails(left)?;
    let right_rails = vertical_rails(right)?;
    if left_rails.span != right_rails.span {
        return None;
    }
    let start_gap = right_rails.left.start.checked_sub(left_rails.right.start)?;
    let end_gap = right_rails.left.end.checked_sub(left_rails.right.end)?;
    let maximum_gap = width.checked_div(16)?.max(1);
    if start_gap > maximum_gap || end_gap > maximum_gap {
        return None;
    }
    let start = left_rails.right.start.checked_add(right_rails.left.start)?;
    let end = left_rails.right.end.checked_add(right_rails.left.end)?;
    let direction = start.cmp(&end);
    if direction == Ordering::Equal {
        return None;
    }
    let left_center = panel_center(left)?.0;
    let right_center = panel_center(right)?.0;
    (left_center < right_center).then_some(SlantProof {
        axis: SlantAxis::Vertical,
        panels,
        span: left_rails.span,
        scan: CoordinatePair {
            start: left_center,
            end: right_center,
        },
        direction,
    })
}

fn horizontal_slant_proof(
    top: &Value,
    bottom: &Value,
    height: u64,
    panels: PanelPair,
) -> Option<SlantProof> {
    let top_rails = horizontal_rails(top)?;
    let bottom_rails = horizontal_rails(bottom)?;
    if top_rails.span != bottom_rails.span {
        return None;
    }
    let start_gap = bottom_rails.top.start.checked_sub(top_rails.bottom.start)?;
    let end_gap = bottom_rails.top.end.checked_sub(top_rails.bottom.end)?;
    let maximum_gap = height.checked_div(16)?.max(1);
    if start_gap > maximum_gap || end_gap > maximum_gap {
        return None;
    }
    let start = top_rails.bottom.start.checked_add(bottom_rails.top.start)?;
    let end = top_rails.bottom.end.checked_add(bottom_rails.top.end)?;
    let direction = start.cmp(&end);
    if direction == Ordering::Equal {
        return None;
    }
    let top_center = panel_center(top)?.1;
    let bottom_center = panel_center(bottom)?.1;
    (top_center < bottom_center).then_some(SlantProof {
        axis: SlantAxis::Horizontal,
        panels,
        span: top_rails.span,
        scan: CoordinatePair {
            start: top_center,
            end: bottom_center,
        },
        direction,
    })
}

fn vertical_rails(panel: &Value) -> Option<VerticalRails> {
    let polygon = polygon_points(panel)?;
    if polygon.is_empty() {
        return None;
    }
    let top = polygon.iter().map(|(_, y)| *y).min()?;
    let bottom = polygon.iter().map(|(_, y)| *y).max()?;
    if top >= bottom {
        return None;
    }
    let top_x = polygon
        .iter()
        .filter(|(_, y)| *y == top)
        .map(|(x, _)| *x)
        .collect::<Vec<_>>();
    let bottom_x = polygon
        .iter()
        .filter(|(_, y)| *y == bottom)
        .map(|(x, _)| *x)
        .collect::<Vec<_>>();
    Some(VerticalRails {
        span: CoordinatePair {
            start: top,
            end: bottom,
        },
        left: CoordinatePair {
            start: top_x.iter().copied().min()?,
            end: bottom_x.iter().copied().min()?,
        },
        right: CoordinatePair {
            start: top_x.iter().copied().max()?,
            end: bottom_x.iter().copied().max()?,
        },
    })
}

fn horizontal_rails(panel: &Value) -> Option<HorizontalRails> {
    let polygon = polygon_points(panel)?;
    if polygon.is_empty() {
        return None;
    }
    let left = polygon.iter().map(|(x, _)| *x).min()?;
    let right = polygon.iter().map(|(x, _)| *x).max()?;
    if left >= right {
        return None;
    }
    let left_y = polygon
        .iter()
        .filter(|(x, _)| *x == left)
        .map(|(_, y)| *y)
        .collect::<Vec<_>>();
    let right_y = polygon
        .iter()
        .filter(|(x, _)| *x == right)
        .map(|(_, y)| *y)
        .collect::<Vec<_>>();
    Some(HorizontalRails {
        span: CoordinatePair {
            start: left,
            end: right,
        },
        top: CoordinatePair {
            start: left_y.iter().copied().min()?,
            end: right_y.iter().copied().min()?,
        },
        bottom: CoordinatePair {
            start: left_y.iter().copied().max()?,
            end: right_y.iter().copied().max()?,
        },
    })
}

fn slant_matches(
    border: &BorderDetector,
    image: &image::GrayImage,
    canvas: CanvasSize,
    proof: SlantProof,
    regions: RegionPair,
) -> bool {
    let Some(span) = proof.span.end.checked_sub(proof.span.start) else {
        return false;
    };
    let Some(first) = proof.span.start.checked_add(span / 4) else {
        return false;
    };
    let Some(second) = proof.span.start.checked_add(span.saturating_mul(3) / 4) else {
        return false;
    };
    let Some(first) = separator_position(border, image, canvas, proof, first, regions) else {
        return false;
    };
    let Some(second) = separator_position(border, image, canvas, proof, second, regions) else {
        return false;
    };
    first.cmp(&second) == proof.direction
}

fn separator_position(
    border: &BorderDetector,
    image: &image::GrayImage,
    canvas: CanvasSize,
    proof: SlantProof,
    position: u64,
    regions: RegionPair,
) -> Option<u64> {
    let span = proof.scan.end.checked_sub(proof.scan.start)?;
    let mut points = (0u64..=32)
        .map(|step| {
            let value = proof
                .scan
                .start
                .checked_add(span.checked_mul(step)?.checked_div(32)?)?;
            match proof.axis {
                SlantAxis::Vertical => Some((
                    scale_point(value, canvas.width, image.width())?,
                    scale_point(position, canvas.height, image.height())?,
                )),
                SlantAxis::Horizontal => Some((
                    scale_point(position, canvas.width, image.width())?,
                    scale_point(value, canvas.height, image.height())?,
                )),
            }
        })
        .collect::<Option<Vec<_>>>()?;
    points.dedup();
    let (_, labels) = border.region_measure(image, points.as_slice());
    transition_position(
        points.as_slice(),
        labels.as_slice(),
        proof.axis,
        regions.first,
        regions.second,
    )
}

fn transition_position(
    points: &[(u32, u32)],
    labels: &[Option<usize>],
    axis: SlantAxis,
    first: usize,
    second: usize,
) -> Option<u64> {
    if points.len() != labels.len() {
        return None;
    }
    let mut first_edge = None;
    let mut second_edge = None;
    for (point, label) in points.iter().zip(labels) {
        let coordinate = match axis {
            SlantAxis::Vertical => point.0,
            SlantAxis::Horizontal => point.1,
        };
        match label {
            Some(label) if *label == first && second_edge.is_none() => {
                first_edge = Some(coordinate)
            }
            Some(label) if *label == first => return None,
            Some(label) if *label == second && second_edge.is_none() => {
                second_edge = Some(coordinate)
            }
            Some(label) if *label == second => {}
            Some(_) => return None,
            None => {}
        }
    }
    let first = first_edge?;
    let second = second_edge?;
    (first < second).then(|| u64::from(first).saturating_add(u64::from(second)))
}

fn panel_witness_anchors(panel: &Value) -> Option<Vec<(u64, u64)>> {
    let polygon = polygon_points(panel)?;
    if polygon.is_empty() {
        return Some(vec![registry_bounds(panel)?.center()?]);
    }
    let center = polygon_center(polygon.as_slice())?;
    let mut witnesses = Vec::with_capacity(polygon.len().checked_add(1)?);
    witnesses.push(center);
    for (x, y) in polygon {
        witnesses.push((contract(x, center.0)?, contract(y, center.1)?));
    }
    witnesses.sort_unstable();
    witnesses.dedup();
    Some(witnesses)
}

fn witness_labels(witnesses: &[Vec<(u32, u32)>], labels: &[Option<usize>]) -> Option<Vec<usize>> {
    let mut offset = 0usize;
    let regions = witnesses
        .iter()
        .map(|points| {
            let end = offset.checked_add(points.len())?;
            let labels = labels.get(offset..end)?;
            let first = labels.first().copied().flatten()?;
            offset = end;
            labels
                .iter()
                .all(|label| *label == Some(first))
                .then_some(first)
        })
        .collect::<Option<Vec<_>>>()?;
    (offset == labels.len()).then_some(regions)
}

fn polygon_points(panel: &Value) -> Option<Vec<(u64, u64)>> {
    let Some(polygon) = panel.pointer("/frame/polygon").and_then(Value::as_array) else {
        return Some(Vec::new());
    };
    polygon
        .iter()
        .map(|point| {
            Some((
                point
                    .as_array()
                    .and_then(|coordinates| coordinates.first())
                    .or_else(|| point.get("x"))?
                    .as_u64()?,
                point
                    .as_array()
                    .and_then(|coordinates| coordinates.get(1))
                    .or_else(|| point.get("y"))?
                    .as_u64()?,
            ))
        })
        .collect()
}

fn polygon_center(polygon: &[(u64, u64)]) -> Option<(u64, u64)> {
    let count = u64::try_from(polygon.len())
        .ok()
        .filter(|count| *count > 0)?;
    let (x, y) = polygon.iter().try_fold((0u64, 0u64), |sum, point| {
        Some((sum.0.checked_add(point.0)?, sum.1.checked_add(point.1)?))
    })?;
    Some((x / count, y / count))
}

fn contract(value: u64, center: u64) -> Option<u64> {
    value.checked_mul(7)?.checked_add(center)?.checked_div(8)
}

fn scale_point(value: u64, canvas: u64, image: u32) -> Option<u32> {
    if canvas == 0 || value >= canvas || image == 0 {
        return None;
    }
    let scaled = value.checked_mul(u64::from(image))?.checked_div(canvas)?;
    u32::try_from(scaled).ok().filter(|value| *value < image)
}

fn device_pair(scene: &Value) -> Option<(&[Value], &str, &str)> {
    let device = scene.pointer("/manga_panel/page_design/special_device")?;
    let source = device.get("source_panel")?.as_str()?;
    let target = device.get("target_panel")?.as_str()?;
    let panels = scene.pointer("/manga_panel/panels")?.as_array()?;
    (!source.is_empty() && !target.is_empty() && source != target).then_some((
        panels.as_slice(),
        source,
        target,
    ))
}

fn panel<'a>(panels: &'a [Value], id: &str) -> Option<&'a Value> {
    panel_index(panels, id).and_then(|index| panels.get(index))
}

fn panel_index(panels: &[Value], id: &str) -> Option<usize> {
    panels
        .iter()
        .position(|panel| panel.get("id").and_then(Value::as_str) == Some(id))
}

fn frame_level(panel: &Value) -> u64 {
    panel
        .pointer("/frame/z_index")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryBounds {
    x: u64,
    y: u64,
    width: u64,
    height: u64,
}

impl RegistryBounds {
    fn center(self) -> Option<(u64, u64)> {
        Some((
            self.x.checked_add(self.width / 2)?,
            self.y.checked_add(self.height / 2)?,
        ))
    }

    fn contains(self, other: Self) -> bool {
        self.x <= other.x
            && self.y <= other.y
            && self
                .right()
                .is_some_and(|right| other.right().is_some_and(|other| other <= right))
            && self
                .bottom()
                .is_some_and(|bottom| other.bottom().is_some_and(|other| other <= bottom))
    }

    fn overlaps(self, other: Self) -> bool {
        self.right()
            .zip(other.right())
            .is_some_and(|(right, other_right)| self.x < other_right && other.x < right)
            && self
                .bottom()
                .zip(other.bottom())
                .is_some_and(|(bottom, other_bottom)| self.y < other_bottom && other.y < bottom)
    }

    fn right(self) -> Option<u64> {
        self.x.checked_add(self.width)
    }

    fn bottom(self) -> Option<u64> {
        self.y.checked_add(self.height)
    }
}

fn registry_bounds(panel: &Value) -> Option<RegistryBounds> {
    let bounds = panel.get("bounds")?;
    Some(RegistryBounds {
        x: bounds.get("x")?.as_u64()?,
        y: bounds.get("y")?.as_u64()?,
        width: bounds.get("width")?.as_u64()?,
        height: bounds.get("height")?.as_u64()?,
    })
}

fn significant_registry_text(found: &str) -> bool {
    found
        .split(|value: char| !value.is_alphanumeric())
        .any(|token| {
            !token.is_empty()
                && (token.chars().any(|value| value.is_numeric())
                    || !token.is_ascii()
                    || token.len() >= 3)
        })
}

fn requires_gutter(scene: &Value, panels: usize) -> bool {
    if panels <= 1 {
        return false;
    }
    !matches!(
        scene
            .pointer("/manga_panel/page_design/special_device/kind")
            .and_then(Value::as_str),
        Some("inset" | "crossing" | "overlap" | "diagonal_release" | "open_frame")
    )
}
