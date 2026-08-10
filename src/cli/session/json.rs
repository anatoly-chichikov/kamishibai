//! JSON projections of the session commands: pure `Serialize` DTOs assembled
//! from the same `view` computations the plain-text renderers use, plus the
//! one [`emit`] point. No logic of its own lives here — phases, readiness, and
//! presence rules come from `view` and the record.
//!
//! Schema promises: one compact document per invocation, `ok` discriminates
//! success from the error envelope (`error::json_line`), absent options are
//! omitted (never `null`), `senses[].number` is 1-based to match
//! `select --sense`. Fields evolve additively; closed token vocabularies change
//! only with an explicit release-contract update and backward read aliases.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{
    AxisSet, CardDraft, CardMetaCache, LanguagePair, SentenceAxis, SentenceBatchSettings,
    SentenceLabelSelection, SentenceLabels,
};
use crate::vocabulary::VocabularyEntry;

use super::store::{Phase, ResultRecord, SessionRecord};
use super::view;

/// Print one document as the single JSON line `--json` mode puts on stdout.
pub(super) fn emit(document: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(document)?);
    Ok(())
}

/// Project one record against the live cache and print its session document —
/// the one emission seam every stateful verb's JSON branch goes through.
pub(super) fn emit_session(record: &SessionRecord) -> Result<()> {
    println!("{}", session_line(record)?);
    Ok(())
}

/// Serialize one record's session document to a single line (no trailing
/// newline) — the same string `emit_session` prints, for callers that must
/// write it to a descriptor other than the global stdout.
pub(super) fn session_line(record: &SessionRecord) -> Result<String> {
    let root = cache_root(&SystemContext)?;
    Ok(serde_json::to_string(&SessionDoc::of(
        record,
        root.as_path(),
    ))?)
}

/// The one session document every stateful verb returns: identity, the live
/// phase, the curatable candidates, the committed cards with cache-derived
/// readiness, and the published result when one exists.
#[derive(Serialize)]
pub(super) struct SessionDoc {
    ok: bool,
    session: String,
    created: String,
    pair: PairDoc,
    senses: String,
    sentences: SentenceBatchSettings,
    source: String,
    out: String,
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    worker: Option<WorkerDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<ProgressDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidates: Option<CandidatesDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cards: Option<CardsDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ResultPathsDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PairDoc {
    known: String,
    learning: String,
}

impl PairDoc {
    /// Build the canonical uppercase pair document from a record.
    fn of(record: &SessionRecord) -> Self {
        Self {
            known: record.known.to_uppercase(),
            learning: record.learning.to_uppercase(),
        }
    }
}

#[derive(Serialize)]
struct WorkerDoc {
    pid: i32,
    alive: bool,
}

#[derive(Serialize)]
struct ProgressDoc {
    term: String,
    artifact: String,
}

#[derive(Serialize)]
struct CandidatesDoc {
    items: Vec<CandidateDoc>,
}

#[derive(Serialize)]
struct CandidateDoc {
    term: String,
    included: bool,
    senses: Vec<SenseDoc>,
}

#[derive(Serialize)]
struct SenseDoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    understanding: String,
    selected: bool,
}

#[derive(Serialize)]
struct CardsDoc {
    pending: usize,
    items: Vec<CardDoc>,
}

#[derive(Serialize)]
struct CardDoc {
    term: String,
    understanding: String,
    artifacts: ArtifactsDoc,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<LabelsDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adjustment: Option<AdjustmentDoc>,
}

#[derive(Serialize)]
struct ArtifactsDoc {
    meta: bool,
    sound: bool,
    scene: bool,
    picture: bool,
}

#[derive(Serialize)]
struct LabelsDoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    register: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    pinned: Vec<&'static str>,
    approx: Vec<&'static str>,
}

impl LabelsDoc {
    fn current(labels: &SentenceLabels) -> Self {
        Self {
            register: labels.token(SentenceAxis::Register),
            level: labels.token(SentenceAxis::Level),
            kind: labels.token(SentenceAxis::Type),
            pinned: axes(labels.pinned()),
            approx: axes(labels.approx()),
        }
    }

    fn requested(labels: &SentenceLabelSelection) -> Self {
        Self {
            register: labels.token(SentenceAxis::Register),
            level: labels.token(SentenceAxis::Level),
            kind: labels.token(SentenceAxis::Type),
            pinned: axes(labels.pinned()),
            approx: axes(labels.approx()),
        }
    }
}

#[derive(Serialize)]
struct AdjustmentDoc {
    state: &'static str,
    labels: LabelsDoc,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn axes(axes: &AxisSet) -> Vec<&'static str> {
    axes.iter().map(axis).collect()
}

fn axis(axis: SentenceAxis) -> &'static str {
    match axis {
        SentenceAxis::Register => "register",
        SentenceAxis::Level => "level",
        SentenceAxis::Type => "kind",
    }
}

#[derive(Serialize)]
struct ResultPathsDoc {
    deck: String,
    pdf: String,
    dir: String,
}

impl SessionDoc {
    /// Project one record against the cache: the same live-phase reconciliation
    /// and per-card artifact probing `status` renders as text. Only data — no
    /// derived counts, no card state words; the agent computes those from the
    /// `artifacts` booleans plus `phase`.
    pub(super) fn of(record: &SessionRecord, cache_root: &Path) -> Self {
        let (phase, pid, live) = view::live_phase(record, cache_root);
        Self {
            ok: true,
            session: record.id.clone(),
            created: record.created.clone(),
            pair: PairDoc::of(record),
            senses: record.senses.clone(),
            sentences: record.sentences,
            source: record.source.clone(),
            out: record.out.clone(),
            phase: view::phase_label(phase),
            worker: pid.map(|pid| WorkerDoc { pid, alive: live }),
            progress: matches!(phase, Phase::Generating)
                .then(|| record.progress.as_ref())
                .flatten()
                .map(|progress| ProgressDoc {
                    term: progress.term.clone(),
                    artifact: progress.artifact.clone(),
                }),
            candidates: candidates_doc(record),
            cards: cards_doc(record, cache_root),
            result: record.result.as_ref().map(|result| ResultPathsDoc {
                deck: result.deck.clone(),
                pdf: result.report.clone(),
                dir: result.output.clone(),
            }),
            error: record.error.clone(),
        }
    }
}

fn candidates_doc(record: &SessionRecord) -> Option<CandidatesDoc> {
    if record.candidates.is_empty() {
        return None;
    }
    let items: Vec<CandidateDoc> = record
        .candidates
        .iter()
        .map(|stored| {
            let candidate = stored.clone().candidate();
            let senses = candidate
                .senses()
                .iter()
                .enumerate()
                .map(|(index, sense)| SenseDoc {
                    tag: sense.tag().map(String::from),
                    understanding: String::from(sense.understanding()),
                    selected: candidate.ok() && candidate.selected_senses().contains(&index),
                })
                .collect();
            CandidateDoc {
                term: String::from(candidate.term()),
                included: candidate.ok(),
                senses,
            }
        })
        .collect();
    Some(CandidatesDoc { items })
}

fn cards_doc(record: &SessionRecord, cache_root: &Path) -> Option<CardsDoc> {
    if record.drafts.is_empty() {
        return None;
    }
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let cache = CardMetaCache::new(cache_root);
    let pending = record
        .drafts
        .iter()
        .filter(|draft| {
            draft
                .rewrite
                .as_ref()
                .is_some_and(|rewrite| !rewrite.started())
        })
        .count();
    let items = record
        .drafts
        .iter()
        .zip(view::cards(record, cache_root))
        .map(|(draft, card)| CardDoc {
            term: card.term,
            understanding: card.understanding,
            artifacts: ArtifactsDoc {
                meta: card.meta,
                sound: card.sound,
                scene: card.scene,
                picture: card.picture,
            },
            labels: current_labels(draft, &cache, &pair),
            adjustment: draft.rewrite.as_ref().map(|rewrite| AdjustmentDoc {
                state: if rewrite.started() {
                    "active"
                } else {
                    "pending"
                },
                labels: LabelsDoc::requested(rewrite.selection()),
                note: (!rewrite.note().trim().is_empty()).then(|| rewrite.note().to_string()),
            }),
        })
        .collect();
    Some(CardsDoc { pending, items })
}

fn current_labels(
    draft: &super::store::DraftRecord,
    cache: &CardMetaCache,
    pair: &LanguagePair,
) -> Option<LabelsDoc> {
    if view::awaits_initial_meta(draft) {
        return None;
    }
    if let Some(previous) = draft
        .rewrite
        .as_ref()
        .and_then(crate::session::CardRewrite::previous)
    {
        return previous.sentence_labels().map(LabelsDoc::current);
    }
    cache
        .load(draft.term.as_str(), draft.understanding.as_str(), pair)
        .ok()
        .flatten()
        .and_then(|meta| meta.sentence_labels().map(LabelsDoc::current))
}

/// The `result --json` document: published paths plus every settled card with
/// current cached meta as a strict `VocabularyEntry`, exactly like the text
/// render. The entry schema is the one `new --build` imports, so items round-trip
/// back into a new session.
#[derive(Serialize)]
pub(super) struct ResultDoc {
    ok: bool,
    session: String,
    pair: PairDoc,
    sentences: SentenceBatchSettings,
    phase: &'static str,
    paths: PathsDoc,
    failed: usize,
    items: Vec<VocabularyEntry>,
}

#[derive(Serialize)]
struct PathsDoc {
    deck: String,
    pdf: String,
    dir: String,
}

impl ResultDoc {
    /// Assemble the published document, loading each settled draft's meta from
    /// the shared cache and bridging it through the same `to_entry` path the
    /// deck itself is built from. Drafts without cached meta and drafts whose
    /// initial metadata request is still pending are skipped because their
    /// preserved cache is rollback material, exactly like the text render.
    pub(super) fn of(
        record: &SessionRecord,
        cache_root: &Path,
        phase: Phase,
        paths: &ResultRecord,
    ) -> Result<Self> {
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        let cache = CardMetaCache::new(cache_root.to_path_buf());
        let mut items = Vec::with_capacity(record.drafts.len());
        for draft in &record.drafts {
            if view::awaits_initial_meta(draft) {
                continue;
            }
            if let Some(meta) =
                cache.load(draft.term.as_str(), draft.understanding.as_str(), &pair)?
            {
                let card = CardDraft::new(
                    draft.term.as_str(),
                    draft.understanding.as_str(),
                    pair.clone(),
                )
                .with_meta(meta, None);
                items.push(crate::session::to_entry(&card)?);
            }
        }
        Ok(Self {
            ok: true,
            session: record.id.clone(),
            pair: PairDoc::of(record),
            sentences: record.sentences,
            phase: view::phase_label(phase),
            paths: PathsDoc {
                deck: paths.deck.clone(),
                pdf: paths.report.clone(),
                dir: paths.output.clone(),
            },
            failed: paths.failed,
            items,
        })
    }
}

/// The `ls --json` document: every session as one summary item.
#[derive(Serialize)]
pub(super) struct LsDoc {
    ok: bool,
    sessions: Vec<LsItem>,
}

#[derive(Serialize)]
pub(super) struct LsItem {
    id: String,
    pair: PairDoc,
    created: String,
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cards: Option<LsCards>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected: Option<usize>,
}

#[derive(Serialize)]
struct LsCards {
    total: usize,
    ready: usize,
}

impl LsDoc {
    /// Wrap every record as one `ls --json` document.
    pub(super) fn of(records: &[SessionRecord], cache_root: &Path) -> Self {
        Self {
            ok: true,
            sessions: ls_items(records, cache_root),
        }
    }
}

/// Project records into `ls --json` items, the way `summary_line` does:
/// committed plans carry `cards{total,ready}`, curatable sessions carry the
/// `selected` count. The ambiguous-resolution envelope reuses these so its
/// `sessions` array matches `ls` exactly.
pub(super) fn ls_items(records: &[SessionRecord], cache_root: &Path) -> Vec<LsItem> {
    records
        .iter()
        .map(|record| {
            let (phase, _, _) = view::live_phase(record, cache_root);
            let cards = view::cards(record, cache_root);
            let committed = !record.drafts.is_empty();
            LsItem {
                id: record.id.clone(),
                pair: PairDoc::of(record),
                created: record.created.clone(),
                phase: view::phase_label(phase),
                cards: committed.then(|| LsCards {
                    total: cards.len(),
                    ready: cards.iter().filter(|card| card.ready()).count(),
                }),
                selected: (!committed).then(|| view::selected_cards(record)),
            }
        })
        .collect()
}

/// The `rm --json` acknowledgement.
#[derive(Serialize)]
pub(super) struct RemovedDoc {
    ok: bool,
    removed: String,
}

impl RemovedDoc {
    /// Acknowledge the removal of session `id`.
    pub(super) fn of(id: &str) -> Self {
        Self {
            ok: true,
            removed: String::from(id),
        }
    }
}

/// The `cache-path --json` document.
#[derive(Serialize)]
pub(super) struct CacheDoc {
    ok: bool,
    cache: String,
}

impl CacheDoc {
    /// Wrap the cache root path as the `cache-path` document.
    pub(super) fn of(path: &Path) -> Self {
        Self {
            ok: true,
            cache: path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::super::store::{DraftRecord, WorkerHandle};
    use super::*;
    use crate::generation::artifact_cache::{ILLUSTRATION_FILE, SCENE_FILE, VOICE_FILE};
    use crate::generation::visual_revision;
    use crate::session::{
        CandidateRecord, CardCell, CardMeta, CardMetaCache, CardRewrite, Register, Sense,
        SentenceKind, SentenceLevel, SentenceTypeMix, WordCandidate,
    };

    fn unlabeled_meta() -> CardMeta {
        CardMeta::new(
            "/ka.naʁ/",
            "/lə ka.naʁ naʒ/",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A common concrete noun",
            "Le canard nage",
        )
    }

    fn fixture_meta() -> CardMeta {
        unlabeled_meta().with_sentence_labels(SentenceLabels::new(
            Register::Neutral,
            SentenceLevel::B1,
            SentenceKind::Statement,
            AxisSet::default(),
            AxisSet::default(),
        ))
    }

    fn labeled_meta(register: Register, level: SentenceLevel, kind: SentenceKind) -> CardMeta {
        CardMeta::new(
            "/ka.naʁ/",
            "/lə ka.naʁ naʒ/",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A common concrete noun",
            "Le canard nage",
        )
        .with_sentence_labels(SentenceLabels::new(
            register,
            level,
            kind,
            AxisSet::default(),
            AxisSet::default(),
        ))
    }

    fn store_artifacts(home: &TempDir, term: &str, understanding: &str) {
        let pair = LanguagePair::new("fr", "en");
        CardMetaCache::new(home.path())
            .store(term, understanding, &pair, &fixture_meta())
            .expect("valid meta fixture must be stored");
        let cache = CardCell::new(home.path(), &pair, term, understanding).cache();
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::write(
            cache.filepath(VOICE_FILE).expect("voice path must resolve"),
            b"x",
        )
        .expect("voice written");
        fs::write(
            visual
                .filepath(SCENE_FILE)
                .expect("scene path must resolve"),
            include_bytes!("../../../tests/fixtures/production-scene.json"),
        )
        .expect("scene written");
        fs::write(
            visual
                .filepath(ILLUSTRATION_FILE)
                .expect("picture path must resolve"),
            b"x",
        )
        .expect("picture written");
    }

    fn record() -> SessionRecord {
        SessionRecord::understood(
            String::from("fr-1"),
            String::from("2026-06-06T00:00:00Z"),
            String::from("en"),
            String::from("fr"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard")],
            vec![CandidateRecord::from_candidate(
                &WordCandidate::with_selected_senses(
                    "canard",
                    vec![Sense::plain("a duck"), Sense::plain("a hoax")],
                    vec![1],
                    true,
                ),
            )],
        )
    }

    fn value_of(record: &SessionRecord, root: &Path) -> serde_json::Value {
        serde_json::to_value(SessionDoc::of(record, root)).expect("the document must serialize")
    }

    fn nulls_in(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Null => 1,
            serde_json::Value::Array(items) => items.iter().map(nulls_in).sum(),
            serde_json::Value::Object(map) => map.values().map(nulls_in).sum(),
            _ => 0,
        }
    }

    #[test]
    fn an_understood_document_carries_candidates_and_no_cards() {
        let home = TempDir::new().expect("tempdir must be created");
        let value = value_of(&record(), home.path());
        assert_eq!(
            (
                value["phase"].as_str(),
                value["candidates"]["items"][0]["senses"][1]["understanding"].as_str(),
                value["candidates"]["items"][0]["senses"][1]["selected"].as_bool(),
                value.get("cards"),
                value["candidates"]["items"][0]["senses"][1].get("number"),
                value["sentences"].clone(),
            ),
            (
                Some("understood"),
                Some("a hoax"),
                Some(true),
                None,
                None,
                serde_json::json!({"types": "best-fit"}),
            ),
            "an understood document must carry candidate senses and best-fit example settings while omitting the cards block"
        );
    }

    #[test]
    fn nondefault_sentence_settings_are_exposed_by_session_and_result_documents() {
        let home = TempDir::new().expect("tempdir must be created");
        let record = record().with_sentences(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Mixed,
        ));
        let session = value_of(&record, home.path());
        let paths = ResultRecord {
            deck: String::from("/out/cards.apkg"),
            report: String::from("/out/cards.pdf"),
            output: String::from("/out"),
            cards: 0,
            failed: 0,
        };
        let result = serde_json::to_value(
            ResultDoc::of(&record, home.path(), Phase::Published, &paths)
                .expect("result document must build"),
        )
        .expect("result document must serialize");
        let expected = serde_json::json!({"level": "b1", "types": "mixed"});
        assert_eq!(
            (session["sentences"].clone(), result["sentences"].clone()),
            (expected.clone(), expected),
            "session or result JSON omitted the configured generation guidance"
        );
    }

    #[test]
    fn a_committed_document_reads_artifact_presence_from_the_cache() {
        let home = TempDir::new().expect("tempdir must be created");
        let mut record = record();
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            costs: crate::session::ArtifactCosts::default(),
            rewrite: None,
            meta_request: None,
        }];
        let cell = CardCell::new(
            home.path(),
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        );
        let cache = cell.cache();
        CardMetaCache::new(home.path())
            .store(
                "canard",
                "a duck",
                &LanguagePair::new("fr", "en"),
                &fixture_meta(),
            )
            .expect("valid meta fixture must be stored");
        fs::write(cache.path().join(VOICE_FILE), b"x").expect("voice written");
        let value = value_of(&record, home.path());
        assert_eq!(
            (
                value["cards"]["items"][0]["artifacts"]["meta"].as_bool(),
                value["cards"]["items"][0]["artifacts"]["scene"].as_bool(),
                value["cards"]["items"][0].get("state"),
                value["cards"]["pending"].as_u64(),
                value["cards"]["items"][0]["labels"].clone(),
            ),
            (
                Some(true),
                Some(false),
                None,
                Some(0),
                serde_json::json!({
                    "register": "neutral",
                    "level": "b1",
                    "kind": "statement",
                    "pinned": [],
                    "approx": []
                }),
            ),
            "a committed document must carry cache-derived artifacts and current labels without a pending adjustment"
        );
    }

    #[test]
    fn a_partial_result_hides_rollback_cache_while_initial_metadata_is_pending() {
        let home = TempDir::new().expect("tempdir must be created");
        store_artifacts(&home, "canard", "a duck");
        store_artifacts(&home, "hibou", "an owl");
        let mut record = record();
        record.drafts = vec![
            DraftRecord {
                term: String::from("canard"),
                understanding: String::from("a duck"),
                costs: crate::session::ArtifactCosts::default(),
                rewrite: None,
                meta_request: Some(
                    SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 4),
                ),
            },
            DraftRecord {
                term: String::from("hibou"),
                understanding: String::from("an owl"),
                costs: crate::session::ArtifactCosts::default(),
                rewrite: None,
                meta_request: None,
            },
        ];
        let session = value_of(&record, home.path());
        let paths = ResultRecord {
            deck: String::from("/out/cards.apkg"),
            report: String::from("/out/cards.pdf"),
            output: String::from("/out"),
            cards: 1,
            failed: 1,
        };
        let result = serde_json::to_value(
            ResultDoc::of(&record, home.path(), Phase::Partial, &paths)
                .expect("partial result document must build"),
        )
        .expect("partial result document must serialize");
        let stale = &session["cards"]["items"][0];
        assert_eq!(
            (
                stale["artifacts"]["meta"].as_bool(),
                stale["artifacts"]["sound"].as_bool(),
                stale["artifacts"]["scene"].as_bool(),
                stale["artifacts"]["picture"].as_bool(),
                stale.get("labels"),
                result["items"].as_array().map(Vec::len),
                result["items"][0]["term"].as_str(),
            ),
            (
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                None,
                Some(1),
                Some("hibou"),
            ),
            "pending initial metadata exposed rollback cache as current status or published result"
        );
    }

    #[test]
    fn a_staged_adjustment_separates_current_labels_from_the_requested_selection() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        let previous = fixture_meta();
        let baseline = SentenceLabelSelection::from_labels(
            previous
                .sentence_labels()
                .expect("the current metadata must carry labels"),
        );
        let requested = baseline.choosing(SentenceAxis::Level, 3);
        let staged = CardDraft::new("canard", "a duck", pair.clone())
            .with_meta(previous, None)
            .staging_rewrite(requested, "keep it natural");
        let mut record = record();
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            costs: crate::session::ArtifactCosts::default(),
            rewrite: staged.rewrite().cloned(),
            meta_request: None,
        }];
        CardMetaCache::new(home.path())
            .store(
                "canard",
                "a duck",
                &pair,
                &labeled_meta(Register::Casual, SentenceLevel::A2, SentenceKind::Question),
            )
            .expect("different cached labels must store");
        let value = value_of(&record, home.path());
        assert_eq!(
            (
                value["cards"]["pending"].as_u64(),
                value["cards"]["items"][0]["labels"].clone(),
                value["cards"]["items"][0]["adjustment"].clone(),
            ),
            (
                Some(1),
                serde_json::json!({
                    "register": "neutral",
                    "level": "b1",
                    "kind": "statement",
                    "pinned": [],
                    "approx": []
                }),
                serde_json::json!({
                    "state": "pending",
                    "labels": {
                        "register": "neutral",
                        "level": "b2",
                        "kind": "statement",
                        "pinned": ["level"],
                        "approx": []
                    },
                    "note": "keep it natural"
                }),
            ),
            "a staged adjustment merged requested labels into the current attribution or lost its pending state"
        );
    }

    #[test]
    fn an_active_adjustment_keeps_previous_labels_without_cached_metadata_or_nulls() {
        let home = TempDir::new().expect("tempdir must be created");
        let previous = fixture_meta();
        let requested = SentenceLabelSelection::empty().choosing(SentenceAxis::Type, 1);
        let mut record = record();
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            costs: crate::session::ArtifactCosts::default(),
            rewrite: Some(CardRewrite::new(Some(previous), requested, " \n ")),
            meta_request: None,
        }];
        let value = value_of(&record, home.path());
        assert_eq!(
            (
                value["cards"]["pending"].as_u64(),
                value["cards"]["items"][0]["labels"].clone(),
                value["cards"]["items"][0]["adjustment"].clone(),
                nulls_in(&value),
            ),
            (
                Some(0),
                serde_json::json!({
                    "register": "neutral",
                    "level": "b1",
                    "kind": "statement",
                    "pinned": [],
                    "approx": []
                }),
                serde_json::json!({
                    "state": "active",
                    "labels": {
                        "kind": "question",
                        "pinned": ["kind"],
                        "approx": []
                    }
                }),
                0,
            ),
            "an active adjustment lost its previous labels, exposed a blank note, or serialized absent fields as null"
        );
    }

    #[test]
    fn a_legacy_previous_snapshot_cannot_borrow_labels_from_shared_cache() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        CardMetaCache::new(home.path())
            .store(
                "canard",
                "a duck",
                &pair,
                &labeled_meta(Register::Formal, SentenceLevel::B2, SentenceKind::Question),
            )
            .expect("shared labeled metadata must store");
        let mut record = record();
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            costs: crate::session::ArtifactCosts::default(),
            rewrite: Some(CardRewrite::new(
                Some(unlabeled_meta()),
                SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 2),
                "",
            )),
            meta_request: None,
        }];
        let value = value_of(&record, home.path());
        assert_eq!(
            value["cards"]["items"][0].get("labels"),
            None,
            "a legacy rewrite snapshot leaked labels written later into shared cache"
        );
    }

    #[test]
    fn a_recorded_worker_without_a_held_lock_reads_interrupted_in_json_too() {
        let home = TempDir::new().expect("tempdir must be created");
        let mut record = record();
        record.worker = Some(WorkerHandle {
            pid: 999_999,
            started: String::from("t"),
        });
        let value = value_of(&record, home.path());
        assert_eq!(
            (value["phase"].as_str(), value["worker"]["alive"].as_bool()),
            (Some("interrupted"), Some(false)),
            "the JSON phase must be the same live reconciliation status prints, not the raw stored phase"
        );
    }

    #[test]
    fn no_document_field_ever_serializes_as_null() {
        let home = TempDir::new().expect("tempdir must be created");
        let value = value_of(&record(), home.path());
        assert_eq!(
            nulls_in(&value),
            0,
            "absent options must be omitted from the document, never serialized as null"
        );
    }
}
