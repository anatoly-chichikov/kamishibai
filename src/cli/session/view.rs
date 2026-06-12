//! Cache-derived status projection: one computation feeding both renders (the
//! plain-text blocks here, the JSON documents in `json`).
//!
//! Readiness is the cache truth — for each draft we check whether its four
//! artifact files exist in the shared `CardCell` folder. The session record only
//! supplies phase and liveness. A card incomplete under a terminal phase is a
//! failed card; under `generating` it is still building. Before a plan is
//! committed (no drafts) the projection lists the curatable candidates instead.

use std::fmt::Write;
use std::path::Path;

use crate::generation::artifact_cache::{ILLUSTRATION_FILE, META_FILE, SCENE_FILE, VOICE_FILE};
use crate::session::{CardCell, LanguagePair};

use super::liveness;
use super::store::{DraftRecord, LOCK_FILE, Phase, SessionRecord};

/// One card's cache-derived readiness, shared by the text and JSON renders.
pub(super) struct CardView {
    pub(super) term: String,
    pub(super) understanding: String,
    pub(super) meta: bool,
    pub(super) sound: bool,
    pub(super) scene: bool,
    pub(super) picture: bool,
}

impl CardView {
    pub(super) fn ready(&self) -> bool {
        self.meta && self.sound && self.scene && self.picture
    }
}

fn pair_of(record: &SessionRecord) -> LanguagePair {
    LanguagePair::new(record.to.as_str(), record.from.as_str())
}

/// Probe the shared cache for one card's four artifact files, in display order.
pub(super) fn probe_artifacts(
    cache_root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> [bool; 4] {
    let cache = CardCell::new(cache_root, pair, term, understanding).cache();
    [
        cache.exists(META_FILE),
        cache.exists(VOICE_FILE),
        cache.exists(SCENE_FILE),
        cache.exists(ILLUSTRATION_FILE),
    ]
}

/// Probe every committed draft against the cache, in plan order.
pub(super) fn cards(record: &SessionRecord, cache_root: &Path) -> Vec<CardView> {
    let pair = pair_of(record);
    record
        .drafts
        .iter()
        .map(|draft| {
            let [meta, sound, scene, picture] =
                probe_artifacts(cache_root, &pair, &draft.term, &draft.understanding);
            CardView {
                term: draft.term.clone(),
                understanding: draft.understanding.clone(),
                meta,
                sound,
                scene,
                picture,
            }
        })
        .collect()
}

/// Reconcile the stored phase with worker liveness, returning (phase, pid, live).
///
/// Liveness is the advisory lock, not pid existence: a worker is live only while
/// it holds the session lock, so a stale (possibly reused) pid reads as
/// `Interrupted` rather than faking a running worker.
pub(super) fn live_phase(record: &SessionRecord, cache_root: &Path) -> (Phase, Option<i32>, bool) {
    match record.worker.as_ref() {
        Some(handle) if liveness::is_held(&lock_path(cache_root, record.id.as_str())) => {
            (Phase::Generating, Some(handle.pid), true)
        }
        Some(handle) => (Phase::Interrupted, Some(handle.pid), false),
        None => (record.phase, None, false),
    }
}

fn lock_path(cache_root: &Path, id: &str) -> std::path::PathBuf {
    cache_root.join("sessions").join(id).join(LOCK_FILE)
}

/// Return the single phase word for `status -q` (e.g. `published`).
pub(super) fn phase_word(record: &SessionRecord, cache_root: &Path) -> &'static str {
    phase_label(live_phase(record, cache_root).0)
}

/// Return whether a phase is terminal: incomplete cards under it read failed.
/// Includes `Interrupted` — deliberately wider than the resolution-side
/// `settled` (mod.rs), where an interrupted run still counts as unfinished.
pub(super) fn terminal(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Published | Phase::Partial | Phase::Failed | Phase::Interrupted | Phase::Cancelled
    )
}

/// Map one phase to its lowercase status word (`status -q` prints this).
pub(super) fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Understood => "understood",
        Phase::Generating => "generating",
        Phase::Interrupted => "interrupted",
        Phase::Published => "published",
        Phase::Partial => "partial",
        Phase::Failed => "failed",
        Phase::Cancelled => "cancelled",
    }
}

/// Return the drafts whose artifacts are not all present (failed or unfinished).
pub(super) fn incomplete_drafts<'a>(
    record: &'a SessionRecord,
    cache_root: &Path,
) -> Vec<&'a DraftRecord> {
    let pair = pair_of(record);
    record
        .drafts
        .iter()
        .filter(|draft| {
            probe_artifacts(cache_root, &pair, &draft.term, &draft.understanding)
                .iter()
                .any(|present| !present)
        })
        .collect()
}

/// Render the full session status as a stable, line-oriented text block.
pub(super) fn render_status(record: &SessionRecord, cache_root: &Path) -> String {
    let (phase, pid, live) = live_phase(record, cache_root);
    let mut out = String::new();
    let _ = writeln!(out, "session  {}", record.id);
    let _ = writeln!(out, "pair     {} → {}", record.from, record.to);
    let _ = writeln!(out, "senses   {}", record.senses);
    let _ = writeln!(out, "phase    {}", phase_label(phase));
    if let Some(pid) = pid {
        let alive = if live { "alive" } else { "gone" };
        let _ = writeln!(out, "worker   pid {pid} {alive}");
    }
    if record.drafts.is_empty() {
        let _ = write!(out, "{}", candidate_block(record));
    } else {
        let _ = write!(out, "{}", card_block(record, cache_root, phase));
    }
    let _ = write!(out, "out      {}", record.out);
    out
}

/// Render the per-card readiness block for a committed plan.
fn card_block(record: &SessionRecord, cache_root: &Path, phase: Phase) -> String {
    let cards = cards(record, cache_root);
    let ready = cards.iter().filter(|card| card.ready()).count();
    let failed = if terminal(phase) {
        cards.len() - ready
    } else {
        0
    };
    let width = cards
        .iter()
        .map(|card| card.term.chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "cards    {} total · {ready} ready · {failed} failed",
        cards.len()
    );
    for card in &cards {
        let _ = writeln!(
            out,
            "card  {term:<width$}   meta:{m} sound:{s} scene:{sc} picture:{p}   {label}",
            term = card.term,
            m = token(card.meta),
            s = token(card.sound),
            sc = token(card.scene),
            p = token(card.picture),
            label = row_label(card, phase)
        );
    }
    out
}

/// Render the curatable candidate block shown before a plan is committed.
fn candidate_block(record: &SessionRecord) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "words    {} understood · {} card(s) selected",
        record.candidates.len(),
        selected_cards(record)
    );
    for stored in &record.candidates {
        let candidate = stored.clone().candidate();
        let gate = if candidate.ok() { "card" } else { "skip" };
        let _ = writeln!(out, "word  {}   {gate}", candidate.term());
        for (index, sense) in candidate.senses().iter().enumerate() {
            let chosen = candidate.ok() && candidate.selected_senses().contains(&index);
            let mark = if chosen { '*' } else { ' ' };
            let _ = writeln!(
                out,
                "  {mark} {number:>2}  {tag:<7} {understanding}",
                number = index + 1,
                tag = sense.tag().unwrap_or(""),
                understanding = sense.understanding()
            );
        }
    }
    out
}

/// Count how many cards the current candidate selection would generate.
pub(super) fn selected_cards(record: &SessionRecord) -> usize {
    record
        .candidates
        .iter()
        .map(|stored| {
            let candidate = stored.clone().candidate();
            if candidate.ok() {
                candidate.selected_senses().len()
            } else {
                0
            }
        })
        .sum()
}

/// Render one session list line: `<id>  <from> → <to>  <phase>  <progress>`.
///
/// Before a plan is committed the trailing column shows `-- / <selected>` (a
/// curation count, not progress); once generation has a committed plan it shows
/// `<ready>/<total>`, so the list never reads an understood session as 0-done.
pub(super) fn summary_line(record: &SessionRecord, cache_root: &Path) -> String {
    let cards = cards(record, cache_root);
    let (phase, _, _) = live_phase(record, cache_root);
    let progress = if record.drafts.is_empty() {
        format!("-- / {}", selected_cards(record))
    } else {
        let ready = cards.iter().filter(|card| card.ready()).count();
        format!("{}/{}", ready, cards.len())
    };
    format!(
        "{}  {} → {}  {}  {}",
        record.id,
        record.from,
        record.to,
        phase_label(phase),
        progress
    )
}

fn token(present: bool) -> &'static str {
    if present { "ok" } else { "--" }
}

/// Map one card's readiness under a phase to its state word.
pub(super) fn row_label(card: &CardView, phase: Phase) -> &'static str {
    if card.ready() {
        "ready"
    } else if terminal(phase) {
        "failed"
    } else if matches!(phase, Phase::Generating) {
        "building"
    } else {
        "pending"
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::session::CardCell;

    fn record() -> SessionRecord {
        let mut record = SessionRecord::understood(
            String::from("fr-1"),
            String::from("2026-06-06T00:00:00Z"),
            String::from("en"),
            String::from("fr"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard")],
            Vec::new(),
        );
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
        }];
        record
    }

    #[test]
    fn a_recorded_worker_without_a_held_lock_reads_interrupted() {
        // The #3 fix: a stale (possibly pid-reused) worker whose lock is not held
        // must not fake a running worker. The held → generating path is the
        // cross-process worker-vs-status case, covered by the offline session e2e.
        use super::super::store::WorkerHandle;
        let home = TempDir::new().expect("tempdir must be created");
        let mut record = record();
        record.worker = Some(WorkerHandle {
            pid: 999_999,
            started: String::from("t"),
        });
        let phase = live_phase(&record, home.path()).0;
        assert_eq!(
            phase,
            Phase::Interrupted,
            "a recorded worker whose lock is not held must read as interrupted, not generating"
        );
    }

    #[test]
    fn status_text_shows_artifact_presence_from_the_cache() {
        let home = TempDir::new().expect("tempdir must be created");
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
        let status = render_status(&record(), home.path());
        assert!(
            status.contains("meta:ok sound:ok scene:-- picture:--"),
            "status text must show each card's artifact presence read from the cache"
        );
    }

    #[test]
    fn an_understood_list_line_shows_a_selection_count_not_a_progress_fraction() {
        use crate::session::{CandidateRecord, Sense, WordCandidate};
        let mut record = record();
        record.drafts = Vec::new();
        record.candidates = vec![CandidateRecord::from_candidate(
            &WordCandidate::with_selected_senses(
                "canard",
                vec![Sense::plain("a duck")],
                vec![0],
                true,
            ),
        )];
        let home = TempDir::new().expect("tempdir must be created");
        let line = summary_line(&record, home.path());
        assert!(
            line.contains("understood") && line.contains("-- / 1"),
            "an understood session list line must show a selection count, not a 0-done progress fraction"
        );
    }

    #[test]
    fn status_lists_candidate_senses_before_a_plan_is_committed() {
        use crate::session::{CandidateRecord, Sense, WordCandidate};
        let mut record = record();
        record.drafts = Vec::new();
        record.candidates = vec![CandidateRecord::from_candidate(
            &WordCandidate::with_selected_senses(
                "canard",
                vec![Sense::plain("a duck"), Sense::plain("a hoax")],
                vec![1],
                true,
            ),
        )];
        let home = TempDir::new().expect("tempdir must be created");
        let status = render_status(&record, home.path());
        assert!(
            status.contains("word  canard   card") && status.contains("*  2  "),
            "an understood session must list candidate senses with the selected one marked"
        );
    }
}
