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

use super::monochrome::color_detected;
use super::{
    BorderDetector, ImageSource, Progress, RecallJudge, RecallReview, Renderer, TextJudge,
    TextReview, TextReviewGate, compile_image_prompt,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum RenderRejection {
    Color(String),
    Borderless(String),
    Topology(String),
    Ocr(String),
    RecallText(String),
    Other(String),
}

impl RenderRejection {
    fn color(reason: &str) -> Self {
        Self::Color(String::from(reason))
    }

    fn borderless(reason: &str) -> Self {
        Self::Borderless(String::from(reason))
    }

    fn topology_scored(reason: String) -> Self {
        Self::Topology(reason)
    }

    fn ocr(reason: String) -> Self {
        Self::Ocr(reason)
    }

    fn recall_text(reason: String) -> Self {
        Self::RecallText(reason)
    }

    fn other(reason: String) -> Self {
        Self::Other(reason)
    }

    fn reason(&self) -> &str {
        match self {
            Self::Color(reason)
            | Self::Borderless(reason)
            | Self::Topology(reason)
            | Self::Ocr(reason)
            | Self::RecallText(reason)
            | Self::Other(reason) => reason.as_str(),
        }
    }

    fn category(&self) -> &'static str {
        match self {
            Self::Color(_) => "color",
            Self::Borderless(_) => "borderless",
            Self::Topology(_) => "topology",
            Self::Ocr(_) => "ocr",
            Self::RecallText(_) => "recall_text",
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
pub struct MangaRenderer<J> {
    client: Rc<dyn ImageSource>,
    retries: usize,
    text: Option<Rc<dyn TextJudge>>,
    judge: J,
    border: BorderDetector,
    attempts: Option<PathBuf>,
    salvage: bool,
}

impl<J> MangaRenderer<J> {
    /// Create one validating manga renderer.
    pub fn new<C>(client: C, retries: usize, judge: J, border: BorderDetector) -> Self
    where
        C: ImageSource + 'static,
    {
        Self {
            client: Rc::new(client),
            retries,
            text: None,
            judge,
            border,
            attempts: None,
            salvage: false,
        }
    }

    /// Preserve every raw production image attempt and its validation verdict.
    pub fn with_attempt_archive(mut self, directory: PathBuf) -> Self {
        self.attempts = Some(directory);
        self
    }

    /// Ship the best non-blocked archived attempt when this attempt is the last.
    pub fn with_salvage(mut self) -> Self {
        self.salvage = true;
        self
    }

    /// Apply one literal-writing gate before semantic recall review.
    pub fn with_text_judge<T>(mut self, text: T) -> Self
    where
        T: TextJudge + 'static,
    {
        self.text = Some(Rc::new(text));
        self
    }
}

impl<J> std::fmt::Debug for MangaRenderer<J>
where
    J: std::fmt::Debug,
{
    /// Render one stable debug view for test diagnostics.
    fn fmt(&self, item: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = item.debug_struct("MangaRenderer");
        debug
            .field("client", &"ImageSource")
            .field("retries", &self.retries)
            .field("text", &self.text.as_ref().map(|_| "TextJudge"))
            .field("judge", &self.judge)
            .field("border", &self.border);
        debug.field("attempts", &self.attempts);
        debug.finish()
    }
}

impl<J> Renderer for MangaRenderer<J>
where
    J: RecallJudge,
{
    /// Return one rendered image for the scene.
    fn render(&self, scene: &Value, progress: &mut dyn Progress) -> Result<DynamicImage> {
        let panels = scene
            .get("manga_panel")
            .and_then(|root| root.get("panels"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let prompt = compile_image_prompt(scene)?;
        let mut rejection = RenderRejection::other(String::new());
        for attempt in 0..self.retries {
            let recovered = self
                .attempts
                .as_deref()
                .map(|directory| AttemptJournal::resume_review(directory, scene, prompt.as_str()))
                .transpose()?
                .flatten();
            let (mut journal, bytes) = match recovered {
                Some((journal, bytes)) => (Some(journal), bytes),
                None => {
                    let mut journal = self
                        .attempts
                        .as_deref()
                        .map(|directory| AttemptJournal::start(directory, scene, prompt.as_str()))
                        .transpose()?;
                    let bytes = match self.client.image(prompt.as_str()) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            let reason = error.to_string();
                            record_attempt(journal.as_ref(), "error", "provider", reason.as_str())?;
                            return self.rescued(error);
                        }
                    };
                    if let Some(journal) = journal.as_mut()
                        && let Err(error) = journal.capture_image(bytes.as_slice())
                    {
                        let reason = error.to_string();
                        let _ = journal.record("error", "archive", reason.as_str());
                        return self.rescued(error);
                    }
                    (journal, bytes)
                }
            };
            let decoded = image::load_from_memory(bytes.as_slice());
            let image = match decoded {
                Ok(image) => image,
                Err(error) => {
                    record_attempt(
                        journal.as_ref(),
                        "error",
                        "transport_or_decode",
                        error.to_string().as_str(),
                    )?;
                    return self.rescued(error.into());
                }
            };
            if color_detected(&image) {
                rejection = RenderRejection::color("Color detected");
                record_attempt(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            let gray = image.into_luma8();
            let leveled =
                (!self.border.borders(&gray).is_empty()).then(|| self.border.repaired(&gray));
            let measured = leveled.as_ref().unwrap_or(&gray);
            let mut penalties = Scorecard::default();
            let mut reasons = Vec::new();
            if has_active_layout(scene) {
                if !registry_topology_matches(&self.border, scene, measured, panels) {
                    let (penalty, finding) =
                        topology_penalty(&self.border, scene, measured, panels);
                    penalties.topology = penalty;
                    reasons.push(finding);
                }
            } else if requires_gutter(scene, panels) && !self.border.gutter(measured) {
                penalties.topology = TOPOLOGY_PENALTY;
                reasons.push(String::from("no white gutter separates the panels"));
            }
            let mut ocr_gate = false;
            if let Some(text) = self.text.as_ref() {
                let archived = journal
                    .as_ref()
                    .map(AttemptJournal::text_review)
                    .transpose()?
                    .flatten();
                let review = match archived {
                    Some(review) => review,
                    None => match text.review(bytes.as_slice(), &gray) {
                        Ok(review) => review,
                        Err(error) => {
                            let reason = error.to_string();
                            record_attempt(
                                journal.as_ref(),
                                "error",
                                text.gate().error_category(),
                                reason.as_str(),
                            )?;
                            return self.rescued(error);
                        }
                    },
                };
                if let Some(journal) = journal.as_mut()
                    && journal.text.is_none()
                    && let Err(error) = journal.capture_text(&review)
                {
                    let reason = error.to_string();
                    let _ = journal.record("error", "archive", reason.as_str());
                    return self.rescued(error);
                }
                penalties.text = review.penalty();
                if let Some(reason) = review.rejection() {
                    ocr_gate = review.gate() == TextReviewGate::Ocr;
                    reasons.push(match review.gate() {
                        TextReviewGate::Ocr => reason,
                        TextReviewGate::LlmJudge => format!("Text judge found writing: {reason}"),
                    });
                }
            }
            let review = match self.judge.review(scene, bytes.as_slice()) {
                Ok(review) => review,
                Err(error) => {
                    let reason = error.to_string();
                    record_attempt(journal.as_ref(), "error", "recall_judge", reason.as_str())?;
                    return self.rescued(error);
                }
            };
            if let Some(journal) = journal.as_mut()
                && let Err(error) = journal.capture_recall(&review)
            {
                let reason = error.to_string();
                let _ = journal.record("error", "archive", reason.as_str());
                return self.rescued(error);
            }
            if let Some(reason) = review.rejection() {
                rejection =
                    RenderRejection::recall_text(format!("Recall judge rejected image: {reason}"));
                record_attempt_scored(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                    &penalties,
                    true,
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            if review.frame_borderless() && self.border.perimeter_inked(&gray) {
                rejection = RenderRejection::borderless(
                    "No panel frame anywhere and ink reaches every page edge",
                );
                record_attempt_scored(
                    journal.as_ref(),
                    "rejected",
                    rejection.category(),
                    rejection.reason(),
                    &penalties,
                    true,
                )?;
                progress.retry("Rendering manga", attempt + 1, rejection.reason());
                continue;
            }
            if review.frame_torn() {
                penalties.topology = penalties.topology.max(TOPOLOGY_PENALTY_FLOOR);
                reasons.push(String::from("generation artifact tears the panel frame"));
            } else if review.frame_breakout() && penalties.topology >= TOPOLOGY_PENALTY_FLOOR {
                penalties.topology = BREAKOUT_TOPOLOGY_PENALTY;
                reasons.push(String::from("deliberate panel breakout keeps the frame"));
            }
            penalties.fidelity = review.fidelity_penalty();
            if let Some(reason) = review.scene_fidelity_rejection() {
                reasons.push(format!("Scene fidelity judge: {reason}"));
            }
            if self.text.is_some() {
                penalties.literal = review.literal_penalty();
                if let Some(reason) = review.literal_rejection() {
                    reasons.push(format!("Literal writing: {reason}"));
                }
            }
            if penalties.topology < TOPOLOGY_PENALTY_FLOOR {
                if !review.inspections_complete() {
                    let reason = String::from(
                        "Recall review ended without dedicated fidelity and scale-aware literal inspection",
                    );
                    record_attempt(journal.as_ref(), "error", "recall_judge", reason.as_str())?;
                    return self.rescued(anyhow!(reason));
                }
                record_attempt_scored(
                    journal.as_ref(),
                    "accepted",
                    "accepted",
                    "",
                    &penalties,
                    false,
                )?;
                return Ok(DynamicImage::ImageLuma8(gray));
            }
            rejection = penalties.rejection(ocr_gate, reasons.as_slice());
            record_attempt_scored(
                journal.as_ref(),
                "rejected",
                rejection.category(),
                rejection.reason(),
                &penalties,
                false,
            )?;
            progress.retry("Rendering manga", attempt + 1, rejection.reason());
        }
        self.rescued(MangaRenderRejection::new(self.retries, rejection).into())
    }
}

impl<J> MangaRenderer<J> {
    /// Resolve one terminal failure of the final attempt: ship the best
    /// non-blocked archived frame when salvage is armed, keep the failure
    /// otherwise.
    ///
    /// Every terminal path funnels through here — an exhausted retry loop, a
    /// provider or judge error, an exhausted picture-request budget surfacing
    /// as a provider refusal — so a card can only fail when its archive is
    /// empty or every archived frame was blocked.
    fn rescued(&self, error: anyhow::Error) -> Result<DynamicImage> {
        if self.salvage
            && let Some(directory) = self.attempts.as_deref()
            && let Some(bytes) = salvaged_best(directory)
            && let Ok(image) = image::load_from_memory(bytes.as_slice())
        {
            return Ok(DynamicImage::ImageLuma8(image.into_luma8()));
        }
        Err(error)
    }
}

const TOPOLOGY_PENALTY: u32 = 40;
const TOPOLOGY_PENALTY_FLOOR: u32 = 24;
const TOPOLOGY_DEVIATION_STEP: u32 = 8;
const BREAKOUT_TOPOLOGY_PENALTY: u32 = 8;

/// Weighted per-category quality penalties for one judged image attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Scorecard {
    topology: u32,
    text: u32,
    fidelity: u32,
    literal: u32,
}

impl Scorecard {
    fn total(&self) -> u32 {
        self.topology
            .saturating_add(self.text)
            .saturating_add(self.fidelity)
            .saturating_add(self.literal)
    }

    fn score(&self) -> u32 {
        100u32.saturating_sub(self.total())
    }

    fn rejection(&self, ocr_gate: bool, reasons: &[String]) -> RenderRejection {
        let semantic =
            self.fidelity
                .saturating_add(self.literal)
                .max(if ocr_gate { 0 } else { self.text });
        let reason = format!("quality score {}/100: {}", self.score(), reasons.join("; "));
        if self.topology >= semantic && self.topology >= self.text {
            RenderRejection::topology_scored(reason)
        } else if ocr_gate && self.text >= semantic {
            RenderRejection::ocr(reason)
        } else {
            RenderRejection::recall_text(reason)
        }
    }

    fn encoded(&self, blocker: bool) -> Value {
        json!({
            "score": if blocker { 0 } else { self.score() },
            "blocker": blocker,
            "penalties": {
                "topology": self.topology,
                "text": self.text,
                "fidelity": self.fidelity,
                "literal": self.literal
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AttemptJournal {
    sequence: usize,
    image: Option<String>,
    text: Option<String>,
    recall: Option<String>,
    scene: String,
    verdict: PathBuf,
}

impl AttemptJournal {
    fn start(directory: &Path, scene: &Value, prompt: &str) -> Result<Self> {
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
        let scene_name = format!("attempt-{sequence:04}.scene.json");
        let prompt_name = format!("attempt-{sequence:04}.prompt.txt");
        fs::write(
            directory.join(scene_name.as_str()),
            serde_json::to_vec_pretty(scene)?,
        )?;
        fs::write(directory.join(prompt_name), prompt)?;
        let journal = Self {
            sequence,
            image: None,
            text: None,
            recall: None,
            scene: scene_name,
            verdict: directory.join(format!("attempt-{sequence:04}.json")),
        };
        journal.record("pending", "pending", "")?;
        Ok(journal)
    }

    fn resume_review(
        directory: &Path,
        scene: &Value,
        prompt: &str,
    ) -> Result<Option<(Self, Vec<u8>)>> {
        if !directory.is_dir() {
            return Ok(None);
        }
        let latest = fs::read_dir(directory)?
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let sequence = name
                    .strip_prefix("attempt-")?
                    .strip_suffix(".json")?
                    .parse::<usize>()
                    .ok()?;
                Some((sequence, entry.path()))
            })
            .max_by_key(|(sequence, _)| *sequence);
        let Some((sequence, verdict)) = latest else {
            return Ok(None);
        };
        let value = serde_json::from_slice::<Value>(fs::read(&verdict)?.as_slice())?;
        let status = value.get("status").and_then(Value::as_str);
        let category = value.get("category").and_then(Value::as_str);
        let retryable_error = status == Some("error")
            && matches!(category, Some("ocr" | "text_judge" | "recall_judge"));
        let retryable_pending = status == Some("pending")
            && category == Some("pending")
            && value.get("image").and_then(Value::as_str).is_some();
        if !(retryable_error || retryable_pending)
            || !value.get("recall").is_none_or(Value::is_null)
        {
            return Ok(None);
        }
        let image = attempt_member(&value, "image")?;
        let scene_name = attempt_member(&value, "scene")?;
        let prompt_name = attempt_member(&value, "prompt")?;
        let text = optional_attempt_member(&value, "text")?;
        let archived_scene = serde_json::from_slice::<Value>(
            fs::read(directory.join(scene_name.as_str()))?.as_slice(),
        )?;
        let archived_prompt = fs::read_to_string(directory.join(prompt_name))?;
        if archived_scene != *scene || archived_prompt != prompt {
            return Ok(None);
        }
        let bytes = fs::read(directory.join(image.as_str()))?;
        let journal = Self {
            sequence,
            image: Some(image),
            text,
            recall: None,
            scene: scene_name,
            verdict,
        };
        journal.record("pending", "pending", "")?;
        Ok(Some((journal, bytes)))
    }

    fn capture_image(&mut self, bytes: &[u8]) -> Result<()> {
        let extension = match image::guess_format(bytes).ok() {
            Some(image::ImageFormat::Png) => "png",
            Some(image::ImageFormat::Jpeg) => "jpg",
            Some(image::ImageFormat::WebP) => "webp",
            Some(image::ImageFormat::Gif) => "gif",
            _ => "bin",
        };
        let image = format!("attempt-{:04}.{extension}", self.sequence);
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        fs::write(directory.join(image.as_str()), bytes)?;
        self.image = Some(image);
        self.record("pending", "pending", "")
    }

    fn capture_recall(&mut self, review: &RecallReview) -> Result<()> {
        let recall = format!("attempt-{:04}.recall.json", self.sequence);
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(staged.as_file_mut(), review)?;
        staged.as_file().sync_all()?;
        staged.persist(directory.join(recall.as_str()))?;
        self.recall = Some(recall);
        self.record("pending", "pending", "")
    }

    fn capture_text(&mut self, review: &TextReview) -> Result<()> {
        let text = format!("attempt-{:04}.text.json", self.sequence);
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(staged.as_file_mut(), review)?;
        staged.as_file().sync_all()?;
        staged.persist(directory.join(text.as_str()))?;
        self.text = Some(text);
        self.record("pending", "pending", "")
    }

    fn text_review(&self) -> Result<Option<TextReview>> {
        let Some(text) = self.text.as_ref() else {
            return Ok(None);
        };
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        Ok(Some(serde_json::from_slice::<TextReview>(
            fs::read(directory.join(text))?.as_slice(),
        )?))
    }

    fn record(&self, status: &str, category: &str, reason: &str) -> Result<()> {
        self.write(status, category, reason, None)
    }

    fn record_scored(
        &self,
        status: &str,
        category: &str,
        reason: &str,
        scorecard: &Scorecard,
        blocker: bool,
    ) -> Result<()> {
        self.write(status, category, reason, Some(scorecard.encoded(blocker)))
    }

    fn write(
        &self,
        status: &str,
        category: &str,
        reason: &str,
        scorecard: Option<Value>,
    ) -> Result<()> {
        let directory = self
            .verdict
            .parent()
            .ok_or_else(|| anyhow!("attempt verdict has no parent directory"))?;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        let mut verdict = json!({
            "sequence": self.sequence,
            "image": self.image,
            "text": self.text,
            "recall": self.recall,
            "scene": self.scene,
            "prompt": format!("attempt-{:04}.prompt.txt", self.sequence),
            "status": status,
            "category": category,
            "reason": reason
        });
        if let (Some(fields), Some(scored)) = (verdict.as_object_mut(), scorecard)
            && let Some(scored) = scored.as_object()
        {
            for (key, value) in scored {
                fields.insert(key.clone(), value.clone());
            }
        }
        serde_json::to_writer_pretty(staged.as_file_mut(), &verdict)?;
        staged.as_file().sync_all()?;
        staged.persist(self.verdict.as_path())?;
        Ok(())
    }
}

fn optional_attempt_member(value: &Value, key: &str) -> Result<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => attempt_member(value, key).map(Some),
    }
}

fn attempt_member(value: &Value, key: &str) -> Result<String> {
    let name = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("attempt verdict has no {key} member"))?;
    let path = Path::new(name);
    if path.file_name().and_then(|member| member.to_str()) != Some(name) {
        return Err(anyhow!("attempt verdict {key} member is not a filename"));
    }
    Ok(String::from(name))
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

fn record_attempt_scored(
    journal: Option<&AttemptJournal>,
    status: &str,
    category: &str,
    reason: &str,
    scorecard: &Scorecard,
    blocker: bool,
) -> Result<()> {
    if let Some(journal) = journal {
        journal.record_scored(status, category, reason, scorecard, blocker)?;
    }
    Ok(())
}

/// Select the best non-blocked archived attempt for terminal salvage.
///
/// Returns the archived image of the highest-scoring rejected attempt whose
/// verdict carries a scorecard and no blocker, marking its verdict as
/// salvaged. Blocked attempts — color or answer leakage — never qualify.
fn salvaged_best(directory: &Path) -> Option<Vec<u8>> {
    let entries = fs::read_dir(directory).ok()?;
    let mut best: Option<(u64, usize, PathBuf, Value)> = None;
    for entry in entries.filter_map(std::result::Result::ok) {
        let name = entry.file_name().into_string().ok()?;
        let Some(sequence) = name
            .strip_prefix("attempt-")
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|digits| digits.parse::<usize>().ok())
        else {
            continue;
        };
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(verdict) = serde_json::from_slice::<Value>(bytes.as_slice()) else {
            continue;
        };
        if verdict.get("status").and_then(Value::as_str) != Some("rejected")
            || verdict.get("blocker").and_then(Value::as_bool) != Some(false)
        {
            continue;
        }
        let Some(score) = verdict.get("score").and_then(Value::as_u64) else {
            continue;
        };
        if verdict.get("image").and_then(Value::as_str).is_none() {
            continue;
        }
        let replace = best
            .as_ref()
            .is_none_or(|(top, at, _, _)| (score, sequence) > (*top, *at));
        if replace {
            best = Some((score, sequence, entry.path(), verdict));
        }
    }
    let (_, _, verdict_path, verdict) = best?;
    let image_name = verdict.get("image").and_then(Value::as_str)?;
    let image = fs::read(directory.join(image_name)).ok()?;
    let mut salvaged = verdict;
    if let Some(fields) = salvaged.as_object_mut() {
        fields.insert(
            String::from("status"),
            Value::String(String::from("salvaged")),
        );
        let _ = serde_json::to_vec_pretty(&salvaged).map(|bytes| fs::write(verdict_path, bytes));
    }
    Some(image)
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

/// Grade one failed panel-topology check by its distance from the registered
/// layout and describe what the page actually drew.
///
/// The registered matchers stay the only source of "correct"; this measure only
/// orders failures so retries and the salvage pass can prefer the closest frame.
/// The deviation counts missing or surplus panel regions plus declared panels
/// whose centres share one region, and each step adds `TOPOLOGY_DEVIATION_STEP`
/// above `TOPOLOGY_PENALTY_FLOOR`, saturating at the flat `TOPOLOGY_PENALTY`.
/// The returned finding names the concrete mismatch instead of a generic
/// verdict, so the user can read what went wrong from the attempt row.
fn topology_penalty(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    panels: usize,
) -> (u32, String) {
    let unmeasured = String::from("the planned panel grid was not detected");
    let Some(declared) = scene
        .pointer("/manga_panel/panels")
        .and_then(Value::as_array)
        .filter(|declared| declared.len() == panels)
    else {
        return (TOPOLOGY_PENALTY, unmeasured);
    };
    let Some(centers) = panel_centers(scene, declared, image) else {
        return (TOPOLOGY_PENALTY, unmeasured);
    };
    let (regions, labels) = border.region_measure(image, centers.as_slice());
    let distinct = labels
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .len();
    let deviation = regions
        .abs_diff(panels)
        .saturating_add(panels.saturating_sub(distinct));
    let finding = if regions != panels {
        let regions_word = if regions == 1 { "region" } else { "regions" };
        let panels_word = if panels == 1 { "panel" } else { "panels" };
        format!("found {regions} panel {regions_word} for {panels} planned {panels_word}")
    } else if distinct < panels {
        String::from("planned panels share one drawn region")
    } else if panels == 1 {
        String::from("an internal gutter splits the single planned panel")
    } else {
        String::from("panel geometry misses the planned layout")
    };
    let deviation = u32::try_from(deviation).unwrap_or(u32::MAX);
    let penalty = TOPOLOGY_PENALTY_FLOOR
        .saturating_add(deviation.saturating_mul(TOPOLOGY_DEVIATION_STEP))
        .min(TOPOLOGY_PENALTY);
    (penalty, finding)
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
    }) || none_slanted_topology_matches(border, scene, panels, image, expected)
        || staggered_grid_layout(scene)
            && staggered_grid_topology_matches(border, scene, panels, image, expected)
}

fn none_slanted_topology_matches(
    border: &BorderDetector,
    scene: &Value,
    panels: &[Value],
    image: &image::GrayImage,
    expected: usize,
) -> bool {
    if scene
        .pointer("/manga_panel/page_design/special_device/kind")
        .and_then(Value::as_str)
        != Some("none")
        || staggered_grid_layout(scene)
    {
        return false;
    }
    let Some((regions, assignments)) = registry_assignments(border, scene, panels, image) else {
        return false;
    };
    let distinct = assignments.iter().copied().collect::<BTreeSet<_>>();
    if regions != expected || distinct.len() != expected {
        return false;
    }
    let Some(proofs) = slant_proofs(scene, panels) else {
        return false;
    };
    !proofs.is_empty()
        && (registry_slants_match(
            border,
            scene,
            image,
            assignments.as_slice(),
            proofs.as_slice(),
        ) || mirrored_diagonal_strip_matches(
            border,
            scene,
            image,
            assignments.as_slice(),
            proofs.as_slice(),
        ))
}

fn mirrored_diagonal_strip_matches(
    border: &BorderDetector,
    scene: &Value,
    image: &image::GrayImage,
    assignments: &[usize],
    proofs: &[SlantProof],
) -> bool {
    if scene
        .pointer("/manga_panel/panel_layout/active_layout/template_id")
        .and_then(Value::as_str)
        != Some("diagonal-strip-3-v1")
        || proofs.len() != 2
        || proofs.iter().any(|proof| proof.axis != SlantAxis::Vertical)
    {
        return false;
    }
    let mirrored = proofs
        .iter()
        .map(|proof| SlantProof {
            direction: proof.direction.reverse(),
            ..*proof
        })
        .collect::<Vec<_>>();
    registry_slants_match(border, scene, image, assignments, mirrored.as_slice())
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
                RenderRejection::color("color"),
                RenderRejection::topology_scored(String::from("topology")),
                RenderRejection::ocr(String::from("ocr")),
                RenderRejection::recall_text(String::from("recall text")),
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
                RenderRejection::color("color"),
                RenderRejection::topology_scored(String::from("topology")),
                RenderRejection::ocr(String::from("ocr")),
                RenderRejection::recall_text(String::from("recall text")),
                RenderRejection::other(String::from("other")),
            ]
            .map(|rejection| rejection.category()),
            ["color", "topology", "ocr", "recall_text", "other"],
            "typed manga errors collapse distinct validation gates"
        );
    }

    /// Typed manga errors preserve the renderer's established terminal message.
    #[test]
    fn manga_render_rejection_display_remains_unchanged() {
        assert_eq!(
            MangaRenderRejection::new(
                3,
                RenderRejection::topology_scored(String::from(
                    "Registered panel topology was not detected"
                )),
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
        let mut journal =
            AttemptJournal::start(temporary.path(), &serde_json::json!({}), "compiled prompt")
                .expect("next attempt must be reserved");
        journal
            .capture_image(b"next")
            .expect("next attempt image must be captured");
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
