//! Registry-driven selection and materialization for production manga layouts.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::generation::prompts::device_registry;

const REGISTRY_SOURCE: &str = include_str!("../../assets/layout_registry_v2.json");
const LEFT_TO_RIGHT: &str = "left_to_right_top_to_bottom";
const RIGHT_TO_LEFT: &str = "right_to_left_top_to_bottom";
const PANEL_MIN: i64 = 16;
const PANEL_MAX: i64 = 1008;
const MAX_DEVICE_GUTTER: i64 = 64;
const FEATURE_FIELDS: [&str; 16] = [
    "semantic_beat_count",
    "semantic_relation",
    "coverage_audit",
    "panel_count",
    "panel_relation",
    "panel_emphasis",
    "decomposition_mode",
    "motion_vector",
    "intensity",
    "spatial_relation",
    "transition_type",
    "reading_direction",
    "literal_anchor",
    "camera_arc",
    "shots",
    "selection_logic",
];
const DECOMPOSITION_MODES: [&str; 9] = [
    "single_tableau",
    "one_to_one",
    "context_action_detail",
    "setup_detail_payoff",
    "cause_reaction_detail",
    "wide_detail_pair",
    "aspect_montage",
    "action_phases",
    "contrast_views",
];
const SHOT_ROLES: [&str; 6] = [
    "establishing",
    "action",
    "detail",
    "reaction",
    "payoff",
    "aspect",
];
const COVERAGE_VERDICTS: [&str; 3] = ["insufficient", "selected", "redundant_or_unsupported"];
const CAMERA_ARCS: [&str; 9] = [
    "single_view",
    "hold",
    "push_in",
    "pull_back_reveal",
    "wide_detail_return",
    "crosscut",
    "action_acceleration",
    "contrast",
    "motivated_mixed",
];
const SHOT_SCALES: [&str; 7] = [
    "extreme_wide",
    "wide",
    "full",
    "medium",
    "medium_close",
    "close",
    "extreme_close",
];
const VIEWPOINTS: [&str; 4] = [
    "objective",
    "over_the_shoulder",
    "point_of_view",
    "subjective",
];
const FRAMINGS: [&str; 6] = [
    "environment",
    "single",
    "two_shot",
    "group",
    "insert",
    "cutaway",
];
const CAMERA_ANGLES: [&str; 5] = ["eye_level", "high", "low", "overhead", "dutch"];
const DEPTH_PLANS: [&str; 4] = ["deep", "layered", "shallow", "flat"];
const TRANSITION_TRIGGERS: [&str; 10] = [
    "scene_open",
    "new_action",
    "attention_shift",
    "detail_reveal",
    "emotion_change",
    "subject_handoff",
    "spatial_reorientation",
    "temporal_change",
    "contrast",
    "payoff",
];
const TEXTLESS_MOTION: &str = "Use only abstract speed lines and blur; never render letters, numbers, symbols, onomatopoeia, or letterlike motion marks.";
const TEXTLESS_SOUND_EFFECTS: &str = "Absolutely no sound-effect glyphs, including Japanese onomatopoeia, stylized symbols, or pseudo-writing.";
const TEXTLESS_VISIBLE_WRITING: &str = "No visible letters, words, numerals, state labels, interface marks, symbols, or pseudo-writing anywhere; express every device state only through unlabeled physical position, light, shape, or motion.";
const TEXTLESS_MARKS: &str = "No logos, brand marks, emblems, badges, icons, interface symbols, or decorative pseudo-writing on any object.";
const TEXTLESS_LABELS: &str = "No signs, captions, labels, legends, state names, icons, or interface text; all text-bearing surfaces stay featureless, hidden, or turned away.";
const OUTER_WHITE_BAND: &str = "Reserve a continuous content-free pure-white 16px band on all four canvas edges; no ink, screentone, subject, effect, panel frame, or open-frame artwork may touch or cross it.";
const FEATURE_PROMPT: &str = include_str!("../../assets/layout_features_prompt.txt");
const DEVICE_KINDS: [&str; 7] = [
    "none",
    "crossing",
    "overlap",
    "inset",
    "open_frame",
    "master_view",
    "diagonal_release",
];
const SOFT_FEATURE_FIELDS: [&str; 4] = [
    "motion_vector",
    "intensity",
    "spatial_relation",
    "transition_type",
];

/// Owns one decoded and validated version-two layout registry.
#[derive(Clone, Debug)]
pub(crate) struct LayoutRegistry {
    value: Value,
}

/// Owns one validated narrative and cinematic feature vector.
#[derive(Clone, Debug)]
pub(crate) struct SceneFeatures {
    value: Value,
    lenient: bool,
}

/// Owns the deterministic hard-filter result used for local layout ranking.
#[derive(Clone, Debug)]
pub(crate) struct EligibleLayouts {
    features: SceneFeatures,
    templates: Vec<Value>,
    retry_template: Option<Value>,
    contract: Value,
    policy: Value,
    fallback: Option<Value>,
}

/// Owns one validated, unpadded ranking over eligible canonical layouts.
#[derive(Clone, Debug)]
pub(crate) struct LayoutRanking {
    features: SceneFeatures,
    templates: Vec<Value>,
    eligible_count: usize,
    primary_count: usize,
    candidates: Vec<Value>,
    fallback: Option<Value>,
}

/// Owns the chosen canonical template and the exact selection record sent to the composer.
#[derive(Clone, Debug)]
pub(crate) struct LayoutSelection {
    summary: Value,
    template: Value,
}

/// Identifies a valid narrative tuple that the automatic registry cannot render.
#[derive(Clone, Debug)]
pub(crate) struct UnsupportedNarrativeTuple {
    features: Value,
}

/// Owns one validated rectangular device materialization region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeviceBounds {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

impl DeviceBounds {
    /// Decode one positive rectangular region from a JSON bounds object.
    fn decode(value: &Map<String, Value>) -> Result<Self> {
        let bounds = Self {
            x: i64_field(value, "x")?,
            y: i64_field(value, "y")?,
            width: i64_field(value, "width")?,
            height: i64_field(value, "height")?,
        };
        if bounds.width <= 0 || bounds.height <= 0 {
            bail!("device bounds must have positive dimensions");
        }
        bounds.right()?;
        bounds.bottom()?;
        Ok(bounds)
    }

    /// Return the exclusive right edge while rejecting arithmetic overflow.
    fn right(self) -> Result<i64> {
        self.x
            .checked_add(self.width)
            .ok_or_else(|| anyhow!("device bounds horizontal edge overflow"))
    }

    /// Return the exclusive bottom edge while rejecting arithmetic overflow.
    fn bottom(self) -> Result<i64> {
        self.y
            .checked_add(self.height)
            .ok_or_else(|| anyhow!("device bounds vertical edge overflow"))
    }

    /// Return whether two regions overlap along their vertical axes.
    fn vertical_overlap(self, other: Self) -> bool {
        self.y < other.y.saturating_add(other.height)
            && other.y < self.y.saturating_add(self.height)
    }

    /// Return whether two regions overlap along their horizontal axes.
    fn horizontal_overlap(self, other: Self) -> bool {
        self.x < other.x.saturating_add(other.width) && other.x < self.x.saturating_add(self.width)
    }

    /// Return whether two regions share positive rectangular area.
    fn intersects(self, other: Self) -> Result<bool> {
        Ok(self.x < other.right()?
            && other.x < self.right()?
            && self.y < other.bottom()?
            && other.y < self.bottom()?)
    }

    /// Render these bounds through the production JSON wire.
    fn json(self) -> Value {
        json!({"x": self.x, "y": self.y, "width": self.width, "height": self.height})
    }
}

impl Display for UnsupportedNarrativeTuple {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "no automatic left-to-right layout satisfies narrative tuple {}",
            self.features
        )
    }
}

impl Error for UnsupportedNarrativeTuple {}

impl LayoutRegistry {
    /// Decode and validate the embedded production layout registry.
    pub(crate) fn embedded() -> Result<Self> {
        Self::decode(REGISTRY_SOURCE)
    }

    /// Decode and validate one version-two layout registry document.
    pub(crate) fn decode(source: &str) -> Result<Self> {
        let value =
            serde_json::from_str::<Value>(source).context("cannot decode manga layout registry")?;
        validate_registry(&value)?;
        Ok(Self { value })
    }

    /// Build the strict JSON schema used by the independent narrative and shot pass.
    pub(crate) fn feature_schema(&self) -> Result<Value> {
        let contract = object_field(root_object(&self.value)?, "selection_contract")?;
        let coverage = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "panel_count": {"type": "integer", "minimum": 1, "maximum": 4},
                "added_view": {"type": "string"},
                "source_support": {"type": "string"},
                "verdict": {"type": "string", "enum": COVERAGE_VERDICTS},
                "reason": {"type": "string"}
            },
            "required": ["panel_count", "added_view", "source_support", "verdict", "reason"]
        });
        let continuity = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "axis_mode": {"type": "string", "enum": ["not_applicable", "preserve", "reestablish", "deliberate_break"]},
                "axis": {"type": "string"},
                "screen_direction": {"type": "string", "enum": ["not_applicable", "stationary", "left_to_right", "right_to_left", "toward_camera", "away_from_camera", "converging", "diverging"]},
                "eyeline_policy": {"type": "string", "enum": ["not_applicable", "matched", "deliberately_broken"]}
            },
            "required": ["axis_mode", "axis", "screen_direction", "eyeline_policy"]
        });
        let camera_arc = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "strategy": {"type": "string", "enum": CAMERA_ARCS},
                "progression": {"type": "string"},
                "motivation": {"type": "string"},
                "continuity": continuity
            },
            "required": ["strategy", "progression", "motivation", "continuity"]
        });
        let shot = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": {"type": "string", "enum": ["s1", "s2", "s3", "s4"]},
                "semantic_beat_index": {"type": "integer", "minimum": 1, "maximum": 4},
                "role": {"type": "string", "enum": SHOT_ROLES},
                "visible_anchor": {"type": "string"},
                "source_support": {"type": "string"},
                "shot_scale": {"type": "string", "enum": SHOT_SCALES},
                "viewpoint": {"type": "string", "enum": VIEWPOINTS},
                "viewpoint_anchor": {"type": "string"},
                "framing": {"type": "string", "enum": FRAMINGS},
                "angle": {"type": "string", "enum": CAMERA_ANGLES},
                "depth_plan": {"type": "string", "enum": DEPTH_PLANS},
                "camera_motivation": {"type": "string"},
                "information_gain": {"type": "string"},
                "transition_trigger": {"type": "string", "enum": TRANSITION_TRIGGERS}
            },
            "required": ["id", "semantic_beat_index", "role", "visible_anchor", "source_support", "shot_scale", "viewpoint", "viewpoint_anchor", "framing", "angle", "depth_plan", "camera_motivation", "information_gain", "transition_trigger"]
        });
        Ok(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "semantic_beat_count": {"type": "integer", "minimum": 1, "maximum": 4},
                "semantic_relation": {"type": "string", "enum": array_field(contract, "allowed_temporal_relations")?},
                "coverage_audit": {
                    "type": "array",
                    "minItems": 4,
                    "maxItems": 4,
                    "items": coverage
                },
                "panel_count": {"type": "integer", "minimum": 1, "maximum": 4},
                "panel_relation": {"type": "string", "enum": array_field(contract, "allowed_temporal_relations")?},
                "panel_emphasis": {"type": "string", "enum": array_field(contract, "allowed_emphasis_curves")?},
                "decomposition_mode": {"type": "string", "enum": DECOMPOSITION_MODES},
                "motion_vector": {"type": "string", "enum": array_field(contract, "allowed_motion_vectors")?},
                "intensity": {"type": "string", "enum": array_field(contract, "allowed_intensities")?},
                "spatial_relation": {"type": "string", "enum": array_field(contract, "allowed_spatial_relations")?},
                "transition_type": {"type": "string", "enum": array_field(contract, "allowed_transition_types")?},
                "reading_direction": {"type": "string", "enum": [LEFT_TO_RIGHT]},
                "literal_anchor": {"type": "string"},
                "camera_arc": camera_arc,
                "shots": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "items": shot
                },
                "selection_logic": {"type": "string"}
            },
            "required": FEATURE_FIELDS
        }))
    }

    /// Decode one strict flat feature response using this registry's closed enums.
    #[cfg(test)]
    pub(crate) fn decode_features(&self, raw: &str) -> Result<SceneFeatures> {
        self.decode_features_lenient(raw, false)
    }

    /// Decode one feature response, optionally relaxing camera-progression quality rules.
    ///
    /// The lenient final scene attempt keeps every structural rule but stops
    /// rejecting a plan for quality-only camera progression violations, so a
    /// card ships the best available plan instead of failing.
    pub(crate) fn decode_features_lenient(
        &self,
        raw: &str,
        lenient: bool,
    ) -> Result<SceneFeatures> {
        let mut value = serde_json::from_str::<Value>(raw)
            .context("layout feature extractor returned invalid JSON")?;
        normalize_semantic_relation(&mut value)?;
        normalize_coverage_audit(&mut value)?;
        normalize_shots(&mut value)?;
        normalize_camera_arc(&mut value)?;
        validate_features(&value, contract(&self.value)?, lenient)?;
        Ok(SceneFeatures { value, lenient })
    }

    /// Apply deterministic hard filters and return only automatic left-to-right templates.
    pub(crate) fn eligible(&self, features: &SceneFeatures) -> Result<EligibleLayouts> {
        validate_features(&features.value, contract(&self.value)?, features.lenient)?;
        let root = root_object(&self.value)?;
        let available = array_field(root, "templates")?;
        let mut templates = available
            .iter()
            .filter(|template| template_matches(template, &features.value))
            .cloned()
            .collect::<Vec<_>>();
        let fallback = if templates.is_empty() {
            let template = nearest_same_count_template(available, &features.value)?;
            let Some(template) = template else {
                return Err(UnsupportedNarrativeTuple {
                    features: features.value.clone(),
                }
                .into());
            };
            let fallback = json!({
                "kind": "nearest_same_panel_count",
                "requested": {
                    "panel_count": field_clone(root_object(&features.value)?, "panel_count")?,
                    "panel_relation": field_clone(root_object(&features.value)?, "panel_relation")?,
                    "panel_emphasis": field_clone(root_object(&features.value)?, "panel_emphasis")?
                },
                "fallback_template_id": template_id(template)?
            });
            templates.push(template.clone());
            Some(fallback)
        } else {
            None
        };
        let feature = root_object(&features.value)?;
        let rankable = templates
            .iter()
            .filter(|template| ranking_template_allowed(template, feature))
            .collect::<Vec<_>>();
        let retry_template = if rankable.len() == 1 && usize_field(feature, "panel_count")? > 1 {
            nearest_same_count_alternative(available, &features.value, rankable[0])?.cloned()
        } else {
            None
        };
        Ok(EligibleLayouts {
            features: features.clone(),
            templates,
            retry_template,
            contract: root
                .get("selection_contract")
                .cloned()
                .ok_or_else(|| anyhow!("layout registry lacks selection_contract"))?,
            policy: root
                .get("layout_policy")
                .and_then(Value::as_object)
                .and_then(|value| value.get("templates"))
                .cloned()
                .ok_or_else(|| anyhow!("layout registry lacks layout policy"))?,
            fallback,
        })
    }
}

impl EligibleLayouts {
    /// Rank the eligible layouts locally by exact soft fit and product policy.
    pub(crate) fn rank(&self) -> Result<LayoutRanking> {
        let policy = self
            .policy
            .as_object()
            .ok_or_else(|| anyhow!("layout policy must be an object"))?;
        let features = root_object(&self.features.value)?;
        let mut templates = self
            .templates
            .iter()
            .filter(|template| ranking_template_allowed(template, features))
            .cloned()
            .collect::<Vec<_>>();
        let mut ranked = templates
            .iter()
            .enumerate()
            .map(|(index, template)| {
                let score = soft_match_score(template, features)?;
                let id = template_id(template)?;
                let (priority, label) = layout_priority(policy, id)?;
                Ok((
                    score,
                    priority,
                    index,
                    template,
                    local_candidate(id, score, label),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let best_score = ranked
            .first()
            .map(|candidate| candidate.0)
            .ok_or_else(|| anyhow!("local layout ranking has no eligible template"))?;
        let dynamic_primary = ranked
            .iter()
            .any(|(score, _, _, template, _)| *score == best_score && dynamic_only(template));
        let maximum = usize_field(root_object(&self.contract)?, "maximum_ranked_candidates")?;
        let mut geometries = BTreeSet::new();
        let mut candidates = Vec::new();
        for (score, _, _, template, candidate) in &ranked {
            if *score == best_score
                && dynamic_only(template) == dynamic_primary
                && geometries.insert(geometry_signature(template)?)
            {
                candidates.push(candidate.clone());
                if candidates.len() == maximum {
                    break;
                }
            }
        }
        if candidates.is_empty() {
            bail!("local layout ranking has no distinct primary geometry");
        }
        let primary_count = candidates.len();
        for (_, _, _, template, candidate) in &ranked {
            if candidates.len() == maximum {
                break;
            }
            if dynamic_only(template) == dynamic_primary
                && geometries.insert(geometry_signature(template)?)
            {
                candidates.push(candidate.clone());
            }
        }
        if candidates.len() == 1 {
            for (_, _, _, template, candidate) in &ranked {
                if candidates.len() == maximum {
                    break;
                }
                if dynamic_only(template) != dynamic_primary
                    && geometries.insert(geometry_signature(template)?)
                {
                    candidates.push(candidate.clone());
                    break;
                }
            }
        }
        let eligible_count = templates.len();
        if let Some(template) = self
            .retry_template
            .as_ref()
            .filter(|template| ranking_template_allowed(template, features))
        {
            templates.push(template.clone());
        }
        if candidates.len() == 1 && candidates.len() < maximum && templates.len() > 1 {
            for template in &templates {
                let id = template_id(template)?;
                let geometry = geometry_signature(template)?;
                let duplicate =
                    candidate_geometry_slot(&candidates, &templates, &geometry)?.is_some();
                if !duplicate {
                    candidates.push(json!({
                        "template_id": id,
                        "adaptation": "exact",
                        "reason": "local deterministic safeguard restored one distinct retry geometry"
                    }));
                    break;
                }
            }
        }
        Ok(LayoutRanking {
            features: self.features.clone(),
            templates,
            eligible_count,
            primary_count,
            candidates,
            fallback: self.fallback.clone(),
        })
    }
}

impl LayoutRanking {
    /// Resolve one viable candidate while preserving required dynamic emphasis.
    pub(crate) fn select(&self, term: &str, attempt: u8) -> Result<LayoutSelection> {
        if term.trim().is_empty() || self.candidates.is_empty() {
            bail!("layout selection requires a nonempty term and ranking");
        }
        let primary = self
            .candidates
            .get(..self.primary_count)
            .filter(|candidates| !candidates.is_empty())
            .ok_or_else(|| anyhow!("layout ranking has an invalid primary candidate count"))?;
        let base = selection_slot(term, primary.len());
        let slot = (base + usize::from(attempt)) % self.candidates.len();
        let chosen = self
            .candidates
            .get(slot)
            .ok_or_else(|| anyhow!("deterministic layout slot leaves the ranking"))?;
        let template = ranked_template(chosen, &self.templates)?.clone();
        let id = template_id(&template)?;
        let eligible = self
            .templates
            .get(..self.eligible_count)
            .ok_or_else(|| anyhow!("layout ranking has an invalid eligible template count"))?
            .iter()
            .map(template_id)
            .collect::<Result<Vec<_>>>()?;
        let devices = device_candidates(&template)?;
        let mut summary = json!({
            "scene_features": self.features.value,
            "eligible_template_ids": eligible,
            "ranked_candidates": self.candidates,
            "chosen_template_id": id,
            "device_candidates": devices,
            "seed_source": term,
            "scene_attempt_index": attempt,
            "deterministic_slot": slot
        });
        if let Some(fallback) = &self.fallback {
            summary
                .as_object_mut()
                .ok_or_else(|| anyhow!("layout selection summary must be an object"))?
                .insert(String::from("layout_fallback"), fallback.clone());
        }
        Ok(LayoutSelection { summary, template })
    }
}

impl LayoutSelection {
    /// Return the exact selection JSON sent to the semantic scene composer.
    pub(crate) fn json(&self) -> &Value {
        &self.summary
    }

    /// Return the geometry-free chosen template card sent to the composer.
    pub(crate) fn composer_card(&self) -> Result<Value> {
        let root = root_object(&self.template)?;
        let devices = self
            .summary
            .get("device_candidates")
            .cloned()
            .ok_or_else(|| anyhow!("layout selection lacks device candidates"))?;
        Ok(json!({
            "template_id": field_clone(root, "template_id")?,
            "family": field_clone(root, "family")?,
            "variant": field_clone(root, "variant")?,
            "panel_count": field_clone(root, "panel_count")?,
            "reading_direction": field_clone(root, "reading_direction")?,
            "dominant_index": field_clone(root, "dominant_index")?,
            "reading_strategy": field_clone(root, "reading_strategy")?,
            "best_for": field_clone(root, "best_for")?,
            "avoid_when": field_clone(root, "avoid_when")?,
            "feature_profile": field_clone(root, "feature_profile")?,
            "device_candidates": devices,
            "risk": field_clone(root, "risk")?
        }))
    }
}

/// Build the four immutable values substituted into the independent feature prompt.
pub(crate) fn feature_prompt_data(language: &str, term: &str, sentence: &str) -> Result<Value> {
    if [language, term, sentence]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        bail!("layout feature prompt requires language, term, and sentence");
    }
    Ok(json!({
        "language": language,
        "term": term,
        "sentence": sentence,
        "reading_direction": LEFT_TO_RIGHT
    }))
}

/// Render the registry-blind narrative feature prompt from validated prompt data.
pub(crate) fn render_feature_prompt(data: &Value) -> Result<String> {
    let root = root_object(data)?;
    exact_keys(
        root,
        &["language", "term", "sentence", "reading_direction"],
        "layout feature prompt data",
    )?;
    let direction = string_field(root, "reading_direction")?;
    if direction != LEFT_TO_RIGHT {
        bail!("production layout selection requires left-to-right reading");
    }
    Ok(FEATURE_PROMPT
        .replace("{language}", nonempty_string_field(root, "language")?)
        .replace("{term}", nonempty_string_field(root, "term")?)
        .replace("{sentence}", nonempty_string_field(root, "sentence")?)
        .replace("{reading_direction}", direction))
}

/// Build the operational one-device catalog specialized to one canonical layout.
fn device_candidates(template: &Value) -> Result<Value> {
    let registry = serde_json::from_str::<Value>(device_registry())
        .context("cannot decode operational manga device registry")?;
    validate_device_registry(&registry)?;
    let layout = root_object(template)?;
    let family = nonempty_string_field(layout, "family")?;
    let panels = usize_field(layout, "panel_count")?;
    let compatible = string_values(
        array_field(layout, "compatible_devices")?,
        "compatible devices",
    )?;
    let constrained = layout.get("dynamic_only").and_then(Value::as_bool) == Some(true);
    let candidates = array_field(root_object(&registry)?, "devices")?
        .iter()
        .filter_map(|device| {
            let root = device.as_object()?;
            let id = root.get("device_id")?.as_str()?;
            let status = root.get("capability_status")?.as_str()?;
            let families = root.get("compatible_topology_families")?.as_array()?;
            let minimum = root.get("min_panels")?.as_u64()?;
            let maximum = root.get("max_panels")?.as_u64()?;
            let count = u64::try_from(panels).ok()?;
            (root.get("automatic_selection").and_then(Value::as_bool) == Some(true)
                && matches!(status, "qualified" | "proven")
                && (!constrained || compatible.contains(&id))
                && families.iter().any(|value| value.as_str() == Some(family))
                && (minimum..=maximum).contains(&count))
            .then_some(device)
        })
        .map(|device| device_candidate(device, template))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if !candidates.iter().any(|value| value["scene_kind"] == "none") {
        bail!("operational device catalog leaves layout without the none device");
    }
    Ok(Value::Array(candidates))
}

/// Project one registry device into a geometry-free model selection card.
fn device_candidate(device: &Value, template: &Value) -> Result<Option<Value>> {
    let root = root_object(device)?;
    let kind = nonempty_string_field(root, "scene_kind")?;
    let references = device_references(kind, template)?;
    if references.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({
        "device_id": field_clone(root, "device_id")?,
        "scene_kind": field_clone(root, "scene_kind")?,
        "strength": field_clone(root, "strength")?,
        "capability_status": field_clone(root, "capability_status")?,
        "best_for": field_clone(root, "best_for")?,
        "avoid_when": field_clone(root, "avoid_when")?,
        "reference_contract": field_clone(root, "reference_contract")?,
        "allowed_references": references,
        "deterministic_materialization": field_clone(root, "deterministic_materialization")?
    })))
}

/// Return the model-visible shot reference combinations safe for one device and layout.
fn device_references(kind: &str, template: &Value) -> Result<Vec<Value>> {
    let root = root_object(template)?;
    let panels = usize_field(root, "panel_count")?;
    let references = match kind {
        "none" => vec![device_reference("", "")],
        "open_frame" => (1..=panels)
            .map(|index| device_reference(format!("s{index}").as_str(), ""))
            .collect(),
        "crossing" => crossing_references(root)?,
        "master_view" | "diagonal_release" => {
            vec![device_reference("s1", format!("s{panels}").as_str())]
        }
        "inset" => inset_references(root, panels)?,
        "overlap" => overlap_references(root)?,
        _ => bail!("operational device kind '{kind}' is unsupported"),
    };
    Ok(references)
}

/// Create one model-visible source and target shot pair.
fn device_reference(source: &str, target: &str) -> Value {
    json!({"source_panel": source, "target_panel": target})
}

/// Return every safe parent-detail relation, preferring the canonical dominant slot as parent.
fn inset_references(template: &Map<String, Value>, panels: usize) -> Result<Vec<Value>> {
    let parents = optional_usize_field(template, "dominant_index")?
        .map(|index| vec![index])
        .unwrap_or_else(|| (0..panels).collect());
    let geometry = array_field(template, "panels")?;
    let mut references = Vec::new();
    for parent in parents {
        for target in 0..panels {
            if target != parent && inset_union(&geometry[parent], &geometry[target]).is_ok() {
                references.push(device_reference(
                    format!("s{}", parent + 1).as_str(),
                    format!("s{}", target + 1).as_str(),
                ));
            }
        }
    }
    Ok(references)
}

/// Return reading-adjacent crossing pairs that share one direct canonical boundary.
fn crossing_references(template: &Map<String, Value>) -> Result<Vec<Value>> {
    let panels = array_field(template, "panels")?;
    let mut references = Vec::new();
    for index in 0..panels.len().saturating_sub(1) {
        if geometry_adjacent(&panels[index], &panels[index + 1])? {
            references.push(device_reference(
                format!("s{}", index + 1).as_str(),
                format!("s{}", index + 2).as_str(),
            ));
        }
    }
    Ok(references)
}

/// Return directed overlap pairs only for canonically edge-adjacent rectangular slots.
fn overlap_references(template: &Map<String, Value>) -> Result<Vec<Value>> {
    let panels = array_field(template, "panels")?;
    let mut references = Vec::new();
    for source in 0..panels.len() {
        for target in (source + 1)..panels.len() {
            if geometry_adjacent(&panels[source], &panels[target])? {
                if safe_overlap_reference(panels, source, target)? {
                    references.push(device_reference(
                        format!("s{}", source + 1).as_str(),
                        format!("s{}", target + 1).as_str(),
                    ));
                }
                if safe_overlap_reference(panels, target, source)? {
                    references.push(device_reference(
                        format!("s{}", target + 1).as_str(),
                        format!("s{}", source + 1).as_str(),
                    ));
                }
            }
        }
    }
    Ok(references)
}

/// Return whether one directed overlap reaches only its named background slot.
fn safe_overlap_reference(panels: &[Value], source: usize, target: usize) -> Result<bool> {
    let source_bounds =
        DeviceBounds::decode(object_field(root_object(&panels[source])?, "bounds")?)?;
    let target_bounds =
        DeviceBounds::decode(object_field(root_object(&panels[target])?, "bounds")?)?;
    let expanded = overlap_bounds(source_bounds, target_bounds)?;
    if !expanded.intersects(target_bounds)? {
        return Ok(false);
    }
    for (index, panel) in panels.iter().enumerate() {
        if index != source
            && index != target
            && expanded.intersects(DeviceBounds::decode(object_field(
                root_object(panel)?,
                "bounds",
            )?)?)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Return whether two canonical bounds share a horizontal or vertical reading edge.
fn geometry_adjacent(left: &Value, right: &Value) -> Result<bool> {
    let left = DeviceBounds::decode(object_field(root_object(left)?, "bounds")?)?;
    let right = DeviceBounds::decode(object_field(root_object(right)?, "bounds")?)?;
    Ok(facing_gap(left, right)?.is_some_and(|gap| gap <= MAX_DEVICE_GUTTER))
}

/// Return one direct facing-edge gap when two rectangles overlap on the other axis.
fn facing_gap(left: DeviceBounds, right: DeviceBounds) -> Result<Option<i64>> {
    if left.vertical_overlap(right) && left.right()? <= right.x {
        return Ok(Some(right.x - left.right()?));
    }
    if left.vertical_overlap(right) && right.right()? <= left.x {
        return Ok(Some(left.x - right.right()?));
    }
    if left.horizontal_overlap(right) && left.bottom()? <= right.y {
        return Ok(Some(right.y - left.bottom()?));
    }
    if left.horizontal_overlap(right) && right.bottom()? <= left.y {
        return Ok(Some(left.y - right.bottom()?));
    }
    Ok(None)
}

/// Return the safe rectangular union of one parent and one consumed detail slot.
fn inset_union(source: &Value, target: &Value) -> Result<DeviceBounds> {
    let source = DeviceBounds::decode(object_field(root_object(source)?, "bounds")?)?;
    let target = DeviceBounds::decode(object_field(root_object(target)?, "bounds")?)?;
    if facing_gap(source, target)?.is_none_or(|gap| gap > MAX_DEVICE_GUTTER)
        || !((source.y == target.y && source.height == target.height)
            || (source.x == target.x && source.width == target.width))
    {
        bail!("inset slots cannot form one safe rectangular parent union");
    }
    let x = source.x.min(target.x);
    let y = source.y.min(target.y);
    Ok(DeviceBounds {
        x,
        y,
        width: source
            .right()?
            .max(target.right()?)
            .checked_sub(x)
            .ok_or_else(|| anyhow!("inset parent union width overflow"))?,
        height: source
            .bottom()?
            .max(target.bottom()?)
            .checked_sub(y)
            .ok_or_else(|| anyhow!("inset parent union height overflow"))?,
    })
}

/// Validate the embedded operational device catalog before exposing it to Gemini.
fn validate_device_registry(value: &Value) -> Result<()> {
    let root = root_object(value)?;
    if string_field(root, "schema")? != "kamishibai.dynamic-manga.operational-device-registry"
        || usize_field(root, "version")? != 3
    {
        bail!("operational manga device registry schema or version is unsupported");
    }
    let policy = object_field(root, "selection_policy")?;
    if usize_field(policy, "maximum_devices")? != 1
        || string_field(policy, "absence_device")? != "none"
        || string_field(policy, "model_references")? != "shot_id"
        || string_field(policy, "materialized_references")? != "canonical_panel_id"
        || string_values(array_field(policy, "hard_rules")?, "device hard rules")?.is_empty()
    {
        bail!("operational manga device selection policy is inconsistent");
    }
    let devices = array_field(root, "devices")?;
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for device in devices {
        let item = root_object(device)?;
        let id = nonempty_string_field(item, "device_id")?;
        let kind = nonempty_string_field(item, "scene_kind")?;
        bool_field(item, "automatic_selection")?;
        if !ids.insert(id)
            || !kinds.insert(kind)
            || !DEVICE_KINDS.contains(&kind)
            || !matches!(
                string_field(item, "capability_status")?,
                "qualified" | "proven" | "qualification_required"
            )
            || !matches!(string_field(item, "strength")?, "light" | "strong")
            || usize_field(item, "min_panels")? > usize_field(item, "max_panels")?
            || string_values(
                array_field(item, "compatible_topology_families")?,
                "device topology families",
            )?
            .is_empty()
            || string_values(array_field(item, "best_for")?, "device best-for")?.is_empty()
            || string_values(array_field(item, "avoid_when")?, "device avoid-when")?.is_empty()
            || nonempty_string_field(item, "reference_contract")?.is_empty()
            || nonempty_string_field(item, "deterministic_materialization")?.is_empty()
        {
            bail!("operational manga device '{id}' is invalid");
        }
    }
    if kinds != DEVICE_KINDS.into_iter().collect::<BTreeSet<_>>() {
        bail!("operational manga device registry is incomplete");
    }
    Ok(())
}

/// Replace all model-authored topology with the selected registry geometry and policy.
pub(crate) fn materialize(scene: &mut Value, selection: &LayoutSelection) -> Result<()> {
    validate_selected_template(&selection.template)?;
    let template = root_object(&selection.template)?;
    let id = template_id(&selection.template)?.to_owned();
    let family = nonempty_string_field(template, "family")?.to_owned();
    let direction = string_field(template, "reading_direction")?.to_owned();
    let path = array_field(template, "reading_path")?.clone();
    let panels = array_field(template, "panels")?.clone();
    let panel_count = usize_field(template, "panel_count")?;
    let dominant = optional_usize_field(template, "dominant_index")?;
    let strategy = nonempty_string_field(template, "reading_strategy")?.to_owned();
    let geometry_directive = template
        .get("rendering_directive")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(String::from);
    let trigger = selection
        .summary
        .pointer("/scene_features/selection_logic")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("layout selection lacks feature selection logic"))?
        .to_owned();
    let slot = selection
        .summary
        .get("deterministic_slot")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow!("layout selection lacks deterministic slot"))?;
    let reason = selection
        .summary
        .pointer(&format!("/ranked_candidates/{slot}/reason"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("layout selection lacks candidate reason"))?
        .to_owned();
    let root = scene_root_mut(scene)?;
    let requested = root
        .get("page_design")
        .and_then(Value::as_object)
        .and_then(|value| value.get("special_device"))
        .cloned();
    let recovery = selection
        .summary
        .get("scene_attempt_index")
        .and_then(Value::as_u64)
        .is_some_and(|attempt| attempt > 0);
    let mut device = if recovery {
        canonical_device_fallback(
            selection,
            &path,
            planned_shots(selection)?,
            "recovery scene uses canonical panel topology",
        )?
    } else {
        match requested {
            Some(requested) => {
                normalize_device(&requested, selection, &path, planned_shots(selection)?)?
            }
            None => canonical_device_fallback(
                selection,
                &path,
                planned_shots(selection)?,
                "semantic composer omitted special_device",
            )?,
        }
    };
    ensure_object(root, "meta")?
        .insert(String::from("layout_selection"), selection.summary.clone());
    ensure_object(root, "canvas")?
        .insert(String::from("reading_direction"), Value::String(direction));
    {
        let page = ensure_object(root, "page_design")?;
        page.insert(
            String::from("layout"),
            json!({
                "family": family,
                "template_id": id,
                "trigger": trigger,
                "reading_strategy": strategy,
                "reason": reason
            }),
        );
        page.insert(String::from("archetype"), Value::String(id.clone()));
        page.insert(String::from("reading_path"), Value::Array(path.clone()));
        page.insert(
            String::from("dominant_panel"),
            dominant
                .and_then(|index| path.get(index))
                .cloned()
                .unwrap_or_else(|| Value::String(String::new())),
        );
        page.insert(
            String::from("camera_arc"),
            selection
                .summary
                .pointer("/scene_features/camera_arc")
                .cloned()
                .ok_or_else(|| anyhow!("layout selection lacks its motivated camera arc"))?,
        );
    }
    let semantic = root
        .get_mut("panels")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("semantic composer scene must contain a panels array"))?;
    if semantic.len() != panel_count {
        bail!("selected layout template '{id}' requires exactly {panel_count} semantic panels");
    }
    let planned = selection
        .summary
        .pointer("/scene_features/shots")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("layout selection lacks its cinematic shot plan"))?;
    if planned.len() != semantic.len() {
        bail!("semantic composer panel count differs from the cinematic shot plan");
    }
    let camera_continuity = selection
        .summary
        .pointer("/scene_features/camera_arc/continuity")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("layout selection lacks its camera continuity plan"))?;
    for (index, (panel, geometry)) in semantic.iter_mut().zip(panels.iter()).enumerate() {
        let shot = planned
            .get(index)
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("layout selection contains an invalid cinematic shot"))?;
        let expected = shot
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("layout selection contains an invalid shot id"))?;
        bind_shot_contract(panel, shot, expected)?;
        bind_continuity_contract(panel, camera_continuity, index, panel_count)?;
        materialize_panel(panel, geometry, &path, index, &id)?;
    }
    let mut specialized = semantic.clone();
    match apply_device(&mut specialized, &device, &path) {
        Ok(()) => semantic.clone_from(&specialized),
        Err(error) => {
            device = canonical_device_fallback(
                selection,
                &path,
                planned_shots(selection)?,
                error.to_string().as_str(),
            )?;
        }
    }
    let kind = device
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("materialized special device lacks kind"))?
        .to_owned();
    let page = ensure_object(root, "page_design")?;
    page.insert(String::from("special_device"), device);
    page.insert(
        String::from("layout_rendering_directive"),
        Value::String(rendering_directive(
            &id,
            panel_count,
            kind.as_str(),
            geometry_directive.as_deref(),
        )),
    );
    let constraints = ensure_object(root, "constraints")?;
    constraints.insert(
        String::from("maximum_panels"),
        Value::Number(u64::try_from(panel_count)?.into()),
    );
    constraints.insert(String::from("panel_count_lock"), Value::Bool(true));
    constraints.insert(
        String::from("visible_writing"),
        Value::String(String::from(TEXTLESS_VISIBLE_WRITING)),
    );
    ensure_object(ensure_object(root, "art_style")?, "composition")?.insert(
        String::from("motion_rendering"),
        Value::String(String::from(TEXTLESS_MOTION)),
    );
    let rendering = ensure_object(root, "rendering_rules")?;
    rendering.insert(
        String::from("sound_effects"),
        Value::String(String::from(TEXTLESS_SOUND_EFFECTS)),
    );
    rendering.insert(
        String::from("symbols_or_writing"),
        Value::String(String::from(TEXTLESS_VISIBLE_WRITING)),
    );
    rendering.insert(
        String::from("logos_and_emblems"),
        Value::String(String::from(TEXTLESS_MARKS)),
    );
    rendering.insert(
        String::from("signs_and_labels"),
        Value::String(String::from(TEXTLESS_LABELS)),
    );
    rendering.insert(
        String::from("outer_border"),
        Value::String(String::from(OUTER_WHITE_BAND)),
    );
    Ok(())
}

fn bind_shot_contract(
    panel: &mut Value,
    planned: &Map<String, Value>,
    expected: &str,
) -> Result<()> {
    consume_shot_id(panel, expected)?;
    let camera = panel
        .pointer_mut("/scene/camera")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("semantic composer panel lacks its camera object"))?;
    for (planned_field, camera_field) in [
        ("shot_scale", "shot_scale"),
        ("viewpoint", "viewpoint"),
        ("viewpoint_anchor", "viewpoint_subject_id"),
        ("framing", "framing"),
        ("angle", "angle"),
        ("depth_plan", "depth_plan"),
    ] {
        camera.insert(
            String::from(camera_field),
            Value::String(string_field(planned, planned_field)?.to_owned()),
        );
    }
    panel
        .as_object_mut()
        .ok_or_else(|| anyhow!("semantic composer panels must be objects"))?
        .insert(
            String::from("shot_contract"),
            Value::Object(planned.clone()),
        );
    Ok(())
}

fn bind_continuity_contract(
    panel: &mut Value,
    planned: &Map<String, Value>,
    index: usize,
    panel_count: usize,
) -> Result<()> {
    let mode = string_field(planned, "axis_mode")?;
    let relation = match mode {
        "not_applicable" => "not_applicable",
        "preserve" if index == 0 => "establish",
        "preserve" => "preserve",
        "reestablish" if index == 0 => "establish",
        "reestablish" if index + 1 == panel_count => "reestablish",
        "reestablish" => "preserve",
        "deliberate_break" if index == 0 => "establish",
        "deliberate_break" if index + 1 == panel_count => "deliberate_break",
        "deliberate_break" => "preserve",
        _ => bail!("layout selection contains an unsupported camera axis mode"),
    };
    let direction = string_field(planned, "screen_direction")?;
    let continuity = panel
        .get_mut("continuity")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("semantic composer panel lacks its continuity object"))?;
    continuity.insert(
        String::from("axis_relation_from_previous"),
        Value::String(String::from(relation)),
    );
    continuity.insert(
        String::from("screen_direction"),
        Value::String(direction.to_owned()),
    );
    Ok(())
}

/// Return the immutable cinematic shot plan carried by one layout selection.
fn planned_shots(selection: &LayoutSelection) -> Result<&Vec<Value>> {
    selection
        .summary
        .pointer("/scene_features/shots")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("layout selection lacks its cinematic shot plan"))
}

/// Validate one model choice against the eligible catalog and map shot ids to canonical ids.
fn normalize_device(
    requested: &Value,
    selection: &LayoutSelection,
    path: &[Value],
    shots: &[Value],
) -> Result<Value> {
    let candidates = validated_device_candidates(selection, path, shots)?;
    let fallback = canonical_none_device(candidates)?;
    match normalize_requested_device(requested, candidates, path, shots) {
        Ok(device) => Ok(device),
        Err(error) => device_fallback_reason(fallback, error.to_string().as_str()),
    }
}

fn canonical_device_fallback(
    selection: &LayoutSelection,
    path: &[Value],
    shots: &[Value],
    cause: &str,
) -> Result<Value> {
    device_fallback_reason(
        canonical_none_device(validated_device_candidates(selection, path, shots)?)?,
        cause,
    )
}

fn device_fallback_reason(mut device: Value, cause: &str) -> Result<Value> {
    device
        .as_object_mut()
        .ok_or_else(|| anyhow!("canonical none device must be an object"))?
        .insert(
            String::from("reason"),
            Value::String(format!(
                "canonical panel topology retained after local device fallback: {cause}"
            )),
        );
    Ok(device)
}

fn normalize_requested_device(
    requested: &Value,
    candidates: &[Value],
    path: &[Value],
    shots: &[Value],
) -> Result<Value> {
    let root = root_object(requested)?;
    exact_keys(
        root,
        &[
            "kind",
            "reason",
            "source_panel",
            "target_panel",
            "subject_id",
        ],
        "semantic composer special device",
    )?;
    let requested_kind = nonempty_string_field(root, "kind")?;
    let reason = nonempty_string_field(root, "reason")?;
    let source = string_field(root, "source_panel")?;
    let target = string_field(root, "target_panel")?;
    let subject = string_field(root, "subject_id")?;
    let candidate = candidates
        .iter()
        .find(|value| {
            value["scene_kind"].as_str() == Some(requested_kind)
                || value["device_id"].as_str() == Some(requested_kind)
        })
        .ok_or_else(|| {
            anyhow!("special device '{requested_kind}' is incompatible with the selected layout")
        })?;
    let kind = candidate["scene_kind"]
        .as_str()
        .ok_or_else(|| anyhow!("selected special device lacks its canonical scene kind"))?;
    let allowed = candidate["allowed_references"]
        .as_array()
        .ok_or_else(|| anyhow!("special device '{kind}' lacks allowed references"))?;
    if !allowed.iter().any(|value| {
        value["source_panel"].as_str() == Some(source)
            && value["target_panel"].as_str() == Some(target)
    }) {
        bail!("special device '{kind}' chose unsafe shot references");
    }
    let requires_subject = matches!(kind, "crossing" | "master_view");
    if requires_subject == subject.is_empty()
        || (kind == "none" && (!source.is_empty() || !target.is_empty()))
    {
        bail!("special device '{kind}' violates its subject or panel reference contract");
    }
    Ok(json!({
        "kind": kind,
        "reason": reason,
        "source_panel": canonical_reference(source, shots, path)?,
        "target_panel": canonical_reference(target, shots, path)?,
        "subject_id": subject
    }))
}

fn validated_device_candidates<'a>(
    selection: &'a LayoutSelection,
    path: &[Value],
    shots: &[Value],
) -> Result<&'a Vec<Value>> {
    if path.len() != shots.len() || path.is_empty() {
        bail!("layout selection device context has inconsistent panel and shot counts");
    }
    let mut shot_ids = BTreeSet::new();
    let mut panel_ids = BTreeSet::new();
    for (index, (panel, shot)) in path.iter().zip(shots.iter()).enumerate() {
        let panel = panel
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("layout selection has an invalid canonical panel id"))?;
        let shot = nonempty_string_field(root_object(shot)?, "id")?;
        if shot != format!("s{}", index + 1)
            || !shot_ids.insert(shot.to_owned())
            || !panel_ids.insert(panel.to_owned())
        {
            bail!("layout selection has an invalid device shot-to-panel context");
        }
    }
    let candidates = selection
        .summary
        .get("device_candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("layout selection lacks device candidates"))?;
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for candidate in candidates {
        let root = root_object(candidate)?;
        exact_keys(
            root,
            &[
                "device_id",
                "scene_kind",
                "strength",
                "capability_status",
                "best_for",
                "avoid_when",
                "reference_contract",
                "allowed_references",
                "deterministic_materialization",
            ],
            "layout selection device candidate",
        )?;
        let id = nonempty_string_field(root, "device_id")?;
        let kind = nonempty_string_field(root, "scene_kind")?;
        if !ids.insert(id.to_owned())
            || !kinds.insert(kind.to_owned())
            || !DEVICE_KINDS.contains(&kind)
        {
            bail!("layout selection device candidates are inconsistent");
        }
        let references = array_field(root, "allowed_references")?;
        if references.is_empty() {
            bail!("layout selection device candidate has no safe references");
        }
        for reference in references {
            let reference = root_object(reference)?;
            exact_keys(
                reference,
                &["source_panel", "target_panel"],
                "layout selection device reference",
            )?;
            let source = string_field(reference, "source_panel")?;
            let target = string_field(reference, "target_panel")?;
            if (!source.is_empty() && !shot_ids.contains(source))
                || (!target.is_empty() && !shot_ids.contains(target))
                || (!source.is_empty() && source == target)
            {
                bail!("layout selection device candidate contains an unsafe reference");
            }
        }
    }
    if !candidates.iter().any(|candidate| {
        candidate["scene_kind"] == "none"
            && candidate["allowed_references"]
                .as_array()
                .is_some_and(|references| {
                    references.iter().any(|reference| {
                        reference["source_panel"] == "" && reference["target_panel"] == ""
                    })
                })
    }) {
        bail!("layout selection lacks an explicit safe none device");
    }
    Ok(candidates)
}

fn canonical_none_device(candidates: &[Value]) -> Result<Value> {
    candidates
        .iter()
        .find(|candidate| candidate["scene_kind"] == "none")
        .ok_or_else(|| anyhow!("layout selection lacks an explicit safe none device"))?;
    Ok(json!({
        "kind": "none",
        "reason": "the requested structural device was rejected locally; canonical panel topology remains unchanged",
        "source_panel": "",
        "target_panel": "",
        "subject_id": ""
    }))
}

/// Map one model-visible shot id to its selected canonical panel id.
fn canonical_reference(reference: &str, shots: &[Value], path: &[Value]) -> Result<String> {
    if reference.is_empty() {
        return Ok(String::new());
    }
    let index = shots
        .iter()
        .position(|shot| shot["id"].as_str() == Some(reference))
        .ok_or_else(|| anyhow!("special device refers to unknown shot '{reference}'"))?;
    path.get(index)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow!("special device shot '{reference}' has no canonical panel"))
}

/// Apply exactly one normalized device specialization to canonical panels.
fn apply_device(panels: &mut [Value], device: &Value, path: &[Value]) -> Result<()> {
    let root = root_object(device)?;
    let kind = string_field(root, "kind")?;
    let source = string_field(root, "source_panel")?;
    let target = string_field(root, "target_panel")?;
    let subject = string_field(root, "subject_id")?;
    match kind {
        "none" | "diagonal_release" => Ok(()),
        "crossing" => apply_crossing(panels, source, target, subject),
        "overlap" => apply_overlap(panels, source, target),
        "inset" => apply_inset(panels, source, target),
        "open_frame" => apply_open_frame(panels, source),
        "master_view" => validate_master_view(panels, source, target, subject, path),
        _ => bail!("materializer does not support special device '{kind}'"),
    }
}

/// Enable one subject breakout toward the declared adjacent canonical panel.
fn apply_crossing(panels: &mut [Value], source: &str, target: &str, subject: &str) -> Result<()> {
    let source_index = canonical_panel_index(panels, source)?;
    let target_index = canonical_panel_index(panels, target)?;
    if !panel_has_subject(&panels[source_index], subject) {
        bail!("crossing subject '{subject}' is absent from source panel '{source}'");
    }
    let source_bounds = panel_bounds(&panels[source_index])?;
    let target_bounds = panel_bounds(&panels[target_index])?;
    let edge = edge_toward(source_bounds, target_bounds)?;
    let root = panels[source_index]
        .as_object_mut()
        .ok_or_else(|| anyhow!("crossing source panel must be an object"))?;
    ensure_object(root, "continuity")?.insert(
        String::from("breakout"),
        json!({
            "enabled": true,
            "subject_id": subject,
            "edge": edge,
            "destination_panel": target
        }),
    );
    Ok(())
}

/// Move one existing detail slot into a deterministic safe zone inside its parent slot.
fn apply_inset(panels: &mut [Value], source: &str, target: &str) -> Result<()> {
    let source_index = canonical_panel_index(panels, source)?;
    let target_index = canonical_panel_index(panels, target)?;
    let parent = inset_union(&panels[source_index], &panels[target_index])?;
    let bounds = inset_bounds(parent)?;
    panels[source_index]
        .as_object_mut()
        .ok_or_else(|| anyhow!("inset parent panel must be an object"))?
        .insert(String::from("bounds"), parent.json());
    let root = panels[target_index]
        .as_object_mut()
        .ok_or_else(|| anyhow!("inset target panel must be an object"))?;
    root.insert(String::from("bounds"), bounds.json());
    root.insert(String::from("bleed"), Value::Bool(false));
    root.insert(
        String::from("frame"),
        json!({
            "border": "solid",
            "geometry_intent": format!("One protected detail inset inside canonical parent {source}."),
            "overlaps_panel": "",
            "parent_panel": source,
            "polygon": [],
            "shape": "inset",
            "z_index": 1
        }),
    );
    Ok(())
}

/// Extend one foreground slot through one gutter and assign its overlap relation.
fn apply_overlap(panels: &mut [Value], source: &str, target: &str) -> Result<()> {
    let source_index = canonical_panel_index(panels, source)?;
    let target_index = canonical_panel_index(panels, target)?;
    let source_bounds = panel_bounds(&panels[source_index])?;
    let target_bounds = panel_bounds(&panels[target_index])?;
    let bounds = overlap_bounds(source_bounds, target_bounds)?;
    let root = panels[source_index]
        .as_object_mut()
        .ok_or_else(|| anyhow!("overlap source panel must be an object"))?;
    root.insert(String::from("bounds"), bounds.json());
    let frame = ensure_object(root, "frame")?;
    frame.insert(
        String::from("overlaps_panel"),
        Value::String(target.to_owned()),
    );
    frame.insert(String::from("z_index"), Value::Number(1.into()));
    frame.insert(
        String::from("geometry_intent"),
        Value::String(format!(
            "Canonical foreground panel {source} overlaps adjacent panel {target} by one bounded gutter step."
        )),
    );
    Ok(())
}

/// Remove exactly one canonical panel border while preserving its bounds.
fn apply_open_frame(panels: &mut [Value], source: &str) -> Result<()> {
    let index = canonical_panel_index(panels, source)?;
    let root = panels[index]
        .as_object_mut()
        .ok_or_else(|| anyhow!("open-frame source panel must be an object"))?;
    let frame = ensure_object(root, "frame")?;
    frame.insert(String::from("border"), Value::String(String::from("none")));
    frame.insert(
        String::from("shape"),
        Value::String(String::from("open_frame")),
    );
    frame.insert(String::from("polygon"), Value::Array(Vec::new()));
    frame.insert(
        String::from("geometry_intent"),
        Value::String(format!(
            "Canonical panel {source} keeps its clip bounds while its local border opens into the page."
        )),
    );
    Ok(())
}

/// Validate the model-authored continuous-space subject phases used by a master view.
fn validate_master_view(
    panels: &[Value],
    source: &str,
    target: &str,
    subject: &str,
    path: &[Value],
) -> Result<()> {
    let source_index = canonical_panel_index(panels, source)?;
    let target_index = canonical_panel_index(panels, target)?;
    let environment = panel_continuity(&panels[source_index], "shared_environment_id")?.trim();
    if environment.is_empty()
        || panel_continuity(&panels[target_index], "shared_environment_id")?.trim() != environment
    {
        bail!("master view endpoints do not share one nonempty environment id");
    }
    let mut phases = BTreeSet::new();
    for id in path {
        let id = id
            .as_str()
            .ok_or_else(|| anyhow!("master view path contains an invalid panel id"))?;
        let panel = &panels[canonical_panel_index(panels, id)?];
        if panel_continuity(panel, "shared_environment_id")?.trim() == environment {
            let phase = panel_continuity(panel, "subject_phase")?.trim();
            if phase.is_empty() || !panel_has_subject(panel, subject) || !phases.insert(phase) {
                bail!("master view participants lack one subject or distinct phases");
            }
        }
    }
    if phases.len() < 2 {
        bail!("master view requires at least two participating phases");
    }
    Ok(())
}

/// Resolve one canonical panel id to its current panel index.
fn canonical_panel_index(panels: &[Value], id: &str) -> Result<usize> {
    panels
        .iter()
        .position(|panel| panel["id"].as_str() == Some(id))
        .ok_or_else(|| anyhow!("special device refers to unavailable canonical panel '{id}'"))
}

/// Return whether a panel contains one exact stable subject id.
fn panel_has_subject(panel: &Value, subject: &str) -> bool {
    panel
        .pointer("/scene/subjects")
        .and_then(Value::as_array)
        .is_some_and(|subjects| {
            subjects
                .iter()
                .any(|value| value["id"].as_str() == Some(subject))
        })
}

/// Read one required continuity string from a materialized panel.
fn panel_continuity<'a>(panel: &'a Value, field: &str) -> Result<&'a str> {
    panel
        .get("continuity")
        .and_then(Value::as_object)
        .and_then(|value| value.get(field))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("panel continuity field '{field}' must be a string"))
}

/// Decode one panel's current rectangular bounds.
fn panel_bounds(panel: &Value) -> Result<DeviceBounds> {
    DeviceBounds::decode(object_field(root_object(panel)?, "bounds")?)
}

/// Derive a stable breakout edge from the relative canonical panel positions.
fn edge_toward(source: DeviceBounds, target: DeviceBounds) -> Result<&'static str> {
    if source.vertical_overlap(target)
        && source.right()? <= target.x
        && target.x - source.right()? <= MAX_DEVICE_GUTTER
    {
        return Ok("right");
    }
    if source.vertical_overlap(target)
        && target.right()? <= source.x
        && source.x - target.right()? <= MAX_DEVICE_GUTTER
    {
        return Ok("left");
    }
    if source.horizontal_overlap(target)
        && source.bottom()? <= target.y
        && target.y - source.bottom()? <= MAX_DEVICE_GUTTER
    {
        return Ok("bottom");
    }
    if source.horizontal_overlap(target)
        && target.bottom()? <= source.y
        && source.y - target.bottom()? <= MAX_DEVICE_GUTTER
    {
        return Ok("top");
    }
    bail!("crossing panels do not expose one canonical boundary")
}

/// Derive one upper-right inset rectangle with stable parent-relative margins.
fn inset_bounds(parent: DeviceBounds) -> Result<DeviceBounds> {
    let margin = 24;
    let width = (parent.width * 2) / 5;
    let height = (parent.height * 2) / 5;
    if width <= margin || height <= margin {
        bail!("canonical parent panel is too small for one inset");
    }
    let x = parent
        .right()?
        .checked_sub(width)
        .and_then(|value| value.checked_sub(margin))
        .ok_or_else(|| anyhow!("inset horizontal placement overflow"))?;
    let y = parent
        .y
        .checked_add(margin)
        .ok_or_else(|| anyhow!("inset vertical placement overflow"))?;
    Ok(DeviceBounds {
        x,
        y,
        width,
        height,
    })
}

/// Derive one bounded positive-area overlap across the nearest shared edge.
fn overlap_bounds(source: DeviceBounds, target: DeviceBounds) -> Result<DeviceBounds> {
    if facing_gap(source, target)?.is_none_or(|gap| gap > MAX_DEVICE_GUTTER) {
        bail!("overlap panels do not share one direct canonical edge");
    }
    let horizontal = source.width.min(target.width) / 5;
    let vertical = source.height.min(target.height) / 5;
    if source.vertical_overlap(target) && source.right()? <= target.x {
        return Ok(DeviceBounds {
            width: target
                .x
                .checked_sub(source.x)
                .and_then(|value| value.checked_add(horizontal))
                .ok_or_else(|| anyhow!("right overlap width overflow"))?,
            ..source
        });
    }
    if source.vertical_overlap(target) && target.right()? <= source.x {
        let x = target
            .right()?
            .checked_sub(horizontal)
            .ok_or_else(|| anyhow!("left overlap position overflow"))?;
        return Ok(DeviceBounds {
            x,
            width: source
                .right()?
                .checked_sub(x)
                .ok_or_else(|| anyhow!("left overlap width overflow"))?,
            ..source
        });
    }
    if source.horizontal_overlap(target) && source.bottom()? <= target.y {
        return Ok(DeviceBounds {
            height: target
                .y
                .checked_sub(source.y)
                .and_then(|value| value.checked_add(vertical))
                .ok_or_else(|| anyhow!("bottom overlap height overflow"))?,
            ..source
        });
    }
    if source.horizontal_overlap(target) && target.bottom()? <= source.y {
        let y = target
            .bottom()?
            .checked_sub(vertical)
            .ok_or_else(|| anyhow!("top overlap position overflow"))?;
        return Ok(DeviceBounds {
            y,
            height: source
                .bottom()?
                .checked_sub(y)
                .ok_or_else(|| anyhow!("top overlap height overflow"))?,
            ..source
        });
    }
    bail!("overlap panels do not share one canonical edge")
}

fn consume_shot_id(panel: &mut Value, expected: &str) -> Result<()> {
    let root = panel
        .as_object_mut()
        .ok_or_else(|| anyhow!("semantic composer panels must be objects"))?;
    let actual = root
        .get("shot_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("semantic composer panel lacks shot_id"))?;
    if actual != expected {
        bail!("semantic composer reordered the cinematic shot plan");
    }
    root.remove("shot_id");
    Ok(())
}

fn validate_registry(value: &Value) -> Result<()> {
    let root = root_object(value)?;
    if string_field(root, "schema")? != "kamishibai.dynamic-manga.layout-registry"
        || usize_field(root, "version")? != 2
    {
        bail!("manga layout registry schema or version is unsupported");
    }
    validate_coordinate_space(object_field(root, "coordinate_space")?)?;
    let contract = object_field(root, "selection_contract")?;
    validate_contract(contract)?;
    let families = string_values(
        array_field(root, "canonical_families")?,
        "canonical families",
    )?;
    let directions = string_values(
        array_field(root, "reading_directions")?,
        "reading directions",
    )?;
    if families.is_empty()
        || directions != [LEFT_TO_RIGHT, RIGHT_TO_LEFT]
        || directions
            != string_values(
                array_field(contract, "allowed_reading_directions")?,
                "allowed reading directions",
            )?
    {
        bail!("manga layout registry root enums are inconsistent");
    }
    let templates = array_field(root, "templates")?;
    if templates.is_empty() {
        bail!("manga layout registry contains no templates");
    }
    let mut ids = BTreeSet::new();
    for template in templates {
        let id = template_id(template)?;
        if !ids.insert(id.to_owned()) {
            bail!("manga layout registry template '{id}' is duplicated");
        }
        validate_template(template, contract, &families, &directions)?;
    }
    validate_automatic_coverage(templates, contract)?;
    validate_policy(object_field(root, "layout_policy")?, &ids)?;
    Ok(())
}

fn validate_coordinate_space(value: &Map<String, Value>) -> Result<()> {
    if usize_field(value, "canvas")? != 1024
        || i64_field(value, "safe_min")? != PANEL_MIN
        || i64_field(value, "safe_max")? != PANEL_MAX
        || usize_field(value, "default_gutter")? != 16
    {
        bail!("manga layout registry coordinate space is unsupported");
    }
    Ok(())
}

fn validate_contract(value: &Map<String, Value>) -> Result<()> {
    let features = string_values(array_field(value, "features")?, "selection features")?;
    let hard = string_values(array_field(value, "hard_features")?, "hard features")?;
    let soft = string_values(array_field(value, "soft_features")?, "soft features")?;
    if usize_field(value, "minimum_ranked_candidates")? != 1
        || usize_field(value, "maximum_ranked_candidates")? != 3
        || usize_field(value, "maximum_panels")? != 4
        || features
            != [
                "beat_count",
                "temporal_relation",
                "emphasis_curve",
                "motion_vector",
                "intensity",
                "spatial_relation",
                "transition_type",
                "reading_direction",
            ]
        || hard
            != [
                "beat_count",
                "temporal_relation",
                "emphasis_curve",
                "reading_direction",
            ]
        || soft
            != [
                "motion_vector",
                "intensity",
                "spatial_relation",
                "transition_type",
            ]
    {
        bail!("manga layout registry selection contract is inconsistent");
    }
    for field in [
        "allowed_temporal_relations",
        "allowed_emphasis_curves",
        "allowed_motion_vectors",
        "allowed_intensities",
        "allowed_spatial_relations",
        "allowed_transition_types",
        "allowed_reading_directions",
        "hard_rules",
    ] {
        if string_values(array_field(value, field)?, field)?.is_empty() {
            bail!("manga layout registry selection contract has an empty '{field}'");
        }
    }
    Ok(())
}

fn validate_template(
    value: &Value,
    contract: &Map<String, Value>,
    families: &[&str],
    directions: &[&str],
) -> Result<()> {
    let root = root_object(value)?;
    let id = nonempty_string_field(root, "template_id")?;
    let family = nonempty_string_field(root, "family")?;
    let direction = string_field(root, "reading_direction")?;
    let status = string_field(root, "capability_status")?;
    let automatic = bool_field(root, "automatic_selection")?;
    let panel_count = usize_field(root, "panel_count")?;
    if !families.contains(&family)
        || !directions.contains(&direction)
        || !matches!(
            status,
            "candidate" | "qualified" | "quarantined" | "qualification_required"
        )
        || (automatic && matches!(status, "quarantined" | "qualification_required"))
        || !(1..=4).contains(&panel_count)
    {
        bail!("manga layout registry template '{id}' has invalid selection metadata");
    }
    validate_dynamic_constraint(root, id)?;
    for field in [
        "variant",
        "name",
        "evidence_level",
        "reading_strategy",
        "risk",
        "entry_panel",
        "exit_panel",
    ] {
        nonempty_string_field(root, field)?;
    }
    let evidence = string_field(root, "evidence_level")?;
    if !matches!(
        evidence,
        "empirical_geometry" | "narrative_theory" | "design_heuristic"
    ) {
        bail!("manga layout registry template '{id}' has invalid evidence level");
    }
    for field in [
        "source_basis",
        "best_for",
        "avoid_when",
        "compatible_devices",
    ] {
        if string_values(array_field(root, field)?, field)?.is_empty() {
            bail!("manga layout registry template '{id}' has empty '{field}'");
        }
    }
    if automatic
        && !string_values(
            array_field(root, "compatible_devices")?,
            "compatible devices",
        )?
        .contains(&"none")
    {
        bail!("automatic layout template '{id}' cannot isolate topology");
    }
    let expected = (1..=panel_count)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>();
    let path = string_values(array_field(root, "reading_path")?, "reading path")?;
    if path != expected.iter().map(String::as_str).collect::<Vec<_>>()
        || string_field(root, "entry_panel")? != expected[0]
        || string_field(root, "exit_panel")? != expected[panel_count - 1]
        || optional_usize_field(root, "dominant_index")?.is_some_and(|index| index >= panel_count)
    {
        bail!("manga layout registry template '{id}' has an invalid reading contract");
    }
    let mut grouped = Vec::new();
    collect_grouping(
        root.get("grouping_tree")
            .ok_or_else(|| anyhow!("layout '{id}' lacks grouping_tree"))?,
        &mut grouped,
    )?;
    if grouped != expected {
        bail!("manga layout registry template '{id}' grouping does not match reading order");
    }
    validate_profile(
        object_field(root, "feature_profile")?,
        contract,
        panel_count,
        id,
    )?;
    let panels = array_field(root, "panels")?;
    if panels.len() != panel_count {
        bail!("manga layout registry template '{id}' has inconsistent panel metadata");
    }
    for panel in panels {
        validate_geometry(panel, id)?;
    }
    Ok(())
}

fn validate_profile(
    profile: &Map<String, Value>,
    contract: &Map<String, Value>,
    panel_count: usize,
    id: &str,
) -> Result<()> {
    for (field, allowed) in [
        ("temporal_relation", "allowed_temporal_relations"),
        ("emphasis_curve", "allowed_emphasis_curves"),
        ("motion_vector", "allowed_motion_vectors"),
        ("intensity", "allowed_intensities"),
        ("spatial_relation", "allowed_spatial_relations"),
        ("transition_type", "allowed_transition_types"),
    ] {
        let values = string_values(array_field(profile, field)?, field)?;
        let choices = string_values(array_field(contract, allowed)?, allowed)?;
        if (values.is_empty() && !(field == "transition_type" && panel_count == 1))
            || values.iter().any(|value| !choices.contains(value))
        {
            bail!("layout template '{id}' has invalid {field} values");
        }
    }
    Ok(())
}

fn validate_dynamic_constraint(root: &Map<String, Value>, id: &str) -> Result<()> {
    let Some(value) = root.get("dynamic_only") else {
        return Ok(());
    };
    let Some(dynamic) = value.as_bool() else {
        bail!("layout template '{id}' has a non-boolean dynamic constraint");
    };
    if !dynamic {
        return Ok(());
    }
    let profile = object_field(root, "feature_profile")?;
    let intensities = string_values(array_field(profile, "intensity")?, "intensity")?;
    let motions = string_values(array_field(profile, "motion_vector")?, "motion vector")?;
    if intensities.contains(&"quiet")
        || motions.contains(&"still")
        || nonempty_string_field(root, "rendering_directive")?.is_empty()
    {
        bail!("dynamic-only layout template '{id}' accepts calm scene features");
    }
    Ok(())
}

fn validate_automatic_coverage(templates: &[Value], contract: &Map<String, Value>) -> Result<()> {
    let relations = string_values(
        array_field(contract, "allowed_temporal_relations")?,
        "allowed temporal relations",
    )?;
    let emphases = string_values(
        array_field(contract, "allowed_emphasis_curves")?,
        "allowed emphasis curves",
    )?;
    let maximum = usize_field(contract, "maximum_panels")?;
    for panel_count in 1..=maximum {
        for relation in &relations {
            for emphasis in &emphases {
                if !valid_hard_tuple(panel_count, relation, emphasis) {
                    continue;
                }
                let features = json!({
                    "panel_count": panel_count,
                    "panel_relation": relation,
                    "panel_emphasis": emphasis
                });
                if !templates
                    .iter()
                    .any(|template| template_matches(template, &features))
                {
                    bail!(
                        "manga layout registry has no automatic template for {panel_count} panels, {relation}, {emphasis}"
                    );
                }
            }
        }
    }
    Ok(())
}

fn valid_hard_tuple(panel_count: usize, relation: &str, emphasis: &str) -> bool {
    match panel_count {
        1 => relation == "single_moment" && emphasis == "equal",
        2 => relation != "single_moment" && !matches!(emphasis, "rising" | "falling"),
        3 | 4 => relation != "single_moment",
        _ => false,
    }
}

fn collect_grouping(value: &Value, panels: &mut Vec<String>) -> Result<()> {
    let root = root_object(value)?;
    match string_field(root, "kind")? {
        "leaf" => {
            let panel = nonempty_string_field(root, "panel")?;
            if panels.iter().any(|value| value == panel) {
                bail!("layout grouping tree contains a duplicate panel");
            }
            panels.push(panel.to_owned());
        }
        "group" => {
            if !matches!(
                string_field(root, "axis")?,
                "horizontal" | "vertical" | "non_slicing"
            ) {
                bail!("layout grouping tree contains an invalid axis");
            }
            let children = array_field(root, "children")?;
            if children.len() < 2 {
                bail!("layout grouping tree contains an undersized group");
            }
            for child in children {
                collect_grouping(child, panels)?;
            }
        }
        _ => bail!("layout grouping tree contains an invalid node"),
    }
    Ok(())
}

fn validate_geometry(value: &Value, id: &str) -> Result<()> {
    let root = root_object(value)?;
    let shape = nonempty_string_field(root, "shape")?;
    let bounds = object_field(root, "bounds")?;
    let x = i64_field(bounds, "x")?;
    let y = i64_field(bounds, "y")?;
    let width = i64_field(bounds, "width")?;
    let height = i64_field(bounds, "height")?;
    let right = x
        .checked_add(width)
        .ok_or_else(|| anyhow!("layout '{id}' horizontal bounds overflow"))?;
    let bottom = y
        .checked_add(height)
        .ok_or_else(|| anyhow!("layout '{id}' vertical bounds overflow"))?;
    if x < PANEL_MIN
        || y < PANEL_MIN
        || width <= 0
        || height <= 0
        || right > PANEL_MAX
        || bottom > PANEL_MAX
    {
        bail!("manga layout registry template '{id}' leaves the safe canvas");
    }
    let polygon = array_field(root, "polygon")?;
    if (shape == "polygon" && polygon.len() < 3) || (shape != "polygon" && polygon.len() == 1) {
        bail!("manga layout registry template '{id}' has invalid panel geometry");
    }
    for point in polygon {
        let coordinates = point
            .as_array()
            .filter(|value| value.len() == 2)
            .ok_or_else(|| anyhow!("layout '{id}' polygon point must have two coordinates"))?;
        let px = coordinates[0]
            .as_i64()
            .ok_or_else(|| anyhow!("layout '{id}' polygon x must be an integer"))?;
        let py = coordinates[1]
            .as_i64()
            .ok_or_else(|| anyhow!("layout '{id}' polygon y must be an integer"))?;
        if px < x || px > right || py < y || py > bottom {
            bail!("manga layout registry template '{id}' has a polygon outside its bounds");
        }
    }
    Ok(())
}

fn validate_policy(value: &Map<String, Value>, ids: &BTreeSet<String>) -> Result<()> {
    nonempty_string_field(value, "criterion")?;
    let templates = object_field(value, "templates")?;
    if templates.is_empty() {
        bail!("manga layout registry policy is empty");
    }
    for (id, policy) in templates {
        let root = root_object(policy)?;
        if !ids.contains(id)
            || !matches!(
                string_field(root, "priority")?,
                "preferred" | "conditional" | "deprioritized"
            )
            || nonempty_string_field(root, "selection_effect")?.is_empty()
        {
            bail!("manga layout registry policy for '{id}' is invalid");
        }
    }
    Ok(())
}

fn validate_features(value: &Value, contract: &Map<String, Value>, lenient: bool) -> Result<()> {
    let root = root_object(value)?;
    exact_keys(root, &FEATURE_FIELDS, "layout scene features")?;
    let semantic_count = usize_field(root, "semantic_beat_count")?;
    let semantic_relation = enum_field(
        root,
        "semantic_relation",
        contract,
        "allowed_temporal_relations",
    )?;
    let panel_count = usize_field(root, "panel_count")?;
    let panel_relation = enum_field(
        root,
        "panel_relation",
        contract,
        "allowed_temporal_relations",
    )?;
    let panel_emphasis = enum_field(root, "panel_emphasis", contract, "allowed_emphasis_curves")?;
    let decomposition = string_field(root, "decomposition_mode")?;
    if !DECOMPOSITION_MODES.contains(&decomposition) {
        bail!("layout scene feature 'decomposition_mode' has an unsupported value");
    }
    enum_field(root, "motion_vector", contract, "allowed_motion_vectors")?;
    enum_field(root, "intensity", contract, "allowed_intensities")?;
    enum_field(
        root,
        "spatial_relation",
        contract,
        "allowed_spatial_relations",
    )?;
    let transition = enum_field(
        root,
        "transition_type",
        contract,
        "allowed_transition_types",
    )?;
    let direction = enum_field(
        root,
        "reading_direction",
        contract,
        "allowed_reading_directions",
    )?;
    nonempty_string_field(root, "literal_anchor")?;
    nonempty_string_field(root, "selection_logic")?;
    validate_coverage_audit(root, panel_count)?;
    if !(1..=4).contains(&semantic_count)
        || !(1..=4).contains(&panel_count)
        || panel_count < semantic_count
        || direction != LEFT_TO_RIGHT
        || (semantic_count == 1 && semantic_relation != "single_moment")
        || (semantic_count > 1 && semantic_relation == "single_moment")
        || !valid_hard_tuple(panel_count, panel_relation, panel_emphasis)
        || (panel_count == 1 && (transition != "none" || decomposition != "single_tableau"))
        || (panel_count > 1
            && (panel_relation == "single_moment"
                || transition == "none"
                || decomposition == "single_tableau"))
        || (panel_count > semantic_count && decomposition == "one_to_one")
    {
        bail!("layout feature extractor returned a contradictory feature vector");
    }
    let camera_arc = validate_camera_arc(root, panel_count)?;
    validate_shots(root, semantic_count, panel_count, camera_arc, lenient)?;
    Ok(())
}

fn validate_camera_arc(root: &Map<String, Value>, panel_count: usize) -> Result<&str> {
    let arc = object_field(root, "camera_arc")?;
    exact_keys(
        arc,
        &["strategy", "progression", "motivation", "continuity"],
        "camera arc",
    )?;
    let strategy = string_field(arc, "strategy")?;
    if !CAMERA_ARCS.contains(&strategy)
        || (panel_count == 1 && strategy != "single_view")
        || (panel_count > 1 && strategy == "single_view")
    {
        bail!("camera arc strategy contradicts the selected panel count");
    }
    nonempty_string_field(arc, "progression")?;
    nonempty_string_field(arc, "motivation")?;
    let continuity = object_field(arc, "continuity")?;
    exact_keys(
        continuity,
        &["axis_mode", "axis", "screen_direction", "eyeline_policy"],
        "camera arc continuity",
    )?;
    let axis_mode = string_field(continuity, "axis_mode")?;
    if ![
        "not_applicable",
        "preserve",
        "reestablish",
        "deliberate_break",
    ]
    .contains(&axis_mode)
    {
        bail!("camera arc continuity has an unsupported axis mode");
    }
    let axis = string_field(continuity, "axis")?;
    if (axis_mode == "not_applicable") != axis.is_empty() {
        bail!("camera arc axis must be named exactly when continuity uses one");
    }
    if ![
        "not_applicable",
        "stationary",
        "left_to_right",
        "right_to_left",
        "toward_camera",
        "away_from_camera",
        "converging",
        "diverging",
    ]
    .contains(&string_field(continuity, "screen_direction")?)
        || !["not_applicable", "matched", "deliberately_broken"]
            .contains(&string_field(continuity, "eyeline_policy")?)
    {
        bail!("camera arc continuity contains an unsupported visual-continuity policy");
    }
    Ok(strategy)
}

fn validate_coverage_audit(root: &Map<String, Value>, panel_count: usize) -> Result<()> {
    let audit = array_field(root, "coverage_audit")?;
    if audit.len() != 4 {
        bail!("layout feature extractor returned an incomplete coverage audit");
    }
    let mut selected = 0;
    for (index, entry) in audit.iter().enumerate() {
        let value = root_object(entry)?;
        exact_keys(
            value,
            &[
                "panel_count",
                "added_view",
                "source_support",
                "verdict",
                "reason",
            ],
            "layout coverage audit entry",
        )?;
        let count = index + 1;
        let verdict = string_field(value, "verdict")?;
        let expected = match count.cmp(&panel_count) {
            std::cmp::Ordering::Less => "insufficient",
            std::cmp::Ordering::Equal => "selected",
            std::cmp::Ordering::Greater => "redundant_or_unsupported",
        };
        if usize_field(value, "panel_count")? != count
            || !COVERAGE_VERDICTS.contains(&verdict)
            || verdict != expected
            || nonempty_string_field(value, "added_view")?.is_empty()
            || nonempty_string_field(value, "source_support")?.is_empty()
            || nonempty_string_field(value, "reason")?.is_empty()
        {
            bail!(
                "layout feature extractor returned a coverage audit that disagrees with panel_count"
            );
        }
        if verdict == "selected" {
            selected += 1;
        }
    }
    if selected != 1 {
        bail!("layout feature extractor must select exactly one coverage count");
    }
    Ok(())
}

fn validate_shots(
    root: &Map<String, Value>,
    semantic_count: usize,
    panel_count: usize,
    camera_arc: &str,
    lenient: bool,
) -> Result<()> {
    let shots = array_field(root, "shots")?;
    if shots.len() != panel_count {
        bail!("layout feature extractor returned a shot count that differs from panel_count");
    }
    let mut covered = BTreeSet::new();
    let mut camera_setups = Vec::with_capacity(shots.len());
    let mut scales = Vec::with_capacity(shots.len());
    for (index, shot) in shots.iter().enumerate() {
        let value = root_object(shot)?;
        exact_keys(
            value,
            &[
                "id",
                "semantic_beat_index",
                "role",
                "visible_anchor",
                "source_support",
                "shot_scale",
                "viewpoint",
                "viewpoint_anchor",
                "framing",
                "angle",
                "depth_plan",
                "camera_motivation",
                "information_gain",
                "transition_trigger",
            ],
            "layout shot",
        )?;
        let expected = format!("s{}", index + 1);
        if string_field(value, "id")? != expected {
            bail!("layout feature extractor returned unordered shot ids");
        }
        let beat = usize_field(value, "semantic_beat_index")?;
        if !(1..=semantic_count).contains(&beat) {
            bail!("layout shot refers to an unavailable semantic beat");
        }
        covered.insert(beat);
        let role = string_field(value, "role")?;
        if !SHOT_ROLES.contains(&role) {
            bail!("layout shot has an unsupported cinematic role");
        }
        nonempty_string_field(value, "visible_anchor")?;
        nonempty_string_field(value, "source_support")?;
        let scale = string_field(value, "shot_scale")?;
        let viewpoint = string_field(value, "viewpoint")?;
        let viewpoint_anchor = string_field(value, "viewpoint_anchor")?;
        let framing = string_field(value, "framing")?;
        let angle = string_field(value, "angle")?;
        let depth = string_field(value, "depth_plan")?;
        let trigger = string_field(value, "transition_trigger")?;
        if !SHOT_SCALES.contains(&scale)
            || !VIEWPOINTS.contains(&viewpoint)
            || !FRAMINGS.contains(&framing)
            || !CAMERA_ANGLES.contains(&angle)
            || !DEPTH_PLANS.contains(&depth)
            || !TRANSITION_TRIGGERS.contains(&trigger)
            || (viewpoint == "objective") != viewpoint_anchor.is_empty()
            || (index == 0 && trigger != "scene_open")
            || (index > 0 && trigger == "scene_open")
            || (framing == "insert"
                && (role != "detail" || !matches!(scale, "close" | "extreme_close")))
        {
            bail!("layout shot contains a contradictory motivated-camera setup");
        }
        nonempty_string_field(value, "camera_motivation")?;
        nonempty_string_field(value, "information_gain")?;
        camera_setups.push((scale, viewpoint, framing, angle, depth));
        scales.push(
            SHOT_SCALES
                .iter()
                .position(|value| value == &scale)
                .ok_or_else(|| anyhow!("layout shot scale has no cinematic rank"))?,
        );
    }
    if covered.len() != semantic_count {
        bail!("layout shot plan leaves a semantic beat uncovered");
    }
    if !lenient {
        validate_camera_progression(camera_arc, &camera_setups, &scales)?;
    }
    Ok(())
}

fn validate_camera_progression(
    strategy: &str,
    setups: &[(&str, &str, &str, &str, &str)],
    scales: &[usize],
) -> Result<()> {
    let pairs = setups.windows(2);
    if strategy == "hold" {
        if pairs.clone().any(|pair| pair[0] != pair[1]) {
            bail!("hold camera arc must preserve one deliberate camera setup");
        }
    } else if pairs.clone().any(|pair| pair[0] == pair[1]) {
        bail!("motivated camera progression cannot repeat an unchanged adjacent setup");
    }
    if strategy == "push_in"
        && (!scales.windows(2).all(|pair| pair[0] <= pair[1]) || scales.first() >= scales.last())
    {
        bail!("push-in camera arc must progress toward a closer final shot");
    }
    if strategy == "pull_back_reveal"
        && (!scales.windows(2).all(|pair| pair[0] >= pair[1]) || scales.first() <= scales.last())
    {
        bail!("pull-back camera arc must progress toward a wider final shot");
    }
    if strategy == "wide_detail_return" {
        let first = scales
            .first()
            .ok_or_else(|| anyhow!("wide-detail-return camera arc has no opening shot"))?;
        let last = scales
            .last()
            .ok_or_else(|| anyhow!("wide-detail-return camera arc has no return shot"))?;
        let middle = scales
            .get(1..scales.len().saturating_sub(1))
            .unwrap_or_default();
        if scales.len() < 3 || middle.iter().all(|scale| scale <= first || scale <= last) {
            bail!("wide-detail-return camera arc requires a closer middle evidence shot");
        }
    }
    Ok(())
}

fn normalize_semantic_relation(value: &mut Value) -> Result<()> {
    let root = root_object(value)?;
    let semantic_count = usize_field(root, "semantic_beat_count")?;
    let semantic_relation = string_field(root, "semantic_relation")?;
    let panel_relation = string_field(root, "panel_relation")?;
    let transition = string_field(root, "transition_type")?;
    if semantic_count <= 1
        || semantic_relation != "single_moment"
        || panel_relation != "simultaneous"
        || transition != "aspect_to_aspect"
    {
        return Ok(());
    }
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("layout feature response must be an object"))?
        .insert(
            String::from("semantic_relation"),
            Value::String(String::from("simultaneous")),
        );
    Ok(())
}

fn normalize_camera_arc(value: &mut Value) -> Result<()> {
    let root = root_object(value)?;
    let arc = object_field(root, "camera_arc")?;
    let strategy = string_field(arc, "strategy")?;
    if !matches!(
        strategy,
        "push_in" | "pull_back_reveal" | "wide_detail_return"
    ) {
        return Ok(());
    }
    let scales = array_field(root, "shots")?
        .iter()
        .map(|shot| {
            let scale = String::from(string_field(root_object(shot)?, "shot_scale")?);
            let rank = SHOT_SCALES
                .iter()
                .position(|candidate| candidate == &scale)
                .ok_or_else(|| anyhow!("layout shot scale has no cinematic rank"))?;
            Ok((scale, rank))
        })
        .collect::<Result<Vec<_>>>()?;
    let ranks = scales.iter().map(|(_, rank)| *rank).collect::<Vec<_>>();
    let push = ranks.windows(2).all(|pair| pair[0] <= pair[1])
        && ranks
            .first()
            .is_some_and(|first| ranks.last() > Some(first));
    let pull = ranks.windows(2).all(|pair| pair[0] >= pair[1])
        && ranks
            .first()
            .is_some_and(|first| ranks.last() < Some(first));
    let detail = ranks.len() >= 3
        && ranks[1..ranks.len() - 1]
            .iter()
            .any(|rank| *rank > ranks[0] && *rank > ranks[ranks.len() - 1]);
    let compatible = match strategy {
        "push_in" => push,
        "pull_back_reveal" => pull,
        "wide_detail_return" => detail,
        _ => true,
    };
    if compatible {
        return Ok(());
    }
    let replacement = if push {
        "push_in"
    } else if pull {
        "pull_back_reveal"
    } else if detail {
        "wide_detail_return"
    } else {
        "motivated_mixed"
    };
    let root = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("layout feature response must be an object"))?;
    let arc = root
        .get_mut("camera_arc")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("layout feature response must contain a camera arc"))?;
    arc.insert(
        String::from("strategy"),
        Value::String(String::from(replacement)),
    );
    arc.insert(
        String::from("progression"),
        Value::String(
            scales
                .into_iter()
                .map(|(scale, _)| scale)
                .collect::<Vec<_>>()
                .join(" -> "),
        ),
    );
    Ok(())
}

fn normalize_shots(value: &mut Value) -> Result<()> {
    let shots = value
        .as_object_mut()
        .and_then(|root| root.get_mut("shots"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("layout feature response must contain shots"))?;
    for (index, shot) in shots.iter_mut().enumerate() {
        let root = shot
            .as_object_mut()
            .ok_or_else(|| anyhow!("layout feature shots must be objects"))?;
        let viewpoint = String::from(string_field(root, "viewpoint")?);
        let anchor = String::from(string_field(root, "viewpoint_anchor")?);
        if viewpoint == "objective" {
            root.insert(
                String::from("viewpoint_anchor"),
                Value::String(String::new()),
            );
        } else if anchor.trim().is_empty() {
            root.insert(
                String::from("viewpoint"),
                Value::String(String::from("objective")),
            );
            root.insert(
                String::from("viewpoint_anchor"),
                Value::String(String::new()),
            );
        }
        let trigger = String::from(string_field(root, "transition_trigger")?);
        if index == 0 && trigger != "scene_open" {
            root.insert(
                String::from("transition_trigger"),
                Value::String(String::from("scene_open")),
            );
        } else if index > 0 && trigger == "scene_open" {
            let replacement = match string_field(root, "role")? {
                "establishing" => "spatial_reorientation",
                "action" => "new_action",
                "detail" => "detail_reveal",
                "reaction" => "emotion_change",
                "payoff" => "payoff",
                "aspect" => "attention_shift",
                _ => "attention_shift",
            };
            root.insert(
                String::from("transition_trigger"),
                Value::String(String::from(replacement)),
            );
        }
        let framing = string_field(root, "framing")?;
        let role = string_field(root, "role")?;
        let scale = string_field(root, "shot_scale")?;
        if framing == "insert" && (role != "detail" || !matches!(scale, "close" | "extreme_close"))
        {
            root.insert(
                String::from("framing"),
                Value::String(String::from("single")),
            );
        }
    }
    Ok(())
}

fn normalize_coverage_audit(value: &mut Value) -> Result<()> {
    let root = root_object(value)?;
    let panel_count = usize_field(root, "panel_count")?;
    let supplied = root
        .get("coverage_audit")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let normalized = (1..=4)
        .map(|count| {
            let source = supplied.iter().find_map(|entry| {
                entry
                    .as_object()
                    .filter(|item| item.get("panel_count").and_then(Value::as_u64) == u64::try_from(count).ok())
            });
            let prose = |field: &str, fallback: String| {
                source
                    .and_then(|item| item.get(field))
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .map(String::from)
                    .unwrap_or(fallback)
            };
            let verdict = match count.cmp(&panel_count) {
                std::cmp::Ordering::Less => "insufficient",
                std::cmp::Ordering::Equal => "selected",
                std::cmp::Ordering::Greater => "redundant_or_unsupported",
            };
            json!({
                "panel_count": count,
                "added_view": prose("added_view", format!("canonical {count}-panel coverage option")),
                "source_support": prose("source_support", String::from("locally normalized from the authoritative panel count")),
                "verdict": verdict,
                "reason": prose("reason", format!("local coverage normalization marks {count} panels as {verdict}"))
            })
        })
        .collect::<Vec<_>>();
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("layout feature response must be an object"))?
        .insert(String::from("coverage_audit"), Value::Array(normalized));
    Ok(())
}

fn nearest_same_count_template<'a>(
    templates: &'a [Value],
    features: &Value,
) -> Result<Option<&'a Value>> {
    nearest_same_count_template_excluding(templates, features, None)
}

fn nearest_same_count_alternative<'a>(
    templates: &'a [Value],
    features: &Value,
    selected: &Value,
) -> Result<Option<&'a Value>> {
    nearest_same_count_template_excluding(templates, features, Some(selected))
}

fn nearest_same_count_template_excluding<'a>(
    templates: &'a [Value],
    features: &Value,
    excluded: Option<&Value>,
) -> Result<Option<&'a Value>> {
    let feature = root_object(features)?;
    let panel_count = usize_field(feature, "panel_count")?;
    let mut ranked = Vec::new();
    for template in templates {
        let root = root_object(template)?;
        let id = template_id(template)?;
        if let Some(excluded) = excluded {
            let same_id = template_id(excluded)? == id;
            let same_geometry = root.get("panels") == root_object(excluded)?.get("panels");
            if same_id || same_geometry {
                continue;
            }
        }
        if !bool_field(root, "automatic_selection")?
            || string_field(root, "reading_direction")? != LEFT_TO_RIGHT
            || !matches!(
                string_field(root, "capability_status")?,
                "candidate" | "qualified"
            )
            || usize_field(root, "panel_count")? != panel_count
            || !dynamic_constraint_matches(root, feature)
            || !ranking_template_allowed(template, feature)
        {
            continue;
        }
        let distance = usize::from(!profile_includes(
            root,
            "temporal_relation",
            feature,
            "panel_relation",
        )) + usize::from(!profile_includes(
            root,
            "emphasis_curve",
            feature,
            "panel_emphasis",
        ));
        let ordinary = usize::from(!matches!(
            id,
            "splash-1-v1" | "equal-split-vertical-2-v1" | "orthogonal-grid-3-v1" | "grid-2x2-4-v1"
        ));
        let dynamic = usize::from(dynamic_only(template));
        ranked.push(((distance, ordinary, dynamic, id.to_owned()), template));
    }
    ranked.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(ranked.first().map(|(_, template)| *template))
}

fn template_matches(template: &Value, features: &Value) -> bool {
    let Some(root) = template.as_object() else {
        return false;
    };
    let Some(feature) = features.as_object() else {
        return false;
    };
    root.get("automatic_selection").and_then(Value::as_bool) == Some(true)
        && matches!(
            root.get("capability_status").and_then(Value::as_str),
            Some("candidate" | "qualified")
        )
        && root.get("reading_direction").and_then(Value::as_str) == Some(LEFT_TO_RIGHT)
        && root.get("panel_count").and_then(Value::as_u64)
            == feature.get("panel_count").and_then(Value::as_u64)
        && profile_includes(root, "temporal_relation", feature, "panel_relation")
        && profile_includes(root, "emphasis_curve", feature, "panel_emphasis")
        && dynamic_constraint_matches(root, feature)
}

fn dynamic_constraint_matches(
    template: &Map<String, Value>,
    features: &Map<String, Value>,
) -> bool {
    match template.get("dynamic_only").and_then(Value::as_bool) {
        None | Some(false) => true,
        Some(true) => {
            matches!(
                features.get("intensity").and_then(Value::as_str),
                Some("medium" | "high")
            ) && !matches!(
                features.get("motion_vector").and_then(Value::as_str),
                None | Some("still")
            )
        }
    }
}

fn profile_includes(
    template: &Map<String, Value>,
    profile_field: &str,
    features: &Map<String, Value>,
    feature_field: &str,
) -> bool {
    let expected = features.get(feature_field).and_then(Value::as_str);
    template
        .get("feature_profile")
        .and_then(Value::as_object)
        .and_then(|value| value.get(profile_field))
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == expected))
}

fn ranking_template_allowed(template: &Value, features: &Map<String, Value>) -> bool {
    let calm = features.get("intensity").and_then(Value::as_str) == Some("quiet")
        || features.get("motion_vector").and_then(Value::as_str) == Some("still");
    (!calm || template.get("family").and_then(Value::as_str) != Some("diagonal_sequence"))
        && shot_hierarchy_matches(template, features)
}

fn shot_hierarchy_matches(template: &Value, features: &Map<String, Value>) -> bool {
    let Some(id) = template.get("template_id").and_then(Value::as_str) else {
        return false;
    };
    let Some(shots) = features.get("shots").and_then(Value::as_array) else {
        return false;
    };
    let roles = shots
        .iter()
        .filter_map(|shot| shot.get("role").and_then(Value::as_str))
        .collect::<Vec<_>>();
    match id {
        "diagonal-split-2-end-strong-v1" => {
            roles.len() == 2 && roles[0] != "payoff" && roles[1] == "payoff"
        }
        "slanted-t-bottom-3-p2-v1" | "slanted-dominant-rail-3-p2-v1" => {
            roles.len() == 3
                && matches!(roles[0], "establishing" | "aspect")
                && matches!(roles[1], "action" | "detail" | "reaction")
                && roles[2] == "payoff"
        }
        _ => true,
    }
}

fn soft_match_score(template: &Value, features: &Map<String, Value>) -> Result<usize> {
    let root = root_object(template)?;
    Ok(SOFT_FEATURE_FIELDS
        .iter()
        .filter(|field| profile_includes(root, field, features, field))
        .count())
}

fn layout_priority(
    policy: &Map<String, Value>,
    template_id: &str,
) -> Result<(usize, &'static str)> {
    let Some(value) = policy.get(template_id) else {
        return Ok((1, "neutral"));
    };
    match string_field(root_object(value)?, "priority")? {
        "preferred" => Ok((0, "preferred")),
        "conditional" => Ok((2, "conditional")),
        "deprioritized" => Ok((3, "deprioritized")),
        priority => bail!("layout policy priority '{priority}' is unsupported"),
    }
}

fn local_candidate(template_id: &str, score: usize, priority: &str) -> Value {
    json!({
        "template_id": template_id,
        "adaptation": "exact",
        "reason": format!(
            "local deterministic fit matched {score} of {} soft features with {priority} policy",
            SOFT_FEATURE_FIELDS.len()
        )
    })
}

fn geometry_signature(template: &Value) -> Result<String> {
    serde_json::to_string(array_field(root_object(template)?, "panels")?)
        .context("cannot encode canonical layout geometry")
}

fn candidate_geometry_slot(
    candidates: &[Value],
    templates: &[Value],
    geometry: &str,
) -> Result<Option<usize>> {
    for (index, candidate) in candidates.iter().enumerate() {
        let template = ranked_template(candidate, templates)?;
        if geometry_signature(template)? == geometry {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn validate_selected_template(value: &Value) -> Result<()> {
    let root = root_object(value)?;
    let id = template_id(value)?;
    if !bool_field(root, "automatic_selection")?
        || string_field(root, "reading_direction")? != LEFT_TO_RIGHT
        || !matches!(
            string_field(root, "capability_status")?,
            "candidate" | "qualified"
        )
    {
        bail!("selected layout template '{id}' is not eligible for production");
    }
    Ok(())
}

fn materialize_panel(
    panel: &mut Value,
    geometry: &Value,
    path: &[Value],
    index: usize,
    template_id: &str,
) -> Result<()> {
    let root = panel
        .as_object_mut()
        .ok_or_else(|| anyhow!("semantic composer panels must be objects"))?;
    let source = root_object(geometry)?;
    let id = path
        .get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("canonical reading path contains an invalid panel id"))?;
    root.insert(String::from("id"), Value::String(id.to_owned()));
    root.insert(String::from("bleed"), Value::Bool(false));
    root.insert(String::from("bounds"), field_clone(source, "bounds")?);
    root.insert(
        String::from("frame"),
        json!({
            "border": "solid",
            "geometry_intent": format!("Canonical panel {} of template {}.", index + 1, template_id),
            "overlaps_panel": "",
            "parent_panel": "",
            "polygon": field_clone(source, "polygon")?,
            "shape": field_clone(source, "shape")?,
            "z_index": 0
        }),
    );
    ensure_object(root, "continuity")?.insert(
        String::from("breakout"),
        json!({
            "enabled": false,
            "subject_id": "",
            "edge": "empty",
            "destination_panel": ""
        }),
    );
    ensure_object(root, "scene")?.insert(
        String::from("text_in_frame"),
        Value::String(String::from("none")),
    );
    Ok(())
}

fn rendering_directive(
    template_id: &str,
    panel_count: usize,
    device: &str,
    geometry: Option<&str>,
) -> String {
    if panel_count == 1 {
        if device == "open_frame" {
            return format!(
                "{OUTER_WHITE_BAND} Render exactly one continuous image in template {template_id}. Open only the declared local panel frame inside that closed white band; never extend artwork to a canvas edge. Never add an interior divider, inset, montage, coordinate, label, or construction mark."
            );
        }
        return format!(
            "{OUTER_WHITE_BAND} Render exactly one continuous image inside the single panel frame of template {template_id}. Never add an interior border, white divider, gutter, inset, split composition, diptych, montage, coordinate, label, or construction mark."
        );
    }
    let specialization = match device {
        "none" => String::from(
            "Keep every subject contained and preserve clean white gutters between every panel.",
        ),
        "crossing" => String::from(
            "Render exactly the one declared subject crossing from its source panel into its adjacent destination while every other subject and gutter remains contained.",
        ),
        "overlap" => String::from(
            "Render exactly the declared higher-z foreground panel overlapping its named background panel; preserve every other canonical edge and reading entry.",
        ),
        "inset" => String::from(
            "Render exactly the declared detail panel as a higher-z inset contained inside its named parent; do not create any additional inset or panel.",
        ),
        "open_frame" => String::from(
            "Omit only the declared panel's local border inside its unchanged clip region. The open frame never becomes page bleed: every neighboring panel and the closed outer white band remain intact.",
        ),
        "master_view" => String::from(
            "Keep the declared subject visually identical across distinct phases of one shared continuous environment while preserving every canonical divider.",
        ),
        "diagonal_release" => String::from(
            "Use the canonical diagonal strip as the declared directional route from its source endpoint to its target endpoint without adding crossings or dividers.",
        ),
        _ => String::from("Preserve the declared locally materialized device."),
    };
    let geometry = geometry
        .map(|value| format!(" {value}"))
        .unwrap_or_default();
    format!(
        "{OUTER_WHITE_BAND} Render exactly {panel_count} visible semantic panel regions using the locally materialized geometry for template {template_id}.{geometry} {specialization} Never subdivide, duplicate, merge, or add a semantic panel. Add no invented dividers, coordinates, labels, or construction marks."
    )
}

fn selection_slot(seed: &str, candidates: usize) -> usize {
    assert!(
        candidates > 0,
        "invariant: selector shortlist cannot be empty"
    );
    let hash = seed
        .as_bytes()
        .iter()
        .fold(2_166_136_261_u32, |value, byte| {
            value.wrapping_mul(16_777_619) ^ u32::from(*byte)
        });
    usize::try_from(hash).expect("invariant: selector hash fits usize") % candidates
}

fn ranked_template<'a>(candidate: &Value, templates: &'a [Value]) -> Result<&'a Value> {
    let id = string_field(root_object(candidate)?, "template_id")?;
    templates
        .iter()
        .find(|template| template_id(template).is_ok_and(|value| value == id))
        .ok_or_else(|| anyhow!("selected layout template '{id}' is unavailable"))
}

fn dynamic_only(template: &Value) -> bool {
    template.get("dynamic_only").and_then(Value::as_bool) == Some(true)
}

fn scene_root_mut(scene: &mut Value) -> Result<&mut Map<String, Value>> {
    if scene.get("manga_panel").is_some() {
        return scene
            .get_mut("manga_panel")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("scene manga_panel must be an object"));
    }
    scene
        .as_object_mut()
        .ok_or_else(|| anyhow!("semantic composer scene must be an object"))
}

fn ensure_object<'a>(
    root: &'a mut Map<String, Value>,
    field: &str,
) -> Result<&'a mut Map<String, Value>> {
    if !root.contains_key(field) {
        root.insert(String::from(field), Value::Object(Map::new()));
    }
    root.get_mut(field)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("scene field '{field}' must be an object"))
}

fn contract(value: &Value) -> Result<&Map<String, Value>> {
    object_field(root_object(value)?, "selection_contract")
}

fn root_object(value: &Value) -> Result<&Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow!("JSON value must be an object"))
}

fn object_field<'a>(root: &'a Map<String, Value>, field: &str) -> Result<&'a Map<String, Value>> {
    root.get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be an object"))
}

fn array_field<'a>(root: &'a Map<String, Value>, field: &str) -> Result<&'a Vec<Value>> {
    root.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be an array"))
}

fn string_field<'a>(root: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
    root.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be a string"))
}

fn nonempty_string_field<'a>(root: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
    let value = string_field(root, field)?;
    if value.trim().is_empty() {
        bail!("JSON field '{field}' must be nonempty");
    }
    Ok(value)
}

fn bool_field(root: &Map<String, Value>, field: &str) -> Result<bool> {
    root.get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be a boolean"))
}

fn usize_field(root: &Map<String, Value>, field: &str) -> Result<usize> {
    let value = root
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be a nonnegative integer"))?;
    usize::try_from(value).with_context(|| format!("JSON field '{field}' does not fit usize"))
}

fn optional_usize_field(root: &Map<String, Value>, field: &str) -> Result<Option<usize>> {
    match root.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("JSON field '{field}' must be null or a nonnegative integer"))
            .and_then(|value| {
                usize::try_from(value)
                    .with_context(|| format!("JSON field '{field}' does not fit usize"))
            })
            .map(Some),
    }
}

fn i64_field(root: &Map<String, Value>, field: &str) -> Result<i64> {
    root.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("JSON field '{field}' must be an integer"))
}

fn field_clone(root: &Map<String, Value>, field: &str) -> Result<Value> {
    root.get(field)
        .cloned()
        .ok_or_else(|| anyhow!("JSON field '{field}' is missing"))
}

fn template_id(value: &Value) -> Result<&str> {
    nonempty_string_field(root_object(value)?, "template_id")
}

fn string_values<'a>(values: &'a [Value], label: &str) -> Result<Vec<&'a str>> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|item| !item.trim().is_empty())
                .ok_or_else(|| anyhow!("{label} must contain nonempty strings"))
        })
        .collect()
}

fn enum_field<'a>(
    root: &'a Map<String, Value>,
    field: &str,
    contract: &Map<String, Value>,
    allowed: &str,
) -> Result<&'a str> {
    let value = string_field(root, field)?;
    if !string_values(array_field(contract, allowed)?, allowed)?.contains(&value) {
        bail!("layout scene feature '{field}' has an unsupported value");
    }
    Ok(value)
}

fn exact_keys(root: &Map<String, Value>, expected: &[&str], label: &str) -> Result<()> {
    let actual = root.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        bail!("{label} has unexpected or missing fields");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_invalid_quarantined_and_rtl_inputs() {
        let mut quarantined = serde_json::from_str::<Value>(REGISTRY_SOURCE)
            .expect("invariant: registry test fixture must decode");
        let layout = quarantined["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
            .iter_mut()
            .find(|value| value["template_id"] == "radial-y-3-v1")
            .expect("invariant: quarantined layout must exist");
        layout["automatic_selection"] = Value::Bool(true);
        let invalid = REGISTRY_SOURCE.replacen(
            "kamishibai.dynamic-manga.layout-registry",
            "invalid.registry",
            1,
        );
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let rtl = feature_json(3, "sequence", "dominant_end", RIGHT_TO_LEFT);
        assert_eq!(
            (
                LayoutRegistry::decode(&invalid).is_err(),
                LayoutRegistry::decode(&quarantined.to_string()).is_err(),
                registry.decode_features(&rtl.to_string()).is_err(),
            ),
            (true, true, true),
            "invalid, quarantined, or RTL registry inputs passed production validation"
        );
    }

    #[test]
    fn registry_fails_before_feature_extraction_when_automatic_coverage_is_missing() {
        let mut source = serde_json::from_str::<Value>(REGISTRY_SOURCE)
            .expect("invariant: registry test fixture must decode");
        let splash = source["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
            .iter_mut()
            .find(|value| value["template_id"] == "splash-1-v1")
            .expect("invariant: splash layout must exist");
        splash["automatic_selection"] = Value::Bool(false);
        assert!(
            LayoutRegistry::decode(&source.to_string()).is_err(),
            "registry deferred a missing automatic hard tuple until after feature extraction"
        );
    }

    #[test]
    fn registry_rejects_a_nonboolean_dynamic_constraint() {
        let mut source = serde_json::from_str::<Value>(REGISTRY_SOURCE)
            .expect("invariant: registry test fixture must decode");
        let layout = source["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
            .iter_mut()
            .find(|template| template["template_id"] == "slanted-t-bottom-3-p2-v1")
            .expect("invariant: dynamic layout must exist");
        layout["dynamic_only"] = Value::String(String::from("yes"));
        assert!(
            LayoutRegistry::decode(&source.to_string()).is_err(),
            "registry accepted a dynamic constraint that cannot be enforced locally"
        );
    }

    #[test]
    fn hard_filter_supports_rising_contrast_coverage() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let three = expanded_feature_json(3, "contrast", "rising", "cause_reaction_detail");
        let four = expanded_feature_json(4, "contrast", "rising", "wide_detail_pair");
        let three = registry
            .decode_features(&three.to_string())
            .expect("three-shot contrast fixture must be valid");
        let four = registry
            .decode_features(&four.to_string())
            .expect("four-shot contrast fixture must be valid");
        assert_eq!(
            (
                registry.eligible(&three).is_ok(),
                registry.eligible(&four).is_ok(),
            ),
            (true, true),
            "rising contrast coverage has no compatible canonical layout"
        );
    }

    #[test]
    fn lenient_final_attempt_accepts_a_repeated_adjacent_camera_setup() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["shots"][1]["shot_scale"] = json!("wide");
        let strict = registry.decode_features(&raw.to_string()).is_err();
        let features = registry
            .decode_features_lenient(&raw.to_string(), true)
            .expect("lenient decode must accept a quality-only camera violation");
        assert_eq!(
            (strict, registry.eligible(&features).is_ok()),
            (true, true),
            "the lenient final scene attempt still rejected a repeated adjacent camera setup"
        );
    }

    #[test]
    fn registry_rejects_an_automatic_hard_tuple_gap() {
        let mut source = serde_json::from_str::<Value>(REGISTRY_SOURCE)
            .expect("invariant: registry test fixture must decode");
        for template in source["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
        {
            let covers_equal_two = template["automatic_selection"] == json!(true)
                && template["panel_count"] == json!(2)
                && template["feature_profile"]["emphasis_curve"]
                    .as_array()
                    .is_some_and(|values| values.contains(&json!("equal")));
            if covers_equal_two {
                template["feature_profile"]["temporal_relation"]
                    .as_array_mut()
                    .expect("invariant: temporal relations must be an array")
                    .retain(|value| value != "convergence");
            }
        }
        assert!(
            LayoutRegistry::decode(&source.to_string()).is_err(),
            "registry accepted a valid hard tuple with no automatic template"
        );
    }

    #[test]
    fn automatic_registry_covers_every_valid_hard_tuple() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let relations = ["sequence", "simultaneous", "contrast", "convergence"];
        let emphases = [
            "equal",
            "dominant_start",
            "dominant_end",
            "rising",
            "falling",
        ];
        let mut missing = Vec::new();
        for panel_count in 2..=4 {
            for relation in relations {
                for emphasis in emphases {
                    if panel_count == 2 && matches!(emphasis, "rising" | "falling") {
                        continue;
                    }
                    let raw = feature_json(panel_count, relation, emphasis, LEFT_TO_RIGHT);
                    let features = registry
                        .decode_features(&raw.to_string())
                        .expect("valid hard tuple fixture must decode");
                    if registry.eligible(&features).is_err() {
                        missing.push((panel_count, relation, emphasis));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "automatic registry leaves valid hard tuples uncovered: {missing:?}"
        );
    }

    #[test]
    fn empty_exact_tuple_uses_one_deterministic_same_count_fallback() {
        let mut registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("three-panel fallback fixture must be valid");
        registry.value["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
            .retain(|template| !template_matches(template, &features.value));
        let first = registry
            .eligible(&features)
            .expect("missing exact tuple must retain one same-count fallback");
        let second = registry
            .eligible(&features)
            .expect("same fallback input must remain selectable");
        let first_ids = first
            .templates
            .iter()
            .filter_map(|template| template["template_id"].as_str())
            .collect::<Vec<_>>();
        let second_ids = second
            .templates
            .iter()
            .filter_map(|template| template["template_id"].as_str())
            .collect::<Vec<_>>();
        let selection = first
            .rank()
            .expect("fallback ranking must build locally")
            .select("fallback-term", 0)
            .expect("fallback ranking must select");
        assert_eq!(
            (
                first_ids,
                second_ids,
                first.templates[0]["panel_count"].as_u64(),
                selection
                    .json()
                    .pointer("/layout_fallback/requested/panel_count")
                    .and_then(Value::as_u64),
                selection
                    .json()
                    .pointer("/layout_fallback/fallback_template_id")
                    .and_then(Value::as_str),
            ),
            (
                vec!["orthogonal-grid-3-v1"],
                vec!["orthogonal-grid-3-v1"],
                Some(3),
                Some(3),
                Some("orthogonal-grid-3-v1"),
            ),
            "nearest fallback changed panel count, choice, or provenance"
        );
    }

    #[test]
    fn empty_exact_tuple_fails_when_no_same_count_template_exists() {
        let mut registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("three-panel fallback fixture must be valid");
        registry.value["templates"]
            .as_array_mut()
            .expect("invariant: templates must be an array")
            .retain(|template| template["panel_count"] != 3);
        assert!(
            registry.eligible(&features).is_err(),
            "tuple fallback changed panel count when no safe layout remained"
        );
    }

    #[test]
    fn dynamic_only_three_panel_templates_cannot_reach_quiet_or_still_shortlists() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut dynamic = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        dynamic["motion_vector"] = Value::String(String::from("diagonal"));
        dynamic["intensity"] = Value::String(String::from("high"));
        let mut quiet = dynamic.clone();
        quiet["intensity"] = Value::String(String::from("quiet"));
        let mut still = dynamic.clone();
        still["motion_vector"] = Value::String(String::from("still"));
        let ids = |value: Value| {
            let features = registry
                .decode_features(&value.to_string())
                .expect("dynamic routing fixture must be valid");
            registry
                .eligible(&features)
                .expect("dynamic routing fixture must retain ordinary coverage")
                .templates
                .iter()
                .filter_map(|template| template["template_id"].as_str())
                .map(String::from)
                .collect::<BTreeSet<_>>()
        };
        let dynamic = ids(dynamic);
        let quiet = ids(quiet);
        let still = ids(still);
        assert_eq!(
            (
                dynamic.contains("slanted-t-bottom-3-p2-v1"),
                dynamic.contains("slanted-dominant-rail-3-p2-v1"),
                quiet
                    .iter()
                    .all(|id| !id.starts_with("slanted-") || !id.ends_with("-p2-v1")),
                still
                    .iter()
                    .all(|id| !id.starts_with("slanted-") || !id.ends_with("-p2-v1")),
            ),
            (true, true, true, true),
            "dynamic asymmetry either disappeared from action or leaked into quiet symmetric coverage"
        );
    }

    #[test]
    fn stronger_two_panel_payoff_cannot_reach_quiet_or_still_shortlists() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut dynamic = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        dynamic["motion_vector"] = Value::String(String::from("diagonal"));
        dynamic["intensity"] = Value::String(String::from("medium"));
        let mut quiet = dynamic.clone();
        quiet["intensity"] = Value::String(String::from("quiet"));
        let mut still = dynamic.clone();
        still["motion_vector"] = Value::String(String::from("still"));
        let eligible = |value: Value| {
            let features = registry
                .decode_features(&value.to_string())
                .expect("payoff routing fixture must be valid");
            registry
                .eligible(&features)
                .expect("payoff routing fixture must retain ordinary coverage")
                .templates
                .iter()
                .any(|template| template["template_id"] == "diagonal-split-2-end-strong-v1")
        };
        assert_eq!(
            (eligible(dynamic), eligible(quiet), eligible(still)),
            (true, false, false),
            "strong payoff geometry either disappeared from action or leaked into calm coverage"
        );
    }

    #[test]
    fn calm_scenes_retain_the_preexisting_symmetric_and_mild_candidates() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut three = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        three["intensity"] = Value::String(String::from("quiet"));
        let three = registry
            .decode_features(&three.to_string())
            .expect("calm three-panel fixture must be valid");
        let three = registry
            .eligible(&three)
            .expect("calm three-panel fixture must retain coverage");
        let mut two = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        two["intensity"] = Value::String(String::from("quiet"));
        let two = registry
            .decode_features(&two.to_string())
            .expect("calm two-panel fixture must be valid");
        let two = registry
            .eligible(&two)
            .expect("calm two-panel fixture must retain coverage");
        assert_eq!(
            (
                three
                    .templates
                    .iter()
                    .any(|template| template["template_id"] == "t-bottom-3-v1"),
                three
                    .templates
                    .iter()
                    .any(|template| template["template_id"] == "dominant-rail-3-v1"),
                two.templates
                    .iter()
                    .any(|template| { template["template_id"] == "diagonal-split-2-end-v1" }),
            ),
            (true, true, true),
            "dynamic routing removed the established calm or mild production alternatives"
        );
    }

    #[test]
    fn local_ranking_selects_the_same_layout_for_the_same_seed_and_attempt() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("deterministic ranking fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("deterministic ranking fixture must retain coverage");
        let first = eligible
            .rank()
            .expect("first local ranking must build")
            .select("stable-term", 1)
            .expect("first local ranking must select");
        let second = eligible
            .rank()
            .expect("second local ranking must build")
            .select("stable-term", 1)
            .expect("second local ranking must select");
        assert_eq!(
            (
                first.json()["chosen_template_id"].clone(),
                first.json()["ranked_candidates"].clone(),
            ),
            (
                second.json()["chosen_template_id"].clone(),
                second.json()["ranked_candidates"].clone(),
            ),
            "local ranking changed its choice or order for identical input"
        );
    }

    #[test]
    fn first_attempt_cannot_hash_away_from_the_best_two_panel_diagonal_fit() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(2, "sequence", "dominant_start", LEFT_TO_RIGHT);
        raw["motion_vector"] = json!("diagonal");
        raw["intensity"] = json!("high");
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-panel best-fit fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("two-panel best-fit fixture must retain coverage")
            .rank()
            .expect("two-panel best-fit fixture must rank locally");
        let choices = ["term-0", "term-1", "term-2"].map(|term| {
            ranking
                .select(term, 0)
                .expect("two-panel best fit must select")
                .summary["chosen_template_id"]
                .clone()
        });
        assert_eq!(
            (
                ranking.primary_count,
                ranking.candidates.len() > ranking.primary_count,
                choices,
            ),
            (
                1,
                true,
                [
                    json!("diagonal-split-2-v1"),
                    json!("diagonal-split-2-v1"),
                    json!("diagonal-split-2-v1"),
                ],
            ),
            "first-attempt hashing selected a weaker two-panel geometry"
        );
    }

    #[test]
    fn first_attempt_cannot_hash_away_from_the_best_calm_four_panel_fit() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(4, "sequence", "equal", LEFT_TO_RIGHT);
        raw["intensity"] = json!("quiet");
        raw["transition_type"] = json!("scene_to_scene");
        let features = registry
            .decode_features(&raw.to_string())
            .expect("calm four-panel best-fit fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("calm four-panel best-fit fixture must retain coverage")
            .rank()
            .expect("calm four-panel best-fit fixture must rank locally");
        let choices = ["term-0", "term-1", "term-2"].map(|term| {
            ranking
                .select(term, 0)
                .expect("calm four-panel best fit must select")
                .summary["chosen_template_id"]
                .clone()
        });
        assert_eq!(
            (
                ranking.primary_count,
                ranking.candidates.len() > ranking.primary_count,
                choices,
            ),
            (
                1,
                true,
                [
                    json!("vertical-strip-4-v1"),
                    json!("vertical-strip-4-v1"),
                    json!("vertical-strip-4-v1"),
                ],
            ),
            "first-attempt hashing selected a weaker calm four-panel geometry"
        );
    }

    #[test]
    fn strong_two_panel_layout_requires_a_setup_then_payoff_hierarchy() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut invalid = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        invalid["motion_vector"] = json!("diagonal");
        invalid["intensity"] = json!("high");
        let invalid_features = registry
            .decode_features(&invalid.to_string())
            .expect("invalid strong hierarchy fixture must remain structurally valid");
        let invalid = registry
            .eligible(&invalid_features)
            .expect("invalid strong hierarchy fixture must retain ordinary coverage");
        let hard_filter_kept = invalid
            .templates
            .iter()
            .any(|template| template["template_id"] == "diagonal-split-2-end-strong-v1");
        let invalid_ranking = invalid
            .rank()
            .expect("invalid strong hierarchy fixture must rank safely");
        let mut valid = invalid_features.value.clone();
        dynamic_hierarchy(&mut valid);
        let valid_features = registry
            .decode_features(&valid.to_string())
            .expect("valid strong hierarchy fixture must decode");
        let valid_ranking = registry
            .eligible(&valid_features)
            .expect("valid strong hierarchy fixture must retain coverage")
            .rank()
            .expect("valid strong hierarchy fixture must rank");
        assert_eq!(
            (
                hard_filter_kept,
                invalid_ranking.templates.iter().any(|template| {
                    template["template_id"] == "diagonal-split-2-end-strong-v1"
                }),
                valid_ranking.templates.iter().any(|template| {
                    template["template_id"] == "diagonal-split-2-end-strong-v1"
                }),
                valid_ranking
                    .select("strong-hierarchy", 0)
                    .expect("valid strong hierarchy must select")
                    .summary["chosen_template_id"]
                    .clone(),
            ),
            (true, false, true, json!("diagonal-split-2-end-strong-v1")),
            "strong two-panel geometry ignored its locally testable shot hierarchy"
        );
    }

    #[test]
    fn p2_three_panel_layouts_require_establish_action_payoff_hierarchy() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut invalid = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        invalid["motion_vector"] = json!("mixed");
        invalid["intensity"] = json!("high");
        let invalid_features = registry
            .decode_features(&invalid.to_string())
            .expect("invalid p2 hierarchy fixture must remain structurally valid");
        let invalid_ranking = registry
            .eligible(&invalid_features)
            .expect("invalid p2 hierarchy fixture must retain ordinary coverage")
            .rank()
            .expect("invalid p2 hierarchy fixture must rank safely");
        let mut valid = invalid_features.value.clone();
        dynamic_hierarchy(&mut valid);
        let valid_features = registry
            .decode_features(&valid.to_string())
            .expect("valid p2 hierarchy fixture must decode");
        let valid_ranking = registry
            .eligible(&valid_features)
            .expect("valid p2 hierarchy fixture must retain coverage")
            .rank()
            .expect("valid p2 hierarchy fixture must rank");
        let p2 = |ranking: &LayoutRanking| {
            ranking
                .templates
                .iter()
                .filter_map(|template| template["template_id"].as_str())
                .filter(|id| {
                    matches!(
                        *id,
                        "slanted-t-bottom-3-p2-v1" | "slanted-dominant-rail-3-p2-v1"
                    )
                })
                .count()
        };
        assert_eq!(
            (p2(&invalid_ranking), p2(&valid_ranking)),
            (0, 2),
            "three-panel p2 geometry ignored its locally testable shot hierarchy"
        );
    }

    #[test]
    fn local_ranking_gives_a_multi_panel_retry_distinct_geometry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "simultaneous", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("retry geometry fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("retry geometry fixture must retain coverage")
            .rank()
            .expect("retry geometry fixture must rank locally");
        let first = ranking
            .select("retry-geometry", 0)
            .expect("first retry geometry attempt must select");
        let retry = ranking
            .select("retry-geometry", 1)
            .expect("second retry geometry attempt must select");
        assert!(
            first.template["panels"] != retry.template["panels"],
            "local ranking repeated identical geometry on the first retry"
        );
    }

    #[test]
    fn local_ranking_preserves_and_selects_dynamic_candidates() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic local ranking fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("dynamic local ranking fixture must retain coverage")
            .rank()
            .expect("dynamic local ranking fixture must rank");
        let preserved = ranking.candidates.iter().any(|candidate| {
            ranked_template(candidate, &ranking.templates).is_ok_and(dynamic_only)
        });
        let selection = ranking
            .select("dynamic-local-ranking", 0)
            .expect("dynamic local ranking must select");
        assert!(
            preserved && dynamic_only(&selection.template),
            "local ranking dropped or failed to select a required dynamic candidate"
        );
    }

    #[test]
    fn local_ranking_never_selects_a_diagonal_family_for_calm_scenes() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("horizontal"));
        raw["intensity"] = Value::String(String::from("quiet"));
        let features = registry
            .decode_features(&raw.to_string())
            .expect("calm local ranking fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("calm local ranking fixture must retain coverage");
        let hard_filter_preserved = eligible
            .templates
            .iter()
            .any(|template| template["family"] == "diagonal_sequence");
        let ranking = eligible
            .rank()
            .expect("calm local ranking fixture must rank");
        let choices = [0, 1].map(|attempt| {
            ranking
                .select("calm-local-ranking", attempt)
                .expect("calm local ranking must select")
                .template
                .clone()
        });
        assert!(
            hard_filter_preserved
                && choices
                    .iter()
                    .all(|template| template["family"] != "diagonal_sequence")
                && choices[0]["panels"] != choices[1]["panels"],
            "calm ranking changed hard eligibility, selected a diagonal, or repeated retry geometry"
        );
    }

    #[test]
    fn approved_asymmetric_templates_preserve_the_reference_geometry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let panels = |id: &str| {
            registry.value["templates"]
                .as_array()
                .expect("invariant: templates must be an array")
                .iter()
                .find(|template| template["template_id"] == id)
                .map(|template| template["panels"].clone())
        };
        assert_eq!(
            (
                panels("slanted-t-bottom-3-p2-v1"),
                panels("slanted-dominant-rail-3-p2-v1"),
                panels("diagonal-split-2-end-strong-v1"),
            ),
            (
                Some(json!([
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 16, "y": 16, "width": 400, "height": 376},
                        "polygon": [[16, 16], [416, 16], [352, 392], [16, 392]]
                    },
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 368, "y": 16, "width": 640, "height": 376},
                        "polygon": [[432, 16], [1008, 16], [1008, 392], [368, 392]]
                    },
                    {
                        "shape": "wide_rectangle",
                        "bounds": {"x": 16, "y": 408, "width": 992, "height": 600},
                        "polygon": []
                    }
                ])),
                Some(json!([
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 16, "y": 16, "width": 300, "height": 432},
                        "polygon": [[16, 16], [316, 16], [316, 448], [16, 384]]
                    },
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 16, "y": 400, "width": 300, "height": 608},
                        "polygon": [[16, 400], [316, 464], [316, 1008], [16, 1008]]
                    },
                    {
                        "shape": "wide_rectangle",
                        "bounds": {"x": 332, "y": 16, "width": 676, "height": 992},
                        "polygon": []
                    }
                ])),
                Some(json!([
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 16, "y": 16, "width": 384, "height": 992},
                        "polygon": [[16, 16], [400, 16], [304, 1008], [16, 1008]]
                    },
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 320, "y": 16, "width": 688, "height": 992},
                        "polygon": [[416, 16], [1008, 16], [1008, 1008], [320, 1008]]
                    }
                ])),
            ),
            "production templates drifted away from the reference geometry"
        );
    }

    #[test]
    fn dynamic_materialization_embeds_the_reference_geometry_directive() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic directive fixture must be valid");
        let selection = registry
            .eligible(&features)
            .expect("dynamic directive layout must be eligible")
            .rank()
            .expect("dynamic directive layout must rank locally")
            .select("dynamic-directive", 0)
            .expect("dynamic directive layout must select");
        let mut scene = composer_scene(3);
        materialize(&mut scene, &selection).expect("dynamic directive layout must materialize");
        assert!(
            scene
                .pointer("/manga_panel/page_design/layout_rendering_directive")
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    value.contains("p1 is visibly smaller than p2")
                        && value.contains("Never straighten that upper divider")
                }),
            "materialized dynamic scene lost its reference geometry-specific rendering directive"
        );
    }

    #[test]
    fn fnv_term_choice_is_stable_and_distributed() {
        let slots = ["term-0", "term-1", "term-2"].map(|term| selection_slot(term, 3));
        assert_eq!(
            slots,
            [0, 1, 2],
            "stable term hashing no longer distributes the three-candidate shortlist"
        );
    }

    #[test]
    fn lower_ranked_dynamic_emphasis_wins_over_ordinary_candidates() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic ranking fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("dynamic ranking fixture must retain coverage")
            .rank()
            .expect("dynamic ranking must build locally");
        let selection = ranking
            .select("get my brother to help", 0)
            .expect("dynamic ranking must select");
        assert_eq!(
            (
                selection.json()["chosen_template_id"].as_str(),
                selection.json()["deterministic_slot"].as_u64(),
            ),
            (Some("slanted-t-bottom-3-p2-v1"), Some(0)),
            "ordinary rank order displaced a required dynamic emphasis candidate"
        );
    }

    #[test]
    fn local_ranking_does_not_force_dynamic_below_the_best_soft_fit() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic omission fixture must be valid");
        let mut eligible = registry
            .eligible(&features)
            .expect("dynamic restoration fixture must retain coverage");
        for template in eligible
            .templates
            .iter_mut()
            .filter(|template| dynamic_only(template))
        {
            template["feature_profile"]["motion_vector"] = json!(["vertical"]);
            template["feature_profile"]["intensity"] = json!(["medium"]);
            template["feature_profile"]["spatial_relation"] = json!(["parallel_spaces"]);
            template["feature_profile"]["transition_type"] = json!(["subject_to_subject"]);
        }
        let ranking = eligible
            .rank()
            .expect("weak dynamic fixture must rank locally");
        let selection = ranking
            .select("weak-dynamic", 0)
            .expect("weak dynamic fixture must select");
        let primary = ranking
            .candidates
            .get(..ranking.primary_count)
            .expect("invariant: primary shortlist must remain bounded");
        assert!(
            !dynamic_only(&selection.template)
                && primary.iter().all(|candidate| {
                    ranked_template(candidate, &ranking.templates).is_ok_and(|template| {
                        !dynamic_only(template)
                            && candidate["reason"]
                                .as_str()
                                .is_some_and(|reason| reason.contains("matched 4 of 4"))
                    })
                }),
            "a weak dynamic layout displaced the best ordinary soft fit"
        );
    }

    #[test]
    fn multiple_dynamic_candidates_retain_diversity_inside_dynamic_subset() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("mixed"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic subset fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("dynamic subset fixture must retain coverage")
            .rank()
            .expect("dynamic subset ranking must build locally");
        let choices = ["term-0", "term-1"].map(|term| {
            ranking
                .select(term, 0)
                .expect("dynamic subset ranking must select")
                .json()["chosen_template_id"]
                .as_str()
                .map(String::from)
        });
        assert_eq!(
            choices,
            [
                Some(String::from("slanted-t-bottom-3-p2-v1")),
                Some(String::from("slanted-dominant-rail-3-p2-v1")),
            ],
            "term diversity escaped the ranked dynamic-only subset"
        );
    }

    #[test]
    fn ordinary_ranked_templates_retain_term_diversity() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("ordinary ranking fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("ordinary ranking fixture must retain coverage")
            .rank()
            .expect("ordinary ranking must build locally");
        let choices = ["term-0", "term-1"].map(|term| {
            ranking
                .select(term, 0)
                .expect("ordinary ranking must select")
                .json()["chosen_template_id"]
                .as_str()
                .map(String::from)
        });
        assert_eq!(
            choices,
            [
                Some(String::from("equal-split-vertical-2-v1")),
                Some(String::from("equal-split-horizontal-2-v1")),
            ],
            "dynamic winner protection disabled diversity for ordinary rankings"
        );
    }

    #[test]
    fn scene_retries_rotate_the_ranked_slot_deterministically() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("retry ranking fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("retry ranking fixture must retain coverage")
            .rank()
            .expect("retry ranking must build locally");
        let first = ranking
            .select("term-0", 0)
            .expect("first scene attempt must select");
        let retry = ranking
            .select("term-0", 1)
            .expect("second scene attempt must select");
        let repeated = ranking
            .select("term-0", 1)
            .expect("same scene retry must remain deterministic");
        assert_eq!(
            (
                first.json()["chosen_template_id"].as_str(),
                retry.json()["chosen_template_id"].as_str(),
                repeated.json()["chosen_template_id"].as_str(),
                retry.json()["scene_attempt_index"].as_u64(),
            ),
            (
                Some("equal-split-vertical-2-v1"),
                Some("equal-split-horizontal-2-v1"),
                Some("equal-split-horizontal-2-v1"),
                Some(1),
            ),
            "scene retry reused the first slot or lost deterministic attempt provenance"
        );
    }

    #[test]
    fn scene_retries_stay_inside_a_required_dynamic_subset() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("mixed"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dynamic retry fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("dynamic retry fixture must retain coverage")
            .rank()
            .expect("dynamic retry ranking must build locally");
        let first = ranking
            .select("dynamic-retry", 0)
            .expect("first dynamic attempt must select");
        let retry = ranking
            .select("dynamic-retry", 1)
            .expect("second dynamic attempt must select");
        let first = first.json()["chosen_template_id"].as_str();
        let retry = retry.json()["chosen_template_id"].as_str();
        assert!(
            matches!(
                first,
                Some("slanted-t-bottom-3-p2-v1" | "slanted-dominant-rail-3-p2-v1")
            ) && matches!(
                retry,
                Some("slanted-t-bottom-3-p2-v1" | "slanted-dominant-rail-3-p2-v1")
            ) && first != retry,
            "scene retry escaped or failed to rotate the required dynamic subset"
        );
    }

    #[test]
    fn scene_retry_escapes_a_single_dynamic_primary_deterministically() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        dynamic_hierarchy(&mut raw);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("single-candidate retry fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("single-candidate retry fixture must retain coverage")
            .rank()
            .expect("single dynamic primary must rank locally");
        let choices = [0, 1, 1].map(|attempt| {
            ranking
                .select("single-dynamic-retry", attempt)
                .expect("scene attempt must select")
                .json()["chosen_template_id"]
                .as_str()
                .map(String::from)
        });
        assert!(
            choices[0] == Some(String::from("diagonal-split-2-end-strong-v1"))
                && choices[0] != choices[1]
                && choices[1] == choices[2],
            "a single dynamic primary defeated deterministic retry variation"
        );
    }

    #[test]
    fn multi_panel_singleton_tuple_gets_a_distinct_retry_geometry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "simultaneous", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("singleton tuple fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("singleton tuple must retain its canonical layout")
            .rank()
            .expect("singleton tuple ranking must build locally");
        let choices = [0, 1].map(|attempt| {
            ranking
                .select("singleton-tuple", attempt)
                .expect("singleton scene attempt must select")
                .json()["chosen_template_id"]
                .as_str()
                .map(String::from)
        });
        assert!(
            choices[0] == Some(String::from("equal-split-vertical-2-v1"))
                && choices[1].is_some()
                && choices[0] != choices[1],
            "a multi-panel singleton tuple repeated identical geometry on retry"
        );
    }

    #[test]
    fn feature_decoder_rejects_cross_field_contradictions() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let one_contrast = feature_json(1, "contrast", "equal", LEFT_TO_RIGHT);
        let two_rising = feature_json(2, "sequence", "rising", LEFT_TO_RIGHT);
        let mut multi_none = feature_json(3, "sequence", "equal", LEFT_TO_RIGHT);
        multi_none["transition_type"] = Value::String(String::from("none"));
        assert_eq!(
            (
                registry.decode_features(&one_contrast.to_string()).is_err(),
                registry.decode_features(&two_rising.to_string()).is_err(),
                registry.decode_features(&multi_none.to_string()).is_err(),
            ),
            (true, true, true),
            "cross-field feature contradictions reached layout filtering"
        );
    }

    #[test]
    fn feature_decoder_repairs_a_multi_beat_simultaneous_single_moment_label() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "simultaneous", "dominant_end", LEFT_TO_RIGHT);
        features["semantic_relation"] = json!("single_moment");
        features["decomposition_mode"] = json!("wide_detail_pair");
        features["transition_type"] = json!("aspect_to_aspect");
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("the narrow simultaneous-relation repair must decode");
        assert_eq!(
            decoded.value["semantic_relation"],
            json!("simultaneous"),
            "coexisting semantic beats kept the model's contradictory single-moment label"
        );
    }

    #[test]
    fn feature_decoder_rejects_single_moment_labels_outside_the_narrow_repair() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut non_simultaneous = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        non_simultaneous["semantic_relation"] = json!("single_moment");
        non_simultaneous["decomposition_mode"] = json!("wide_detail_pair");
        non_simultaneous["transition_type"] = json!("aspect_to_aspect");
        let mut non_aspect = feature_json(2, "simultaneous", "dominant_end", LEFT_TO_RIGHT);
        non_aspect["semantic_relation"] = json!("single_moment");
        non_aspect["decomposition_mode"] = json!("wide_detail_pair");
        non_aspect["transition_type"] = json!("subject_to_subject");
        assert_eq!(
            (
                registry
                    .decode_features(&non_simultaneous.to_string())
                    .is_err(),
                registry.decode_features(&non_aspect.to_string()).is_err(),
            ),
            (true, true),
            "a contradiction outside the simultaneous aspect repair escaped fail-fast validation"
        );
    }

    #[test]
    fn feature_decoder_separates_semantic_beats_from_cinematic_shots() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let features = json!({
            "semantic_beat_count": 2,
            "semantic_relation": "sequence",
            "coverage_audit": coverage_audit(4),
            "panel_count": 4,
            "panel_relation": "simultaneous",
            "panel_emphasis": "equal",
            "decomposition_mode": "wide_detail_pair",
            "motion_vector": "still",
            "intensity": "medium",
            "spatial_relation": "same_space",
            "transition_type": "aspect_to_aspect",
            "reading_direction": LEFT_TO_RIGHT,
            "literal_anchor": "a tested machine continues operating",
            "camera_arc": camera_arc(4),
            "shots": [
                cinematic_shot(1, 1, "establishing", "the complete test chamber", "the system is tested", 4),
                cinematic_shot(2, 1, "action", "the load pressing on one component", "the system is tested", 4),
                cinematic_shot(3, 2, "action", "the mechanism continuing to turn", "the system remains reliable", 4),
                cinematic_shot(4, 2, "payoff", "an intact coupling under load", "the system remains reliable", 4)
            ],
            "selection_logic": "two literal beats need paired context and evidence views without a new event"
        });
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "cinematic coverage remains falsely coupled to semantic chronology"
        );
    }

    #[test]
    fn feature_decoder_allows_a_camera_mode_when_counts_match() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["decomposition_mode"] = Value::String(String::from("wide_detail_pair"));
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "matching semantic and panel counts still force a one-to-one camera mode"
        );
    }

    #[test]
    fn feature_decoder_accepts_consistent_coverage_audit() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let features = expanded_feature_json(4, "contrast", "rising", "wide_detail_pair");
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "a complete ordered coverage audit failed feature validation"
        );
    }

    #[test]
    fn feature_decoder_repairs_a_reopened_later_shot() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["shots"][1]["transition_trigger"] = json!("scene_open");
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("a safe trigger repair must decode");
        assert_eq!(
            decoded.value["shots"][1]["transition_trigger"],
            json!("new_action"),
            "a later camera setup kept reopening the scene"
        );
    }

    #[test]
    fn feature_decoder_repairs_an_unanchored_subjective_viewpoint() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["shots"][1]["viewpoint"] = json!("subjective");
        features["shots"][1]["viewpoint_anchor"] = json!("   ");
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("an ungrounded viewpoint must downgrade safely");
        assert_eq!(
            (
                &decoded.value["shots"][1]["viewpoint"],
                &decoded.value["shots"][1]["viewpoint_anchor"]
            ),
            (&json!("objective"), &json!("")),
            "an unanchored subjective viewpoint survived normalization"
        );
    }

    #[test]
    fn feature_decoder_repairs_an_unsupported_insert_setup() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["shots"][1]["framing"] = json!("insert");
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("an unsupported insert must downgrade safely");
        assert_eq!(
            decoded.value["shots"][1]["framing"],
            json!("single"),
            "an unsupported insert framing survived normalization"
        );
    }

    #[test]
    fn feature_decoder_tolerates_repeated_camera_information_gain() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["shots"][1]["information_gain"] = features["shots"][0]["information_gain"].clone();
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "repeated descriptive information gain still terminates a valid shot plan"
        );
    }

    #[test]
    fn feature_decoder_allows_a_deliberately_stable_hold() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["camera_arc"]["strategy"] = json!("hold");
        features["shots"][1]["shot_scale"] = features["shots"][0]["shot_scale"].clone();
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "a motivated stable camera hold was mistaken for flat coverage"
        );
    }

    #[test]
    fn feature_decoder_accepts_a_widening_reveal() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        features["camera_arc"]["strategy"] = json!("pull_back_reveal");
        features["shots"][0]["shot_scale"] = json!("close");
        features["shots"][1]["shot_scale"] = json!("wide");
        assert!(
            registry.decode_features(&features.to_string()).is_ok(),
            "a motivated pull-back reveal was forced into a universal push-in"
        );
    }

    #[test]
    fn feature_decoder_reclassifies_a_mislabeled_detail_return() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = feature_json(3, "sequence", "rising", LEFT_TO_RIGHT);
        features["shots"][0]["shot_scale"] = json!("medium");
        features["shots"][1]["shot_scale"] = json!("close");
        features["shots"][2]["shot_scale"] = json!("medium_close");
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("a motivated detail return must remain usable");
        assert_eq!(
            decoded.value["camera_arc"]["strategy"].as_str(),
            Some("wide_detail_return"),
            "a useful detail-return setup was discarded because Gemini mislabeled it as push-in"
        );
    }

    #[test]
    fn feature_decoder_normalizes_cosmetic_coverage_audit_disagreement() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut features = expanded_feature_json(4, "contrast", "rising", "wide_detail_pair");
        features["coverage_audit"] = json!([
            {
                "panel_count": 4,
                "added_view": "",
                "source_support": null,
                "verdict": "insufficient",
                "reason": ""
            },
            {"unexpected": "cosmetic model prose"}
        ]);
        let decoded = registry
            .decode_features(&features.to_string())
            .expect("cosmetic coverage audit drift must normalize locally");
        let audit = decoded.value["coverage_audit"]
            .as_array()
            .expect("invariant: normalized audit must be an array")
            .iter()
            .map(|entry| {
                (
                    entry["panel_count"].as_u64(),
                    entry["verdict"].as_str(),
                    entry["added_view"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                    entry["source_support"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                    entry["reason"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            audit,
            vec![
                (Some(1), Some("insufficient"), true, true, true),
                (Some(2), Some("insufficient"), true, true, true),
                (Some(3), Some("insufficient"), true, true, true),
                (Some(4), Some("selected"), true, true, true),
            ],
            "coverage audit was not rebuilt from authoritative panel_count"
        );
    }

    #[test]
    fn feature_decoder_tolerates_duplicate_prose_but_rejects_uncovered_beats() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut duplicated = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        duplicated["shots"][1]["visible_anchor"] = Value::String(String::from("visible beat 1"));
        duplicated["shots"][1]["information_gain"] =
            duplicated["shots"][0]["information_gain"].clone();
        let mut uncovered = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        uncovered["shots"][1]["semantic_beat_index"] = Value::Number(1_u64.into());
        assert_eq!(
            (
                registry.decode_features(&duplicated.to_string()).is_ok(),
                registry.decode_features(&uncovered.to_string()).is_err(),
            ),
            (true, true),
            "descriptive duplicates still fail or a semantic beat can remain uncovered"
        );
    }

    #[test]
    fn feature_contract_stays_registry_blind_and_local_ranking_has_reasons() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let data = feature_prompt_data("English", "outlier", "This point is an outlier")
            .expect("feature prompt data must validate");
        let prompt = render_feature_prompt(&data).expect("feature prompt must render");
        let schema = registry
            .feature_schema()
            .expect("feature schema must build");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let ranking = eligible.rank().expect("local ranking must build");
        assert_eq!(
            (
                schema.pointer("/properties/scene_features").is_none(),
                prompt.contains("template_id"),
                prompt.contains("diagonal-split-2-v1"),
                ranking.candidates.len(),
                ranking.candidates.iter().all(|candidate| {
                    candidate["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.trim().is_empty())
                }),
                features.value["semantic_beat_count"].as_u64(),
            ),
            (true, false, false, 2, true, Some(2)),
            "feature extraction or local ranking crossed its registry isolation boundary"
        );
    }

    #[test]
    fn composer_receives_no_canonical_geometry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let selection = eligible
            .rank()
            .expect("canonical ranking must build locally")
            .select("geometry-blind", 0)
            .expect("canonical ranking must select");
        let card = selection
            .composer_card()
            .expect("geometry-free composer card must build");
        assert_eq!(
            (
                card.get("panels").is_none(),
                card.get("grouping_tree").is_none(),
                serde_json::to_string(&card)
                    .expect("composer card must serialize")
                    .contains("polygon"),
            ),
            (true, true, false),
            "canonical geometry leaked into the semantic composer request"
        );
    }

    #[test]
    fn ranking_deduplicates_geometry_before_taking_the_top_three() {
        let mut source = serde_json::from_str::<Value>(REGISTRY_SOURCE)
            .expect("invariant: registry test fixture must decode");
        let alias = source["templates"]
            .as_array()
            .expect("invariant: templates must be an array")
            .iter()
            .find(|value| value["template_id"] == "equal-split-vertical-2-v1")
            .expect("invariant: equal split layout must exist")
            .clone();
        for (index, id) in [
            "equal-split-vertical-2-alias-a-v1",
            "equal-split-vertical-2-alias-b-v1",
        ]
        .iter()
        .enumerate()
        {
            let mut alias = alias.clone();
            alias["template_id"] = Value::String(String::from(*id));
            source["templates"]
                .as_array_mut()
                .expect("invariant: templates must be an array")
                .insert(index + 2, alias);
        }
        let registry = LayoutRegistry::decode(&source.to_string())
            .expect("geometry alias registry must remain structurally valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let ranking = eligible.rank().expect("geometry aliases must rank locally");
        let ids = ranking
            .candidates
            .iter()
            .filter_map(|candidate| candidate["template_id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec!["equal-split-vertical-2-v1", "equal-split-horizontal-2-v1"],
            "duplicate geometry consumed the local top-three shortlist"
        );
    }

    #[test]
    fn ranking_extends_one_viable_candidate_with_one_distinct_retry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "simultaneous", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("singleton feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("singleton layout must be eligible");
        let selection = eligible
            .rank()
            .expect("singleton layout must rank locally")
            .select("unusual-term", 0)
            .expect("one candidate must be selectable");
        assert_eq!(
            selection.json()["ranked_candidates"]
                .as_array()
                .map(Vec::len),
            Some(2),
            "selection failed to restore exactly one distinct retry alternative"
        );
    }

    #[test]
    fn materialization_replaces_every_topology_field_with_registry_values() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let selection = eligible
            .rank()
            .expect("canonical ranking must build locally")
            .select("materialize", 0)
            .expect("canonical ranking must select");
        let mut scene = composer_scene(2);
        materialize(&mut scene, &selection).expect("canonical layout must materialize");
        assert_eq!(
            (
                scene
                    .pointer("/manga_panel/panels/0/id")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/bounds/width")
                    .and_then(Value::as_i64),
                scene
                    .pointer("/manga_panel/panels/1/bounds/x")
                    .and_then(Value::as_i64),
                scene
                    .pointer("/manga_panel/panels/0/frame/shape")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/reading_path/1")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/dominant_panel")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/special_device/kind")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/constraints/maximum_panels")
                    .and_then(Value::as_u64),
                scene
                    .pointer("/manga_panel/constraints/panel_count_lock")
                    .and_then(Value::as_bool),
                scene
                    .pointer("/manga_panel/meta/layout_selection/chosen_template_id")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/meta/layout_selection/eligible_template_ids")
                    .and_then(Value::as_array)
                    .map(Vec::len),
            ),
            (
                Some("p1"),
                Some(488),
                Some(520),
                Some("tall_rectangle"),
                Some("p2"),
                Some(""),
                Some("none"),
                Some(2),
                Some(true),
                Some("equal-split-vertical-2-v1"),
                Some(2),
            ),
            "semantic composer topology survived canonical materialization"
        );
    }

    #[test]
    fn composer_card_exposes_only_qualified_devices_compatible_with_the_layout() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let card = selection
            .composer_card()
            .expect("device-aware composer card must build");
        let devices = card["device_candidates"]
            .as_array()
            .expect("invariant: composer devices must be an array")
            .iter()
            .map(|value| {
                (
                    value["scene_kind"]
                        .as_str()
                        .expect("invariant: candidate scene kind must be a string"),
                    value["capability_status"]
                        .as_str()
                        .expect("invariant: candidate status must be a string"),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            devices,
            BTreeSet::from([("crossing", "proven"), ("none", "qualified")]),
            "composer card exposed a device that is not qualified for automatic selection"
        );
    }

    #[test]
    fn automatic_catalog_excludes_every_qualification_required_device() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let devices = registry.value["templates"]
            .as_array()
            .expect("invariant: templates must be an array")
            .iter()
            .filter(|template| {
                template["automatic_selection"].as_bool() == Some(true)
                    && matches!(
                        template["capability_status"].as_str(),
                        Some("candidate" | "qualified")
                    )
            })
            .flat_map(|template| {
                device_candidates(template)
                    .expect("automatic device catalog must build")
                    .as_array()
                    .expect("invariant: device candidates must be an array")
                    .clone()
            })
            .filter_map(|candidate| candidate["scene_kind"].as_str().map(String::from))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            devices,
            BTreeSet::from([String::from("crossing"), String::from("none")]),
            "qualification-required device remained reachable through an automatic layout"
        );
    }

    #[test]
    fn dynamic_template_device_allowlist_constrains_the_operational_catalog() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let template = registry.value["templates"]
            .as_array()
            .expect("invariant: templates must be an array")
            .iter()
            .find(|value| value["template_id"] == "slanted-dominant-rail-3-p2-v1")
            .expect("invariant: reviewed dynamic rail must exist");
        let candidates =
            device_candidates(template).expect("dynamic rail device catalog must build");
        let devices = candidates
            .as_array()
            .expect("invariant: composer devices must be an array")
            .iter()
            .filter_map(|value| value["scene_kind"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            devices,
            BTreeSet::from(["none"]),
            "dynamic layout exposed an unqualified device through its allowlist"
        );
    }

    #[test]
    fn irregular_grid_device_catalog_excludes_nonadjacent_reading_and_geometry_pairs() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let template = registry.value["templates"]
            .as_array()
            .expect("invariant: templates must be an array")
            .iter()
            .find(|value| value["template_id"] == "grid-2x2-4-v1")
            .expect("invariant: four-panel grid must exist");
        let pairs = |kind: &str| {
            device_references(kind, template)
                .expect("device reference catalog must build")
                .iter()
                .map(|value| {
                    format!(
                        "{}>{}",
                        value["source_panel"]
                            .as_str()
                            .expect("invariant: source must be a string"),
                        value["target_panel"]
                            .as_str()
                            .expect("invariant: target must be a string")
                    )
                })
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            (pairs("crossing"), pairs("overlap")),
            (
                BTreeSet::from([String::from("s1>s2"), String::from("s3>s4")]),
                BTreeSet::from([
                    String::from("s1>s2"),
                    String::from("s1>s3"),
                    String::from("s2>s1"),
                    String::from("s2>s4"),
                    String::from("s3>s1"),
                    String::from("s3>s4"),
                    String::from("s4>s2"),
                    String::from("s4>s3"),
                ]),
            ),
            "device catalog offered a crossing or overlap across an intervening grid slot"
        );
    }

    #[test]
    fn device_catalog_excludes_unsafe_diagonal_staggered_and_spanning_relations() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let template = |id: &str| {
            registry.value["templates"]
                .as_array()
                .expect("invariant: templates must be an array")
                .iter()
                .find(|value| value["template_id"] == id)
                .expect("invariant: requested layout must exist")
        };
        let candidates = |id: &str| {
            device_candidates(template(id))
                .expect("device catalog must build")
                .as_array()
                .expect("invariant: device catalog must be an array")
                .clone()
        };
        let kinds = |id: &str| {
            candidates(id)
                .iter()
                .filter_map(|value| value["scene_kind"].as_str().map(String::from))
                .collect::<BTreeSet<_>>()
        };
        let overlaps = |id: &str| {
            device_references("overlap", template(id))
                .expect("overlap reference catalog must build")
                .iter()
                .map(|value| {
                    format!(
                        "{}>{}",
                        value["source_panel"]
                            .as_str()
                            .expect("invariant: source must be a string"),
                        value["target_panel"]
                            .as_str()
                            .expect("invariant: target must be a string")
                    )
                })
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            (
                kinds("diagonal-strip-3-v1").contains("crossing"),
                kinds("diagonal-strip-3-v1").contains("diagonal_release"),
                kinds("staggered-grid-4-v1").contains("open_frame"),
                overlaps("t-top-3-v1"),
                overlaps("dominant-rail-3-v1"),
            ),
            (
                false,
                false,
                false,
                BTreeSet::from([
                    String::from("s2>s1"),
                    String::from("s2>s3"),
                    String::from("s3>s1"),
                    String::from("s3>s2"),
                ]),
                BTreeSet::from([
                    String::from("s1>s2"),
                    String::from("s1>s3"),
                    String::from("s2>s1"),
                    String::from("s2>s3"),
                ]),
            ),
            "device catalog exposed an unsupported edge or an overlap spanning a third panel"
        );
    }

    #[test]
    fn materialization_executes_crossing_from_model_selected_shot_references() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "crossing",
                "reason": "one actor visibly continues through the adjacent beat",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        );
        materialize(&mut scene, &selection).expect("eligible crossing must materialize");
        assert_eq!(
            (
                scene
                    .pointer("/manga_panel/page_design/special_device/source_panel")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/special_device/target_panel")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/continuity/breakout/enabled")
                    .and_then(Value::as_bool),
                scene
                    .pointer("/manga_panel/panels/0/continuity/breakout/subject_id")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/continuity/breakout/destination_panel")
                    .and_then(Value::as_str),
            ),
            (
                Some("p1"),
                Some("p2"),
                Some(true),
                Some("actor"),
                Some("p2")
            ),
            "crossing choice did not become one canonical breakout relation"
        );
    }

    #[test]
    fn materialization_accepts_one_current_candidate_device_id_alias() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "character_crossing",
                "reason": "one actor visibly continues through the adjacent beat",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        );
        materialize(&mut scene, &selection).expect("candidate device id alias must materialize");
        assert_eq!(
            (
                scene
                    .pointer("/manga_panel/page_design/special_device/kind")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/continuity/breakout/enabled")
                    .and_then(Value::as_bool),
            ),
            (Some("crossing"), Some(true)),
            "current layout candidate device id did not canonicalize to its scene kind"
        );
    }

    #[test]
    fn recovery_scene_materialization_disables_structural_devices() {
        let mut selection = selected_layout("equal-split-vertical-2-v1", "equal");
        selection.summary["scene_attempt_index"] = json!(1);
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "crossing",
                "reason": "one actor visibly continues through the adjacent beat",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        );
        materialize(&mut scene, &selection).expect("recovery scene must materialize");
        assert_eq!(
            scene
                .pointer("/manga_panel/page_design/special_device/kind")
                .and_then(Value::as_str),
            Some("none"),
            "recovery scene retained a failure-prone structural device"
        );
    }

    #[test]
    fn materialization_executes_inset_overlap_and_open_frame_geometry() {
        let dominant =
            selected_layout_with_device("dominant-split-2-v1", "dominant_start", "inset");
        let overlap_selection =
            selected_layout_with_device("equal-split-vertical-2-v1", "equal", "overlap");
        let open_selection =
            selected_layout_with_device("equal-split-vertical-2-v1", "equal", "open_frame");
        let mut inset = device_composer_scene(
            2,
            json!({
                "kind": "inset",
                "reason": "the second shot is a decisive detail inside the first",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": ""
            }),
        );
        let mut overlap = device_composer_scene(
            2,
            json!({
                "kind": "overlap",
                "reason": "the near-simultaneous views compress into one another",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": ""
            }),
        );
        let mut open = device_composer_scene(
            2,
            json!({
                "kind": "open_frame",
                "reason": "the first atmospheric view dissolves into its environment",
                "source_panel": "s1",
                "target_panel": "",
                "subject_id": ""
            }),
        );
        materialize(&mut inset, &dominant).expect("eligible inset must materialize");
        materialize(&mut overlap, &overlap_selection).expect("eligible overlap must materialize");
        materialize(&mut open, &open_selection).expect("eligible open frame must materialize");
        assert_eq!(
            (
                inset
                    .pointer("/manga_panel/panels/1/frame/shape")
                    .and_then(Value::as_str),
                inset
                    .pointer("/manga_panel/panels/1/frame/parent_panel")
                    .and_then(Value::as_str),
                inset
                    .pointer("/manga_panel/panels/1/frame/z_index")
                    .and_then(Value::as_i64),
                inset
                    .pointer("/manga_panel/panels/0/bounds/width")
                    .and_then(Value::as_i64),
                overlap
                    .pointer("/manga_panel/panels/0/frame/overlaps_panel")
                    .and_then(Value::as_str),
                overlap
                    .pointer("/manga_panel/panels/0/frame/z_index")
                    .and_then(Value::as_i64),
                open.pointer("/manga_panel/panels/0/frame/shape")
                    .and_then(Value::as_str),
                open.pointer("/manga_panel/panels/0/frame/border")
                    .and_then(Value::as_str),
                open.pointer("/manga_panel/page_design/layout_rendering_directive")
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        value.starts_with(OUTER_WHITE_BAND)
                            && value.contains("never becomes page bleed")
                    }),
            ),
            (
                Some("inset"),
                Some("p1"),
                Some(1),
                Some(992),
                Some("p2"),
                Some(1),
                Some("open_frame"),
                Some("none"),
                true,
            ),
            "model-selected structural devices did not specialize canonical geometry"
        );
    }

    #[test]
    fn materialization_preserves_master_view_and_diagonal_release_contracts() {
        let equal =
            selected_layout_with_device("equal-split-vertical-2-v1", "equal", "master_view");
        let diagonal = selected_layout_with_device(
            "diagonal-split-2-v1",
            "dominant_start",
            "diagonal_release",
        );
        let mut master = device_composer_scene(
            2,
            json!({
                "kind": "master_view",
                "reason": "one actor advances through two phases of the same place",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        );
        let mut release = device_composer_scene(
            2,
            json!({
                "kind": "diagonal_release",
                "reason": "the diagonal route carries the decisive directional change",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": ""
            }),
        );
        materialize(&mut master, &equal).expect("eligible master view must materialize");
        materialize(&mut release, &diagonal).expect("eligible diagonal release must materialize");
        assert_eq!(
            (
                master
                    .pointer("/manga_panel/page_design/special_device/kind")
                    .and_then(Value::as_str),
                master
                    .pointer("/manga_panel/page_design/special_device/source_panel")
                    .and_then(Value::as_str),
                master
                    .pointer("/manga_panel/page_design/special_device/target_panel")
                    .and_then(Value::as_str),
                release
                    .pointer("/manga_panel/page_design/special_device/kind")
                    .and_then(Value::as_str),
                release
                    .pointer("/manga_panel/page_design/special_device/source_panel")
                    .and_then(Value::as_str),
                release
                    .pointer("/manga_panel/page_design/special_device/target_panel")
                    .and_then(Value::as_str),
            ),
            (
                Some("master_view"),
                Some("p1"),
                Some("p2"),
                Some("diagonal_release"),
                Some("p1"),
                Some("p2"),
            ),
            "continuity or diagonal device was erased during materialization"
        );
    }

    #[test]
    fn invalid_master_view_continuity_falls_back_to_canonical_none() {
        let selection =
            selected_layout_with_device("equal-split-vertical-2-v1", "equal", "master_view");
        let device = json!({
            "kind": "master_view",
            "reason": "one actor advances through two phases of the same place",
            "source_panel": "s1",
            "target_panel": "s2",
            "subject_id": "actor"
        });
        let mut environment = device_composer_scene(2, device.clone());
        environment["manga_panel"]["panels"][0]["continuity"]["shared_environment_id"] =
            Value::String(String::from("   "));
        environment["manga_panel"]["panels"][1]["continuity"]["shared_environment_id"] =
            Value::String(String::from("   "));
        let mut phases = device_composer_scene(2, device);
        phases["manga_panel"]["panels"][0]["continuity"]["subject_phase"] =
            Value::String(String::from(" "));
        phases["manga_panel"]["panels"][1]["continuity"]["subject_phase"] =
            Value::String(String::from("  "));
        materialize(&mut environment, &selection)
            .expect("invalid master-view environment must degrade");
        materialize(&mut phases, &selection).expect("invalid master-view phases must degrade");
        assert_eq!(
            (
                environment["manga_panel"]["page_design"]["special_device"]["kind"].as_str(),
                phases["manga_panel"]["page_design"]["special_device"]["kind"].as_str(),
            ),
            (Some("none"), Some("none")),
            "invalid master-view continuity still terminated materialization"
        );
    }

    #[test]
    fn absent_model_device_falls_back_to_canonical_none() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = device_composer_scene(2, json!({}));
        scene["manga_panel"]["page_design"]
            .as_object_mut()
            .expect("invariant: page design must be an object")
            .remove("special_device");
        materialize(&mut scene, &selection).expect("absent model device must degrade");
        assert_eq!(
            (
                scene["manga_panel"]["page_design"]["special_device"]["kind"].as_str(),
                scene["manga_panel"]["page_design"]["special_device"]["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("omitted special_device"))
            ),
            (Some("none"), true),
            "absent model device still terminated materialization"
        );
    }

    #[test]
    fn crossing_with_an_absent_subject_falls_back_to_canonical_none() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "crossing",
                "reason": "one actor crosses the canonical divider",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "missing-actor"
            }),
        );
        materialize(&mut scene, &selection).expect("absent crossing subject must degrade");
        assert_eq!(
            (
                scene["manga_panel"]["page_design"]["special_device"]["kind"].as_str(),
                scene["manga_panel"]["panels"][0]["continuity"]["breakout"]["enabled"].as_bool(),
            ),
            (Some("none"), Some(false)),
            "failed crossing specialization leaked state or terminated materialization"
        );
    }

    #[test]
    fn invalid_model_authored_devices_fall_back_to_canonical_none() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let devices = [
            json!({
                "kind": "invented_device",
                "reason": "the model invented an unavailable device",
                "source_panel": "",
                "target_panel": "",
                "subject_id": ""
            }),
            json!({
                "kind": "crossing",
                "reason": "the model chose an unsafe reference pair",
                "source_panel": "s2",
                "target_panel": "s1",
                "subject_id": "actor"
            }),
            json!({
                "kind": "crossing",
                "reason": "the model omitted the required subject",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": ""
            }),
            json!({
                "kind": "crossing",
                "reason": "the model named an unavailable shot",
                "source_panel": "s9",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        ];
        let normalized = devices
            .into_iter()
            .map(|device| {
                let mut scene = device_composer_scene(2, device);
                materialize(&mut scene, &selection)
                    .expect("invalid model device must degrade without losing the scene");
                (
                    scene
                        .pointer("/manga_panel/page_design/special_device/kind")
                        .and_then(Value::as_str)
                        .map(String::from),
                    scene
                        .pointer("/manga_panel/page_design/special_device/reason")
                        .and_then(Value::as_str)
                        .is_some_and(|reason| reason.contains("local device fallback")),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            normalized,
            vec![(Some(String::from("none")), true); 4],
            "invalid model devices still terminate scene materialization"
        );
    }

    #[test]
    fn registered_device_absent_from_candidates_cannot_be_forged_by_the_composer() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "overlap",
                "reason": "the composer requested a registered but unqualified device",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": ""
            }),
        );
        materialize(&mut scene, &selection)
            .expect("device absent from candidates must degrade to canonical none");
        assert_eq!(
            (
                scene["manga_panel"]["page_design"]["special_device"]["kind"].as_str(),
                scene["manga_panel"]["page_design"]["special_device"]["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("incompatible with the selected layout")),
            ),
            (Some("none"), true),
            "composer forged a registered device outside its selected candidate catalog"
        );
    }

    #[test]
    fn device_fallback_refuses_a_selection_without_canonical_none() {
        let mut selection = selected_layout("equal-split-vertical-2-v1", "equal");
        selection.summary["device_candidates"]
            .as_array_mut()
            .expect("invariant: device candidates must be an array")
            .retain(|candidate| candidate["scene_kind"] != "none");
        let mut scene = device_composer_scene(
            2,
            json!({
                "kind": "crossing",
                "reason": "one actor crosses the canonical divider",
                "source_panel": "s1",
                "target_panel": "s2",
                "subject_id": "actor"
            }),
        );
        assert!(
            materialize(&mut scene, &selection).is_err(),
            "selection corruption was hidden behind a model-device fallback"
        );
    }

    #[test]
    fn materialization_strengthens_textless_motion_at_the_image_boundary() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let selection = eligible
            .rank()
            .expect("canonical ranking must build locally")
            .select("textless", 0)
            .expect("canonical ranking must select");
        let mut scene = composer_scene(2);
        materialize(&mut scene, &selection).expect("canonical layout must materialize");
        assert_eq!(
            (
                scene
                    .pointer("/manga_panel/art_style/composition/motion_rendering")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/rendering_rules/sound_effects")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/constraints/visible_writing")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/rendering_rules/logos_and_emblems")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/rendering_rules/signs_and_labels")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/rendering_rules/outer_border")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/layout_rendering_directive")
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        value.starts_with(OUTER_WHITE_BAND)
                            && value.contains("exactly 2 visible semantic panel regions")
                            && value.contains("Keep every subject contained")
                    }),
            ),
            (
                Some(TEXTLESS_MOTION),
                Some(TEXTLESS_SOUND_EFFECTS),
                Some(TEXTLESS_VISIBLE_WRITING),
                Some(
                    "No logos, brand marks, emblems, badges, icons, interface symbols, or decorative pseudo-writing on any object."
                ),
                Some(TEXTLESS_LABELS),
                Some(OUTER_WHITE_BAND),
                true,
            ),
            "canonical materialization weakened the final textless image directive"
        );
    }

    #[test]
    fn materialization_rejects_reordered_cinematic_shots() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "equal", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("two-beat feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("equal two-beat layouts must be eligible");
        let selection = eligible
            .rank()
            .expect("canonical ranking must build locally")
            .select("reordered", 0)
            .expect("canonical ranking must select");
        let mut scene = composer_scene(2);
        scene["manga_panel"]["panels"][0]["shot_id"] = Value::String(String::from("s2"));
        assert!(
            materialize(&mut scene, &selection).is_err(),
            "semantic composer reordered a cinematic shot without rejection"
        );
    }

    #[test]
    fn materialization_canonicalizes_reframed_shots_without_changing_semantics() {
        let selection = selected_layout("equal-split-vertical-2-v1", "equal");
        let mut scene = composer_scene(2);
        scene["manga_panel"]["panels"][1]["scene"] = json!({
            "description": "A runner reaches for the same red railing",
            "subjects": [{
                "id": "runner",
                "figure": "the same runner",
                "pose": "one hand closing around the railing",
                "expression": "focused",
                "blocking": "right hand remains clear against the railing"
            }],
            "camera": {
                "shot_scale": "wide",
                "viewpoint": "over_the_shoulder",
                "viewpoint_subject_id": "invented_observer",
                "framing": "group",
                "angle": "dutch",
                "focus": "the runner's hand and red railing",
                "depth_plan": "flat",
                "eye_flow_exit": "toward the grasping hand"
            }
        });
        materialize(&mut scene, &selection).expect("camera drift must canonicalize locally");
        assert_eq!(
            (
                scene.pointer("/manga_panel/panels/1/scene/camera/shot_scale"),
                scene.pointer("/manga_panel/panels/1/scene/camera/viewpoint"),
                scene.pointer("/manga_panel/panels/1/scene/camera/viewpoint_subject_id"),
                scene.pointer("/manga_panel/panels/1/scene/camera/framing"),
                scene.pointer("/manga_panel/panels/1/scene/camera/angle"),
                scene.pointer("/manga_panel/panels/1/scene/camera/depth_plan"),
                scene.pointer("/manga_panel/panels/1/scene/description"),
                scene.pointer("/manga_panel/panels/1/scene/subjects/0/pose"),
                scene.pointer("/manga_panel/panels/1/scene/subjects/0/blocking"),
            ),
            (
                Some(&json!("close")),
                Some(&json!("objective")),
                Some(&json!("")),
                Some(&json!("single")),
                Some(&json!("eye_level")),
                Some(&json!("layered")),
                Some(&json!("A runner reaches for the same red railing")),
                Some(&json!("one hand closing around the railing")),
                Some(&json!("right hand remains clear against the railing")),
            ),
            "local camera repair changed semantic action or blocking"
        );
    }

    #[test]
    fn materialization_canonicalizes_axis_execution_from_the_camera_arc() {
        let mut selection = selected_layout("equal-split-vertical-2-v1", "equal");
        selection.summary["scene_features"]["camera_arc"]["continuity"] = json!({
            "axis_mode": "preserve",
            "axis": "rescuer to swimmer",
            "screen_direction": "left_to_right",
            "eyeline_policy": "not_applicable"
        });
        let mut scene = composer_scene(2);
        for panel in scene["manga_panel"]["panels"]
            .as_array_mut()
            .expect("invariant: composer panels must be an array")
        {
            panel["continuity"]["axis_relation_from_previous"] = json!("not_applicable");
            panel["continuity"]["screen_direction"] = json!("right_to_left");
        }
        materialize(&mut scene, &selection).expect("camera continuity must materialize");
        assert_eq!(
            (
                scene["manga_panel"]["panels"][0]["continuity"]["axis_relation_from_previous"]
                    .as_str(),
                scene["manga_panel"]["panels"][1]["continuity"]["axis_relation_from_previous"]
                    .as_str(),
                scene["manga_panel"]["panels"][0]["continuity"]["screen_direction"].as_str(),
                scene["manga_panel"]["panels"][1]["continuity"]["screen_direction"].as_str(),
            ),
            (
                Some("establish"),
                Some("preserve"),
                Some("left_to_right"),
                Some("left_to_right"),
            ),
            "panel continuity drifted away from the authoritative camera arc"
        );
    }

    #[test]
    fn corrected_diagonal_split_keeps_dominance_at_the_start() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(2, "sequence", "dominant_start", LEFT_TO_RIGHT);
        raw["motion_vector"] = Value::String(String::from("diagonal"));
        raw["intensity"] = Value::String(String::from("high"));
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dominant-start feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("dominant-start layouts must be eligible");
        let selection = eligible
            .rank()
            .expect("corrected diagonal split must rank locally")
            .select("term-0", 0)
            .expect("corrected diagonal split must select");
        let mut scene = composer_scene(2);
        materialize(&mut scene, &selection).expect("corrected diagonal split must materialize");
        assert_eq!(
            (
                scene
                    .pointer("/manga_panel/page_design/dominant_panel")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/frame/shape")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/panels/0/bounds/width")
                    .and_then(Value::as_i64),
                scene
                    .pointer("/manga_panel/panels/1/bounds/width")
                    .and_then(Value::as_i64),
            ),
            (Some("p1"), Some("trapezoid"), Some(600), Some(472)),
            "corrected diagonal split restored end dominance or reversed its canonical geometry"
        );
    }

    #[test]
    fn dominant_end_sequence_offers_straight_and_mirrored_diagonal_geometry() {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let raw = feature_json(2, "sequence", "dominant_end", LEFT_TO_RIGHT);
        let features = registry
            .decode_features(&raw.to_string())
            .expect("dominant-end feature fixture must be valid");
        let eligible = registry
            .eligible(&features)
            .expect("dominant-end layouts must be eligible");
        let straight = eligible
            .templates
            .iter()
            .find(|value| value["template_id"] == "dominant-split-2-end-v1");
        let diagonal = eligible
            .templates
            .iter()
            .find(|value| value["template_id"] == "diagonal-split-2-end-v1");
        assert_eq!(
            (
                straight.and_then(|value| value["dominant_index"].as_u64()),
                diagonal.and_then(|value| value["dominant_index"].as_u64()),
                straight
                    .zip(diagonal)
                    .map(|(left, right)| left["panels"] != right["panels"]),
                diagonal.map(|value| value["panels"].clone()),
            ),
            (
                Some(1),
                Some(1),
                Some(true),
                Some(json!([
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 16, "y": 16, "width": 472, "height": 992},
                        "polygon": [[16, 16], [488, 16], [392, 1008], [16, 1008]]
                    },
                    {
                        "shape": "trapezoid",
                        "bounds": {"x": 408, "y": 16, "width": 600, "height": 992},
                        "polygon": [[504, 16], [1008, 16], [1008, 1008], [408, 1008]]
                    }
                ])),
            ),
            "dominant-end selection lacks two distinct p2-dominant canonical geometries"
        );
    }

    fn feature_json(beat: usize, temporal: &str, emphasis: &str, direction: &str) -> Value {
        json!({
            "semantic_beat_count": beat,
            "semantic_relation": temporal,
            "coverage_audit": coverage_audit(beat),
            "panel_count": beat,
            "panel_relation": temporal,
            "panel_emphasis": emphasis,
            "decomposition_mode": if beat == 1 { "single_tableau" } else { "one_to_one" },
            "motion_vector": "still",
            "intensity": "medium",
            "spatial_relation": "same_space",
            "transition_type": if beat == 1 { "none" } else { "action_to_action" },
            "reading_direction": direction,
            "literal_anchor": "one concrete action",
            "camera_arc": camera_arc(beat),
            "shots": (1..=beat).map(|index| cinematic_shot(
                index,
                index,
                "action",
                format!("visible beat {index}").as_str(),
                format!("source fact {index}").as_str(),
                beat,
            )).collect::<Vec<_>>(),
            "selection_logic": "the literal beats require this feature vector"
        })
    }

    fn expanded_feature_json(panels: usize, relation: &str, emphasis: &str, mode: &str) -> Value {
        json!({
            "semantic_beat_count": 2,
            "semantic_relation": "contrast",
            "coverage_audit": coverage_audit(panels),
            "panel_count": panels,
            "panel_relation": relation,
            "panel_emphasis": emphasis,
            "decomposition_mode": mode,
            "motion_vector": "still",
            "intensity": "medium",
            "spatial_relation": "same_space",
            "transition_type": "subject_to_subject",
            "reading_direction": LEFT_TO_RIGHT,
            "literal_anchor": "a group pressure and one person's reaction",
            "camera_arc": camera_arc(panels),
            "shots": (1..=panels).map(|index| cinematic_shot(
                index,
                if index == 1 { 1 } else { 2 },
                if index == 1 { "establishing" } else { "action" },
                format!("distinct grounded view {index}").as_str(),
                if index == 1 { "the peers" } else { "felt pressure" },
                panels,
            )).collect::<Vec<_>>(),
            "selection_logic": "the final detail carries increasing visual weight"
        })
    }

    fn coverage_audit(panel_count: usize) -> Value {
        Value::Array(
            (1..=4)
                .map(|count| {
                    let verdict = match count.cmp(&panel_count) {
                        std::cmp::Ordering::Less => "insufficient",
                        std::cmp::Ordering::Equal => "selected",
                        std::cmp::Ordering::Greater => "redundant_or_unsupported",
                    };
                    json!({
                        "panel_count": count,
                        "added_view": format!("candidate view {count}"),
                        "source_support": format!("source support audit {count}"),
                        "verdict": verdict,
                        "reason": format!("coverage decision {count}")
                    })
                })
                .collect(),
        )
    }

    fn camera_arc(panels: usize) -> Value {
        json!({
            "strategy": if panels == 1 { "single_view" } else { "push_in" },
            "progression": if panels == 1 { "one held medium view" } else { "wide context progresses toward close evidence" },
            "motivation": "each closer setup exposes the next supported fact",
            "continuity": {
                "axis_mode": "not_applicable",
                "axis": "",
                "screen_direction": "not_applicable",
                "eyeline_policy": "not_applicable"
            }
        })
    }

    fn cinematic_shot(
        index: usize,
        beat: usize,
        role: &str,
        anchor: &str,
        support: &str,
        panels: usize,
    ) -> Value {
        json!({
            "id": format!("s{index}"),
            "semantic_beat_index": beat,
            "role": role,
            "visible_anchor": anchor,
            "source_support": support,
            "shot_scale": cinematic_scale(index, panels),
            "viewpoint": "objective",
            "viewpoint_anchor": "",
            "framing": "single",
            "angle": "eye_level",
            "depth_plan": "layered",
            "camera_motivation": format!("shot {index} reveals its supported narrative function"),
            "information_gain": format!("new visible evidence {index}"),
            "transition_trigger": if index == 1 { "scene_open" } else { "new_action" }
        })
    }

    fn cinematic_scale(index: usize, panels: usize) -> &'static str {
        match (panels, index) {
            (1, 1) => "medium",
            (2..=4, 1) => "wide",
            (2, 2) | (3, 3) => "close",
            (3..=4, 2) => "medium",
            (4, 3) => "close",
            (4, 4) => "extreme_close",
            _ => panic!("invariant: cinematic fixture index must fit one to four panels"),
        }
    }

    fn dynamic_hierarchy(value: &mut Value) {
        let shots = value["shots"]
            .as_array_mut()
            .expect("invariant: dynamic hierarchy fixture must contain shots");
        match shots.len() {
            2 => {
                shots[0]["role"] = json!("action");
                shots[1]["role"] = json!("payoff");
            }
            3 => {
                shots[0]["role"] = json!("establishing");
                shots[1]["role"] = json!("action");
                shots[2]["role"] = json!("payoff");
            }
            _ => panic!("invariant: dynamic hierarchy fixture must have two or three shots"),
        }
    }

    /// Select one exact automatic layout fixture for device materialization tests.
    fn selected_layout(template_id: &str, emphasis: &str) -> LayoutSelection {
        let registry = LayoutRegistry::embedded().expect("embedded registry must be valid");
        let mut raw = feature_json(
            if template_id == "splash-1-v1" { 1 } else { 2 },
            if template_id == "splash-1-v1" {
                "single_moment"
            } else {
                "sequence"
            },
            emphasis,
            LEFT_TO_RIGHT,
        );
        if template_id.starts_with("diagonal-") {
            raw["motion_vector"] = Value::String(String::from("diagonal"));
            raw["intensity"] = Value::String(String::from("high"));
        }
        let features = registry
            .decode_features(&raw.to_string())
            .expect("device feature fixture must be valid");
        let ranking = registry
            .eligible(&features)
            .expect("device layout must be eligible")
            .rank()
            .expect("device layout must rank locally");
        (0..256)
            .find_map(|index| {
                let term = format!("device-test-{index}");
                let selection = ranking.select(&term, 0).ok()?;
                (selection.summary["chosen_template_id"].as_str() == Some(template_id))
                    .then_some(selection)
            })
            .expect("invariant: requested device layout must remain locally selectable")
    }

    /// Add one explicitly qualified device to a selection for isolated materializer tests.
    fn selected_layout_with_device(
        template_id: &str,
        emphasis: &str,
        kind: &str,
    ) -> LayoutSelection {
        let mut selection = selected_layout(template_id, emphasis);
        let registry = serde_json::from_str::<Value>(device_registry())
            .expect("invariant: device registry fixture must decode");
        let mut device = registry["devices"]
            .as_array()
            .expect("invariant: device registry must contain devices")
            .iter()
            .find(|device| device["scene_kind"] == kind)
            .cloned()
            .expect("invariant: requested materializer device must exist");
        device["automatic_selection"] = Value::Bool(true);
        device["capability_status"] = Value::String(String::from("qualified"));
        let candidate = device_candidate(&device, &selection.template)
            .expect("qualified materializer fixture must project")
            .expect("qualified materializer fixture must have safe references");
        selection.summary["device_candidates"]
            .as_array_mut()
            .expect("invariant: selected device candidates must be an array")
            .push(candidate);
        selection
    }

    /// Create one semantic composer fixture carrying a model-selected device.
    fn device_composer_scene(panels: usize, device: Value) -> Value {
        let mut scene = composer_scene(panels);
        scene["manga_panel"]["page_design"]["special_device"] = device;
        for (index, panel) in scene["manga_panel"]["panels"]
            .as_array_mut()
            .expect("invariant: composer panels must be an array")
            .iter_mut()
            .enumerate()
        {
            panel["scene"]["subjects"] = json!([{
                "id": "actor",
                "figure": "the same actor",
                "pose": format!("phase {}", index + 1),
                "expression": "focused",
                "blocking": "clear of protected focal zones"
            }]);
            panel["continuity"]["shared_environment_id"] =
                Value::String(String::from("shared-room"));
            panel["continuity"]["subject_phase"] = Value::String(format!("phase-{}", index + 1));
        }
        scene
    }

    fn composer_scene(panels: usize) -> Value {
        json!({
            "manga_panel": {
                "canvas": {"reading_direction": RIGHT_TO_LEFT},
                "page_design": {"special_device": {
                    "kind": "none",
                    "reason": "the canonical topology already carries the scene",
                    "source_panel": "",
                    "target_panel": "",
                    "subject_id": ""
                }},
                "constraints": {"maximum_panels": 4, "panel_count_lock": false},
                "panels": (1..=panels).map(|index| json!({
                    "shot_id": format!("s{index}"),
                    "id": format!("agent-{index}"),
                    "bleed": true,
                    "bounds": {"x": 1, "y": 1, "width": 17, "height": 17},
                    "frame": {"shape": "agent_shape", "polygon": []},
                    "continuity": {"shared_environment_id": "room", "subject_phase": format!("phase-{index}"), "breakout": {"enabled": true}},
                    "scene": {
                        "description": format!("visible beat {index}"),
                        "camera": {
                            "shot_scale": cinematic_scale(index, panels),
                            "viewpoint": "objective",
                            "viewpoint_subject_id": "",
                            "framing": "single",
                            "angle": "eye_level",
                            "focus": format!("visible beat {index}"),
                            "depth_plan": "layered",
                            "eye_flow_exit": "toward the next panel"
                        }
                    }
                })).collect::<Vec<_>>()
            }
        })
    }
}
