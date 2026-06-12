//! JSON projections of the session commands: pure `Serialize` DTOs assembled
//! from the same `view` computations the plain-text renderers use, plus the
//! one [`emit`] point. No logic of its own lives here — phases, readiness, and
//! presence rules come from `view` and the record.
//!
//! Schema promises: one compact document per invocation, `ok` discriminates
//! success from the error envelope (`error::json_line`), absent options are
//! omitted (never `null`), `senses[].number` is 1-based to match
//! `select --sense`, and evolution is additive only.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardDraft, CardMetaCache, LanguagePair};
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
    let root = cache_root(&SystemContext)?;
    emit(&SessionDoc::of(record, root.as_path()))
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
    source: String,
    out: String,
    phase: &'static str,
    words: Vec<String>,
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
    from: String,
    to: String,
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
    count: usize,
    selected: usize,
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
    number: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    understanding: String,
    selected: bool,
}

#[derive(Serialize)]
struct CardsDoc {
    total: usize,
    ready: usize,
    failed: usize,
    items: Vec<CardDoc>,
}

#[derive(Serialize)]
struct CardDoc {
    term: String,
    understanding: String,
    state: &'static str,
    artifacts: ArtifactsDoc,
}

#[derive(Serialize)]
struct ArtifactsDoc {
    meta: bool,
    sound: bool,
    scene: bool,
    picture: bool,
}

#[derive(Serialize)]
struct ResultPathsDoc {
    deck: String,
    pdf: String,
    dir: String,
    cards: usize,
    failed: usize,
}

impl SessionDoc {
    /// Project one record against the cache: the same live-phase reconciliation
    /// and per-card artifact probing `status` renders as text.
    pub(super) fn of(record: &SessionRecord, cache_root: &Path) -> Self {
        let (phase, pid, live) = view::live_phase(record, cache_root);
        Self {
            ok: true,
            session: record.id.clone(),
            created: record.created.clone(),
            pair: PairDoc {
                from: record.from.clone(),
                to: record.to.clone(),
            },
            senses: record.senses.clone(),
            source: record.source.clone(),
            out: record.out.clone(),
            phase: view::phase_label(phase),
            words: record.words.clone(),
            worker: pid.map(|pid| WorkerDoc { pid, alive: live }),
            progress: record.progress.as_ref().map(|progress| ProgressDoc {
                term: progress.term.clone(),
                artifact: progress.artifact.clone(),
            }),
            candidates: candidates_doc(record),
            cards: cards_doc(record, cache_root, phase),
            result: record.result.as_ref().map(|result| ResultPathsDoc {
                deck: result.deck.clone(),
                pdf: result.report.clone(),
                dir: result.output.clone(),
                cards: result.cards,
                failed: result.failed,
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
                    number: index + 1,
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
    Some(CandidatesDoc {
        count: items.len(),
        selected: view::selected_cards(record),
        items,
    })
}

fn cards_doc(record: &SessionRecord, cache_root: &Path, phase: Phase) -> Option<CardsDoc> {
    if record.drafts.is_empty() {
        return None;
    }
    let cards = view::cards(record, cache_root);
    let ready = cards.iter().filter(|card| card.ready()).count();
    let failed = if view::terminal(phase) {
        cards.len() - ready
    } else {
        0
    };
    let items = cards
        .iter()
        .map(|card| CardDoc {
            term: card.term.clone(),
            understanding: card.understanding.clone(),
            state: view::row_label(card, phase),
            artifacts: ArtifactsDoc {
                meta: card.meta,
                sound: card.sound,
                scene: card.scene,
                picture: card.picture,
            },
        })
        .collect();
    Some(CardsDoc {
        total: cards.len(),
        ready,
        failed,
        items,
    })
}

/// The `result --json` document: the published paths plus every card with
/// cached meta as a strict `VocabularyEntry`, exactly like the text render —
/// the exact schema `new --build` imports, so items round-trip back into a
/// new session.
#[derive(Serialize)]
pub(super) struct ResultDoc {
    ok: bool,
    session: String,
    pair: PairDoc,
    phase: &'static str,
    paths: PathsDoc,
    cards: usize,
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
    /// Assemble the published document, loading each draft's meta from the
    /// shared cache and bridging it through the same `to_entry` path the deck
    /// itself is built from. Drafts without cached meta are skipped, exactly
    /// like the text render.
    pub(super) fn of(
        record: &SessionRecord,
        cache_root: &Path,
        phase: Phase,
        paths: &ResultRecord,
    ) -> Result<Self> {
        let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
        let cache = CardMetaCache::new(cache_root.to_path_buf());
        let mut items = Vec::with_capacity(record.drafts.len());
        for draft in &record.drafts {
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
            pair: PairDoc {
                from: record.from.clone(),
                to: record.to.clone(),
            },
            phase: view::phase_label(phase),
            paths: PathsDoc {
                deck: paths.deck.clone(),
                pdf: paths.report.clone(),
                dir: paths.output.clone(),
            },
            cards: paths.cards,
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
                pair: PairDoc {
                    from: record.from.clone(),
                    to: record.to.clone(),
                },
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
    use crate::generation::artifact_cache::{META_FILE, VOICE_FILE};
    use crate::session::{CandidateRecord, CardCell, Sense, WordCandidate};

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
                value["candidates"]["items"][0]["senses"][1]["number"].as_u64(),
                value["candidates"]["items"][0]["senses"][1]["selected"].as_bool(),
                value.get("cards"),
            ),
            (Some("understood"), Some(2), Some(true), None),
            "an understood document must list 1-based candidate senses and omit the cards block"
        );
    }

    #[test]
    fn a_committed_document_reads_artifact_presence_from_the_cache() {
        let home = TempDir::new().expect("tempdir must be created");
        let mut record = record();
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
        }];
        let cell = CardCell::new(
            home.path(),
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        );
        let cache = cell.cache();
        fs::create_dir_all(cache.path()).expect("cell dir must be created");
        fs::write(cache.path().join(META_FILE), b"{}").expect("meta written");
        fs::write(cache.path().join(VOICE_FILE), b"x").expect("voice written");
        let value = value_of(&record, home.path());
        assert_eq!(
            (
                value["cards"]["items"][0]["artifacts"]["meta"].as_bool(),
                value["cards"]["items"][0]["artifacts"]["scene"].as_bool(),
                value["cards"]["items"][0]["state"].as_str(),
            ),
            (Some(true), Some(false), Some("pending")),
            "a committed document must carry per-card artifact booleans probed from the cache"
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
