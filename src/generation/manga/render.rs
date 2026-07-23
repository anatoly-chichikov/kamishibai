use std::rc::Rc;
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use anyhow::anyhow;
use image::DynamicImage;
use serde_json::Value;
use serde_json::json;

use super::{BorderDetector, ImageSource, Progress, Renderer, SceneText};

#[derive(Clone, Debug, Eq, PartialEq)]
enum RenderRejection {
    Topology(String),
    Ocr(String),
    Border(String),
    LegacyGutter(String),
    Other(String),
}

impl RenderRejection {
    fn topology(reason: &str) -> Self {
        Self::Topology(String::from(reason))
    }

    fn ocr(reason: String) -> Self {
        Self::Ocr(reason)
    }

    fn border(reason: String) -> Self {
        Self::Border(reason)
    }

    fn legacy_gutter(reason: &str) -> Self {
        Self::LegacyGutter(String::from(reason))
    }

    fn other(reason: String) -> Self {
        Self::Other(reason)
    }

    fn reason(&self) -> &str {
        match self {
            Self::Topology(reason)
            | Self::Ocr(reason)
            | Self::Border(reason)
            | Self::LegacyGutter(reason)
            | Self::Other(reason) => reason.as_str(),
        }
    }

    fn category(&self) -> &'static str {
        match self {
            Self::Topology(_) => "topology",
            Self::Ocr(_) => "ocr",
            Self::Border(_) => "border",
            Self::LegacyGutter(_) => "legacy_gutter",
            Self::Other(_) => "other",
        }
    }
}

/// Report one exhausted manga render while retaining its rejection category.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MangaRenderRejection {
    attempts: usize,
    rejection: RenderRejection,
}

impl MangaRenderRejection {
    fn new(attempts: usize, rejection: RenderRejection) -> Self {
        Self {
            attempts,
            rejection,
        }
    }

    /// Return the durable local validation category for retry planning.
    pub(crate) fn category(&self) -> &'static str {
        self.rejection.category()
    }
}

impl std::fmt::Display for MangaRenderRejection {
    fn fmt(&self, item: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            item,
            "Rejected after {} attempts: {}",
            self.attempts,
            self.rejection.reason()
        )
    }
}

impl std::error::Error for MangaRenderRejection {}

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
        let provider = project(scene);
        let mut rejection = RenderRejection::other(String::new());
        for attempt in 0..self.retries {
            let bytes = self.client.image(&provider)?;
            let journal = self
                .attempts
                .as_deref()
                .map(|directory| AttemptJournal::capture(directory, bytes.as_slice(), &provider))
                .transpose()?;
            let decoded = image::load_from_memory(bytes.as_slice());
            let gray = match decoded {
                Ok(image) => image.into_luma8(),
                Err(error) => {
                    record_attempt(
                        journal.as_ref(),
                        "error",
                        "transport_or_decode",
                        error.to_string().as_str(),
                    )?;
                    return Err(error.into());
                }
            };
            let found = self.text.detected(scene, &gray)?;
            let text_rejected = significant_registry_text(found.as_str());
            if text_rejected {
                rejection = RenderRejection::ocr(format!("OCR detected text: '{found}'"));
                record_attempt(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            let failed = self.border.borders(&gray);
            if !failed.is_empty() {
                rejection = RenderRejection::border(format!(
                    "White border missing on: {}",
                    failed.join(", ")
                ));
                record_attempt(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            if has_active_layout(scene) {
                if panels == 1 && !registry_topology_matches(&self.border, scene, &gray, panels) {
                    rejection =
                        RenderRejection::topology("Unexpected internal gutter in one-panel layout");
                    record_attempt(
                        journal.as_ref(),
                        "rejected",
                        rejection.category(),
                        rejection.reason(),
                    )?;
                    progress.retry("Rendering manga", attempt + 1, rejection.reason());
                    continue;
                }
                if panels > 1 && !registry_topology_matches(&self.border, scene, &gray, panels) {
                    rejection =
                        RenderRejection::topology("Registered panel topology was not detected");
                    record_attempt(
                        journal.as_ref(),
                        "rejected",
                        rejection.category(),
                        rejection.reason(),
                    )?;
                    progress.retry("Rendering manga", attempt + 1, rejection.reason());
                    continue;
                }
            } else if requires_gutter(scene, panels) && !self.border.gutter(&gray) {
                rejection = RenderRejection::legacy_gutter("No white gutter found");
                record_attempt(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            if found.is_empty() {
                record_attempt(journal.as_ref(), "accepted", "accepted", "")?;
            } else {
                record_attempt(
                    journal.as_ref(),
                    "accepted",
                    "accepted",
                    format!("Ignored low-signal OCR: '{found}'").as_str(),
                )?;
            }
            return Ok(DynamicImage::ImageLuma8(gray));
        }
        Err(MangaRenderRejection::new(self.retries, rejection).into())
    }
}

fn project(scene: &Value) -> Value {
    let mut provider = scene.clone();
    if let Some(meta) = provider
        .pointer_mut("/manga_panel/meta")
        .and_then(Value::as_object_mut)
    {
        meta.remove("title");
        meta.remove("description");
    }
    if let Some(selection) = provider
        .pointer_mut("/manga_panel/meta/layout_selection")
        .and_then(Value::as_object_mut)
    {
        selection
            .retain(|key, _| matches!(key.as_str(), "chosen_template_id" | "scene_attempt_index"));
    }
    if let Some(layout) = provider
        .pointer_mut("/manga_panel/panel_layout")
        .and_then(Value::as_object_mut)
    {
        layout.remove("conditional_permissions");
        layout.remove("permissions_from");
    }
    provider
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttemptJournal {
    sequence: usize,
    image: String,
    scene: String,
    verdict: PathBuf,
}

impl AttemptJournal {
    fn capture(directory: &Path, bytes: &[u8], scene: &Value) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let sequence = fs::read_dir(directory)?
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| {
                name.strip_prefix("attempt-")?
                    .split('.')
                    .next()?
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
        let scene_name = format!("attempt-{sequence:04}.scene.json");
        fs::write(directory.join(image.as_str()), bytes)?;
        fs::write(
            directory.join(scene_name.as_str()),
            serde_json::to_vec_pretty(scene)?,
        )?;
        let journal = Self {
            sequence,
            image,
            scene: scene_name,
            verdict: directory.join(format!("attempt-{sequence:04}.json")),
        };
        journal.record("pending", "pending", "")?;
        Ok(journal)
    }

    fn record(&self, status: &str, category: &str, reason: &str) -> Result<()> {
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(
            staged.as_file_mut(),
            &json!({
                "sequence": self.sequence,
                "image": self.image,
                "scene": self.scene,
                "status": status,
                "category": category,
                "reason": reason
            }),
        )?;
        staged.as_file().sync_all()?;
        staged.persist(self.verdict.as_path())?;
        Ok(())
    }
}

fn record_attempt(
    journal: Option<&AttemptJournal>,
    status: &str,
    category: &str,
    reason: &str,
) -> Result<()> {
    if let Some(journal) = journal {
        journal.record(status, category, reason)?;
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
    panel_witnesses(scene, panels, image).is_some_and(|witnesses| {
        let points = witnesses.iter().flatten().copied().collect::<Vec<_>>();
        let (regions, labels) = border.region_measure(image, points.as_slice());
        witness_labels(witnesses.as_slice(), labels.as_slice()).is_some_and(|assignments| {
            let distinct = assignments.into_iter().collect::<BTreeSet<_>>();
            regions == expected && distinct.len() == expected
        })
    }) || staggered_grid_layout(scene)
        && staggered_grid_topology_matches(border, scene, panels, image, expected)
}

fn staggered_grid_layout(scene: &Value) -> bool {
    scene
        .pointer("/manga_panel/panel_layout/active_layout/template_id")
        .and_then(Value::as_str)
        == Some("staggered-grid-4-v1")
}

fn staggered_grid_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    if expected != 4 || panels.len() != expected {
        return false;
    }
    let Some((regions, assignments)) = registry_assignments(border, scene, panels, image) else {
        return false;
    };
    if regions != expected || assignments.iter().copied().collect::<BTreeSet<_>>().len() != expected
    {
        return false;
    }
    let Some(rows) = staggered_rows(panels) else {
        return false;
    };
    let Some(top) = row_separator_position(border, scene, image, rows[0], assignments.as_slice())
    else {
        return false;
    };
    let Some(bottom) =
        row_separator_position(border, scene, image, rows[1], assignments.as_slice())
    else {
        return false;
    };
    let Some(declared_top) = declared_row_separator(panels, rows[0]) else {
        return false;
    };
    let Some(declared_bottom) = declared_row_separator(panels, rows[1]) else {
        return false;
    };
    let minimum = u64::from(image.width().checked_div(64).unwrap_or(0).max(1)).saturating_mul(2);
    top.cmp(&bottom) == declared_top.cmp(&declared_bottom) && top.abs_diff(bottom) >= minimum
}

fn staggered_rows(panels: &[Value]) -> Option<[[usize; 2]; 2]> {
    let mut indices = (0..panels.len()).collect::<Vec<_>>();
    indices.sort_by_key(|index| panel_center(&panels[*index]).map(|(_, y)| y));
    let mut top = [*indices.first()?, *indices.get(1)?];
    let mut bottom = [*indices.get(2)?, *indices.get(3)?];
    top.sort_by_key(|index| panel_center(&panels[*index]).map(|(x, _)| x));
    bottom.sort_by_key(|index| panel_center(&panels[*index]).map(|(x, _)| x));
    Some([top, bottom])
}

fn declared_row_separator(panels: &[Value], row: [usize; 2]) -> Option<u64> {
    let left = registry_bounds(panels.get(row[0])?)?;
    let right = registry_bounds(panels.get(row[1])?)?;
    left.right()?.checked_add(right.x)?.checked_div(2)
}

fn row_separator_position(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    row: [usize; 2],
    assignments: &[usize],
) -> Option<u64> {
    let centers = panel_centers(
        scene,
        scene.pointer("/manga_panel/panels")?.as_array()?,
        image,
    )?;
    let first = *centers.get(row[0])?;
    let second = *centers.get(row[1])?;
    let span = second.0.checked_sub(first.0)?;
    let y = first.1.checked_add(second.1)?.checked_div(2)?;
    let mut points = (0u64..=64)
        .map(|step| {
            let offset = u64::from(span).checked_mul(step)?.checked_div(64)?;
            Some((first.0.checked_add(u32::try_from(offset).ok()?)?, y))
        })
        .collect::<Option<Vec<_>>>()?;
    points.dedup();
    let (_, labels) = border.region_measure(image, points.as_slice());
    transition_position(
        points.as_slice(),
        labels.as_slice(),
        SlantAxis::Vertical,
        *assignments.get(row[0])?,
        *assignments.get(row[1])?,
    )
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
    let source_open = labels.get(source_index).is_some_and(Option::is_none);
    regions >= distinct.len()
        && regions <= expected
        && distinct.len().checked_add(1) == Some(expected)
        && source_open
        && source_content_visible(scene, panels.get(source_index), image)
}

fn source_content_visible(scene: &Value, panel: Option<&Value>, image: &image::GrayImage) -> bool {
    let Some(panel) = panel else {
        return false;
    };
    let Some(bounds) = registry_bounds(panel) else {
        return false;
    };
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
    let Some(right) = bounds.right() else {
        return false;
    };
    let Some(bottom) = bounds.bottom() else {
        return false;
    };
    let Some(left) = scale_point(bounds.x, width, image.width()) else {
        return false;
    };
    let Some(top) = scale_point(bounds.y, height, image.height()) else {
        return false;
    };
    let Some(right) = scale_point(right, width, image.width()) else {
        return false;
    };
    let Some(bottom) = scale_point(bottom, height, image.height()) else {
        return false;
    };
    let total =
        u64::from(right.saturating_sub(left)).saturating_mul(u64::from(bottom.saturating_sub(top)));
    let dark = (top..bottom)
        .flat_map(|y| (left..right).map(move |x| (x, y)))
        .filter(|(x, y)| image.get_pixel(*x, *y)[0] < 220)
        .count();
    let dark = u64::try_from(dark).unwrap_or(u64::MAX);
    total > 0 && dark.saturating_mul(100) >= total.saturating_mul(2)
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
        return crossing_separator_interrupted(scene, panels, image);
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

fn crossing_separator_interrupted(
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
) -> bool {
    let Some((_, source, target)) = device_pair(scene) else {
        return false;
    };
    let Some(source) = panel_index(panels, source) else {
        return false;
    };
    let Some(target) = panel_index(panels, target) else {
        return false;
    };
    let Some(separator) = device_separator(scene, panels, image, source, target) else {
        return false;
    };
    let samples = separator_samples(image, separator);
    let minimum = u64::from(match separator.axis {
        SlantAxis::Horizontal => image.height(),
        SlantAxis::Vertical => image.width(),
    })
    .checked_div(256)
    .unwrap_or(0)
    .max(1);
    let maximum = u64::from(match separator.axis {
        SlantAxis::Horizontal => image.height(),
        SlantAxis::Vertical => image.width(),
    })
    .checked_div(16)
    .unwrap_or(0)
    .max(minimum);
    let midpoint = separator
        .scan
        .start
        .checked_add(separator.scan.end)
        .and_then(|sum| sum.checked_div(2));
    let Some(midpoint) = midpoint else {
        return false;
    };
    let tolerance = separator
        .scan
        .end
        .saturating_sub(separator.scan.start)
        .checked_div(4)
        .unwrap_or(0)
        .max(1);
    separator_groups(samples.as_slice())
        .into_iter()
        .any(|group| {
            u64::try_from(group.len()).is_ok_and(|length| {
                (minimum..=maximum).contains(&length)
                    && group.iter().all(|sample| sample.interrupted)
                    && group
                        .first()
                        .zip(group.last())
                        .map(|(first, last)| {
                            u64::from(first.position)
                                .saturating_add(u64::from(last.position))
                                .checked_div(2)
                                .unwrap_or(u64::MAX)
                                .abs_diff(midpoint)
                                <= tolerance
                        })
                        .unwrap_or(false)
            })
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeviceSeparator {
    axis: SlantAxis,
    scan: CoordinatePair,
    cross: CoordinatePair,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SeparatorSample {
    position: u32,
    candidate: bool,
    interrupted: bool,
}

fn device_separator(
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    source: usize,
    target: usize,
) -> Option<DeviceSeparator> {
    let width = scene
        .pointer("/manga_panel/canvas/width")
        .and_then(Value::as_u64)?;
    let height = scene
        .pointer("/manga_panel/canvas/height")
        .and_then(Value::as_u64)?;
    let source_bounds = registry_bounds(panels.get(source)?)?;
    let target_bounds = registry_bounds(panels.get(target)?)?;
    let centers = panel_centers(scene, panels, image)?;
    let source_center = *centers.get(source)?;
    let target_center = *centers.get(target)?;
    let mut separators = Vec::new();
    if source_bounds.bottom()? <= target_bounds.y || target_bounds.bottom()? <= source_bounds.y {
        let start = source_bounds.x.max(target_bounds.x);
        let end = source_bounds.right()?.min(target_bounds.right()?);
        if start < end {
            separators.push((
                source_bounds
                    .bottom()?
                    .abs_diff(target_bounds.y)
                    .min(target_bounds.bottom()?.abs_diff(source_bounds.y)),
                DeviceSeparator {
                    axis: SlantAxis::Horizontal,
                    scan: CoordinatePair {
                        start: u64::from(source_center.1.min(target_center.1)),
                        end: u64::from(source_center.1.max(target_center.1)),
                    },
                    cross: CoordinatePair {
                        start: u64::from(scale_point(start, width, image.width())?),
                        end: u64::from(scale_point(end, width, image.width())?),
                    },
                },
            ));
        }
    }
    if source_bounds.right()? <= target_bounds.x || target_bounds.right()? <= source_bounds.x {
        let start = source_bounds.y.max(target_bounds.y);
        let end = source_bounds.bottom()?.min(target_bounds.bottom()?);
        if start < end {
            separators.push((
                source_bounds
                    .right()?
                    .abs_diff(target_bounds.x)
                    .min(target_bounds.right()?.abs_diff(source_bounds.x)),
                DeviceSeparator {
                    axis: SlantAxis::Vertical,
                    scan: CoordinatePair {
                        start: u64::from(source_center.0.min(target_center.0)),
                        end: u64::from(source_center.0.max(target_center.0)),
                    },
                    cross: CoordinatePair {
                        start: u64::from(scale_point(start, height, image.height())?),
                        end: u64::from(scale_point(end, height, image.height())?),
                    },
                },
            ));
        }
    }
    separators.sort_by_key(|(gap, _)| *gap);
    separators.first().map(|(_, separator)| *separator)
}

fn separator_samples(image: &image::GrayImage, separator: DeviceSeparator) -> Vec<SeparatorSample> {
    let cross = separator.cross.end.saturating_sub(separator.cross.start);
    let required = cross.checked_div(64).unwrap_or(0).max(1);
    (separator.scan.start..=separator.scan.end)
        .filter_map(|position| {
            let position = u32::try_from(position).ok()?;
            let mut white = 0u64;
            let mut run = 0u64;
            let mut longest = 0u64;
            for cross in separator.cross.start..separator.cross.end {
                let cross = u32::try_from(cross).ok()?;
                let pixel = match separator.axis {
                    SlantAxis::Horizontal => image.get_pixel(cross, position)[0],
                    SlantAxis::Vertical => image.get_pixel(position, cross)[0],
                };
                if pixel >= 220 {
                    white = white.saturating_add(1);
                    run = 0;
                } else {
                    run = run.saturating_add(1);
                    longest = longest.max(run);
                }
            }
            Some(SeparatorSample {
                position,
                candidate: white.saturating_mul(100) >= cross.saturating_mul(60),
                interrupted: longest >= required,
            })
        })
        .collect()
}

fn separator_groups(samples: &[SeparatorSample]) -> Vec<&[SeparatorSample]> {
    let mut groups = Vec::new();
    let mut start = None;
    for (index, sample) in samples.iter().enumerate() {
        if sample.candidate && start.is_none() {
            start = Some(index);
        }
        if !sample.candidate
            && let Some(start) = start.take()
        {
            groups.push(&samples[start..index]);
        }
    }
    if let Some(start) = start {
        groups.push(&samples[start..]);
    }
    groups
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
    match proof.axis {
        SlantAxis::Horizontal => first != second,
        SlantAxis::Vertical => first.cmp(&second) == proof.direction,
    }
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

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Error;
    use tempfile::TempDir;

    use super::{AttemptJournal, MangaRenderRejection, RenderRejection};

    fn local(rejection: RenderRejection) -> bool {
        Error::new(MangaRenderRejection::new(3, rejection))
            .downcast_ref::<MangaRenderRejection>()
            .is_some()
    }

    /// Typed manga errors identify every local image-validation rejection class.
    #[test]
    fn manga_render_rejection_downcast_identifies_every_local_gate() {
        assert_eq!(
            [
                RenderRejection::topology("topology"),
                RenderRejection::ocr(String::from("ocr")),
                RenderRejection::border(String::from("border")),
                RenderRejection::legacy_gutter("legacy gutter"),
                RenderRejection::other(String::from("other")),
            ]
            .map(local),
            [true; 5],
            "typed manga errors no longer identify every local validation rejection"
        );
    }

    /// Provider, transport, and pre-image errors never masquerade as local rejections.
    #[test]
    fn nonlocal_errors_do_not_downcast_as_local_rejections() {
        assert_eq!(
            [
                "provider rejected request",
                "transport failed",
                "scene composition failed before image",
            ]
            .map(|message| {
                Error::msg(message)
                    .downcast_ref::<MangaRenderRejection>()
                    .is_some()
            }),
            [false; 3],
            "a nonlocal failure was typed as a local image-validation rejection"
        );
    }

    /// Typed manga errors expose a distinct persistent category for every validation gate.
    #[test]
    fn manga_render_rejection_categories_distinguish_validation_gates() {
        assert_eq!(
            [
                RenderRejection::topology("topology"),
                RenderRejection::ocr(String::from("ocr")),
                RenderRejection::border(String::from("border")),
                RenderRejection::legacy_gutter("legacy gutter"),
                RenderRejection::other(String::from("other")),
            ]
            .map(|rejection| rejection.category()),
            ["topology", "ocr", "border", "legacy_gutter", "other"],
            "typed manga errors collapse distinct validation gates"
        );
    }

    /// Typed manga errors preserve the renderer's established terminal message.
    #[test]
    fn manga_render_rejection_display_remains_unchanged() {
        assert_eq!(
            MangaRenderRejection::new(
                3,
                RenderRejection::topology("Registered panel topology was not detected"),
            )
            .to_string(),
            String::from("Rejected after 3 attempts: Registered panel topology was not detected"),
            "typed manga error changed the established terminal message"
        );
    }

    /// Attempt capture never reuses a sequence reserved by an interrupted raw image write.
    #[test]
    fn attempt_journal_advances_past_orphaned_evidence() {
        let temporary = TempDir::new().expect("attempt directory must be created");
        let orphan = temporary.path().join("attempt-0001.bin");
        fs::write(orphan.as_path(), b"orphaned").expect("orphaned attempt must be written");
        let journal = AttemptJournal::capture(temporary.path(), b"next", &serde_json::json!({}))
            .expect("next attempt must be captured");
        assert_eq!(
            (
                journal.sequence,
                fs::read(orphan).expect("orphaned evidence must remain readable"),
            ),
            (2, b"orphaned".to_vec()),
            "attempt capture reused and overwrote an interrupted sequence"
        );
    }
}
