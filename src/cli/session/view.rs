//! Cache-derived status projection: one computation feeding both renders (the
//! plain-text blocks here, the JSON documents in `json`).
//!
//! Readiness is the cache truth — for each draft we check meta and audio in the
//! shared `CardCell` folder plus scene and picture in the current visual-policy
//! revision. The session record only supplies phase and liveness. A card
//! incomplete under a terminal phase is a failed card; under `generating` it is
//! still building. Before a plan is committed (no drafts) the projection lists
//! the curatable candidates instead.

use std::fmt::Write;
use std::path::Path;

use crate::generation::artifact_cache::{ILLUSTRATION_FILE, VOICE_FILE};
use crate::generation::visual_revision;
use crate::session::{CardCell, LanguagePair, WordCandidate};

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
    LanguagePair::new(record.learning.as_str(), record.known.as_str())
}

/// Probe the shared cache for one card's four artifact files, in display order.
pub(super) fn probe_artifacts(
    cache_root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
) -> [bool; 4] {
    let cache = CardCell::new(cache_root, pair, term, understanding).cache();
    let visual = cache
        .visual(visual_revision())
        .expect("invariant: production visual revision must be one SHA-256 digest");
    [
        super::cached_meta_is_valid(cache_root, pair, term, understanding),
        cache.exists(VOICE_FILE),
        super::cached_scene_is_valid(&visual),
        visual.exists(ILLUSTRATION_FILE),
    ]
}

fn probe_draft(cache_root: &Path, pair: &LanguagePair, draft: &DraftRecord) -> [bool; 4] {
    if draft
        .rewrite
        .as_ref()
        .is_some_and(crate::session::CardRewrite::started)
        || awaits_initial_meta(draft)
    {
        return [false; 4];
    }
    probe_artifacts(
        cache_root,
        pair,
        draft.term.as_str(),
        draft.understanding.as_str(),
    )
}

/// Return whether a draft still needs its initial requested metadata.
pub(super) fn awaits_initial_meta(draft: &DraftRecord) -> bool {
    draft.rewrite.is_none() && draft.meta_request.is_some()
}

/// Probe every committed draft against the cache, in plan order.
pub(super) fn cards(record: &SessionRecord, cache_root: &Path) -> Vec<CardView> {
    let pair = pair_of(record);
    record
        .drafts
        .iter()
        .map(|draft| {
            let [meta, sound, scene, picture] = probe_draft(cache_root, &pair, draft);
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

/// Map one phase to its lowercase status word.
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
            probe_draft(cache_root, &pair, draft)
                .iter()
                .any(|present| !present)
        })
        .collect()
}

/// The shared first line of every single-session command: identity, direction,
/// and live phase. Language codes are uppercased — the app's canonical form.
pub(super) fn header(record: &SessionRecord, phase: Phase) -> String {
    format!(
        "your session {} · {} → {} · {}",
        record.id,
        record.known.to_uppercase(),
        record.learning.to_uppercase(),
        phase_label(phase)
    )
}

/// Render the full session status: the understood candidate block before a plan
/// is committed, the per-card artifact matrix once it is.
pub(super) fn render_status(record: &SessionRecord, cache_root: &Path) -> String {
    if record.drafts.is_empty() {
        render_understood(record)
    } else {
        render_committed(record, cache_root)
    }
}

/// Render the understood block both `new` and `status` print: the header, a
/// one-line summary, the words with their senses (`*` marks the sense that
/// becomes a card), and the curation guidance. No cache probing — an understood
/// session has no artifacts yet.
pub(super) fn render_understood(record: &SessionRecord) -> String {
    let candidates: Vec<WordCandidate> = record
        .candidates
        .iter()
        .map(|stored| stored.clone().candidate())
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "{}", header(record, Phase::Understood));
    let _ = writeln!(out, "{}", understood_intro(record, &candidates));
    let _ = writeln!(out, "words:");
    for candidate in &candidates {
        let _ = writeln!(out, "  {}", candidate.term());
        for (index, sense) in candidate.senses().iter().enumerate() {
            let chosen = candidate.ok() && candidate.selected_senses().contains(&index);
            let mark = if chosen { '*' } else { ' ' };
            let _ = writeln!(
                out,
                "    {mark} {} {}{}",
                index + 1,
                tag_prefix(sense.tag()),
                sense.understanding()
            );
        }
    }
    let _ = write!(out, "{}", understood_guidance(record, &candidates));
    out
}

/// The one-line summary under the header of an understood session.
fn understood_intro(record: &SessionRecord, candidates: &[WordCandidate]) -> String {
    let count = candidates.len();
    if record.source == "cards" {
        return format!(
            "Imported {count} {} from your JSON, ready to generate.",
            cards_word(count)
        );
    }
    if candidates
        .iter()
        .any(|candidate| candidate.senses().len() > 1)
    {
        format!(
            "I understood {count} {}. * = the sense that becomes a card.",
            words_word(count)
        )
    } else {
        format!(
            "I understood {count} {} — one sense each, so each is a card.",
            words_word(count)
        )
    }
}

/// The curation guidance lines closing an understood block: how to repick a
/// sense or drop a card, then how to generate. Commands omit the id — an
/// omitted id resolves to this session.
fn understood_guidance(record: &SessionRecord, candidates: &[WordCandidate]) -> String {
    if record.source == "cards" {
        return String::from("Generate: kamishibai generate");
    }
    if let Some(multi) = candidates
        .iter()
        .find(|candidate| candidate.senses().len() > 1)
    {
        let sense = first_unselected(multi);
        return format!(
            "Change a card: kamishibai select {term} --sense {sense} (or drop it: kamishibai exclude {term})\nGenerate: kamishibai generate",
            term = multi.term()
        );
    }
    match candidates.first() {
        Some(first) => format!(
            "Generate: kamishibai generate (or drop a word: kamishibai exclude {})",
            first.term()
        ),
        None => String::from("Generate: kamishibai generate"),
    }
}

/// Return the 1-based number of one card's first not-yet-selected sense, to
/// suggest in the `select` guidance; defaults to 2 when all are selected.
fn first_unselected(candidate: &WordCandidate) -> usize {
    (0..candidate.senses().len())
        .find(|index| !candidate.selected_senses().contains(index))
        .map(|index| index + 1)
        .unwrap_or(2)
}

/// Render the per-card artifact matrix for a committed plan: the header, the
/// one-line outcome, one glyph row per card, the output directory, and a single
/// `next:` step when the run is stuck.
fn render_committed(record: &SessionRecord, cache_root: &Path) -> String {
    let (phase, pid, _) = live_phase(record, cache_root);
    let cards = cards(record, cache_root);
    let ready = cards.iter().filter(|card| card.ready()).count();
    let total = cards.len();
    let mut out = String::new();
    let _ = writeln!(out, "{}", header(record, phase));
    let _ = writeln!(
        out,
        "{}",
        committed_summary(phase, total, ready, total - ready, pid)
    );
    let mark_fail = matches!(phase, Phase::Failed | Phase::Partial);
    for card in &cards {
        let _ = writeln!(
            out,
            "  {} {}",
            card.term,
            artifact_row([card.meta, card.sound, card.scene, card.picture], mark_fail)
        );
    }
    let _ = write!(out, "out: {}", record.out);
    if let Some(next) = next_step(phase) {
        let _ = write!(out, "\n{next}");
    }
    out
}

/// Render one card's four artifacts as labelled glyphs: `✓` present, `✗` the
/// give-up point (first absent artifact, only under a failed/partial run), `·`
/// not yet reached or in work.
fn artifact_row(present: [bool; 4], mark_fail: bool) -> String {
    let labels = ["meaning", "audio", "scene", "picture"];
    let mut gave_up = false;
    let mut parts = Vec::with_capacity(4);
    for (index, ready) in present.iter().enumerate() {
        let glyph = if *ready {
            "✓"
        } else if mark_fail && !gave_up {
            gave_up = true;
            "✗"
        } else {
            "·"
        };
        parts.push(format!("{} {glyph}", labels[index]));
    }
    parts.join(" ")
}

/// The one-line, plain-language outcome under the header of a committed session.
pub(super) fn committed_summary(
    phase: Phase,
    total: usize,
    ready: usize,
    failed: usize,
    pid: Option<i32>,
) -> String {
    match phase {
        Phase::Generating => format!(
            "building {total} {} ({})",
            cards_word(total),
            pid_label(pid)
        ),
        Phase::Published if total == 1 => String::from("the card is in the deck"),
        Phase::Published => format!("all {total} cards are in the deck"),
        Phase::Partial => {
            format!("{ready} of {total} cards in the deck, {failed} couldn't be built")
        }
        Phase::Failed => String::from("couldn't build any card — no deck"),
        Phase::Interrupted => format!(
            "{ready} {} built, but the worker stopped before publishing ({})",
            cards_word(ready),
            gone_label(pid)
        ),
        Phase::Cancelled => format!("you stopped the worker, {ready} of {total} cards built"),
        Phase::Understood => String::new(),
    }
}

/// The single `next:` step for a stuck session, or none when it is healthy.
pub(super) fn next_step(phase: Phase) -> Option<&'static str> {
    match phase {
        Phase::Failed | Phase::Partial => Some("next: kamishibai regenerate --failed"),
        Phase::Cancelled | Phase::Interrupted => Some("next: kamishibai generate"),
        _ => None,
    }
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

/// Render one `ls` list line: `<id>  <known> → <learning>  <phase>  <progress>`,
/// columns separated by two spaces. Before a plan is committed the trailing
/// column shows `-- / <selected>` (a curation count, not progress); once a plan
/// is committed it shows `<ready>/<total>`.
pub(super) fn summary_line(record: &SessionRecord, cache_root: &Path) -> String {
    let (phase, _, _) = live_phase(record, cache_root);
    let progress = if record.drafts.is_empty() {
        format!("-- / {}", selected_cards(record))
    } else {
        let cards = cards(record, cache_root);
        let ready = cards.iter().filter(|card| card.ready()).count();
        format!("{}/{}", ready, cards.len())
    };
    format!(
        "{}  {} → {}  {:<10}  {progress}",
        record.id,
        record.known.to_uppercase(),
        record.learning.to_uppercase(),
        phase_label(phase),
    )
}

/// Pluralise the card noun.
fn cards_word(count: usize) -> &'static str {
    if count == 1 { "card" } else { "cards" }
}

/// Pluralise the word noun.
fn words_word(count: usize) -> &'static str {
    if count == 1 { "word" } else { "words" }
}

/// Render one sense's tag as a `(tag) ` prefix, or nothing when untagged.
fn tag_prefix(tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("({tag}) "),
        None => String::new(),
    }
}

/// Render a live worker's pid for the generating summary.
fn pid_label(pid: Option<i32>) -> String {
    match pid {
        Some(pid) => format!("pid {pid}"),
        None => String::from("running"),
    }
}

/// Render a gone worker's pid for the interrupted summary.
fn gone_label(pid: Option<i32>) -> String {
    match pid {
        Some(pid) => format!("pid {pid} gone"),
        None => String::from("the worker is gone"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::generation::artifact_cache::{META_FILE, SCENE_FILE};
    use crate::session::{
        CardCell, CardDraft, CardMeta, CardMetaCache, CardRewrite, SentenceAxis,
        SentenceLabelSelection,
    };

    fn store_meta(home: &TempDir) {
        let meta = CardMeta::new(
            "/ka.naʁ/",
            "/lə ka.naʁ naʒ/",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A common concrete noun",
            "Le canard nage",
        );
        CardMetaCache::new(home.path())
            .store("canard", "a duck", &LanguagePair::new("fr", "en"), &meta)
            .expect("valid meta fixture must be stored");
    }

    fn store_artifacts(home: &TempDir) {
        store_meta(home);
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path(), &pair, "canard", "a duck").cache();
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
            costs: crate::session::ArtifactCosts::default(),
            rewrite: None,
            meta_request: None,
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
        store_meta(&home);
        let cell = CardCell::new(
            home.path(),
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        );
        let cache = cell.cache();
        fs::write(cache.path().join(VOICE_FILE), b"x").expect("voice written");
        let status = render_status(&record(), home.path());
        assert!(
            status.contains("canard meaning ✓ audio ✓ scene · picture ·"),
            "status text must show each card's artifact presence read from the cache"
        );
    }

    #[test]
    fn status_text_hides_visuals_from_an_outdated_policy_revision() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = CardCell::new(
            home.path(),
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        )
        .cache();
        let outdated = cache
            .visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("outdated revision must be valid");
        for file in [SCENE_FILE, ILLUSTRATION_FILE] {
            fs::write(
                outdated.filepath(file).expect("visual path must resolve"),
                b"x",
            )
            .expect("visual written");
        }
        let status = render_status(&record(), home.path());
        assert!(
            status.contains("canard meaning · audio · scene · picture ·"),
            "status must not advertise visual artifacts from an outdated policy revision"
        );
    }

    #[test]
    fn status_text_shows_visuals_from_the_current_policy_revision() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = CardCell::new(
            home.path(),
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        )
        .cache();
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::write(
            visual
                .filepath(SCENE_FILE)
                .expect("visual path must resolve"),
            include_bytes!("../../../tests/fixtures/production-scene.json"),
        )
        .expect("valid scene written");
        fs::write(
            visual
                .filepath(ILLUSTRATION_FILE)
                .expect("visual path must resolve"),
            b"x",
        )
        .expect("picture written");
        let status = render_status(&record(), home.path());
        assert!(
            status.contains("canard meaning · audio · scene ✓ picture ✓"),
            "status must advertise visual artifacts from the current policy revision"
        );
    }

    #[test]
    fn an_activated_rewrite_hides_stale_artifacts_and_remains_retryable() {
        let home = TempDir::new().expect("tempdir must be created");
        store_artifacts(&home);
        let mut record = record();
        record.drafts[0].rewrite =
            Some(CardRewrite::new(None, SentenceLabelSelection::empty(), ""));
        let projected = cards(&record, home.path());
        let incomplete = incomplete_drafts(&record, home.path());
        assert_eq!(
            (projected[0].ready(), incomplete.len()),
            (false, 1),
            "a queued rewrite exposed stale readiness or disappeared from failed retry"
        );
    }

    #[test]
    fn a_pending_initial_meta_request_hides_rollback_artifacts_and_remains_retryable() {
        let home = TempDir::new().expect("tempdir must be created");
        store_artifacts(&home);
        let mut record = record();
        record.drafts[0].meta_request =
            Some(SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 4));
        let card = cards(&record, home.path()).remove(0);
        let incomplete = incomplete_drafts(&record, home.path());
        assert_eq!(
            (
                [card.meta, card.sound, card.scene, card.picture],
                incomplete.len(),
            ),
            ([false; 4], 1),
            "a failed initial metadata refresh exposed rollback artifacts or disappeared from failed retry"
        );
    }

    #[test]
    fn a_staged_rewrite_preserves_cached_readiness_until_batch_start() {
        let home = TempDir::new().expect("tempdir must be created");
        store_artifacts(&home);
        let staged = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"))
            .staging_rewrite(SentenceLabelSelection::empty(), "make it formal");
        let mut record = record();
        record.drafts[0].rewrite = staged.rewrite().cloned();
        let projected = cards(&record, home.path());
        let incomplete = incomplete_drafts(&record, home.path());
        assert_eq!(
            (projected[0].ready(), incomplete.len()),
            (true, 0),
            "a staged rewrite hid current cache readiness before batch activation"
        );
    }

    #[test]
    fn structurally_invalid_meta_is_reported_missing() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path(), &pair, "canard", "a duck").cache();
        fs::write(
            cache.filepath(META_FILE).expect("meta path must resolve"),
            br#"{"manga_panel":{"panels":[{}]}}"#,
        )
        .expect("invalid meta written");
        assert_eq!(
            probe_artifacts(home.path(), &pair, "canard", "a duck"),
            [false, false, false, false],
            "status must not report a structurally invalid meta as ready"
        );
    }

    #[test]
    fn structurally_invalid_scene_is_reported_missing() {
        let home = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("fr", "en");
        let cache = CardCell::new(home.path(), &pair, "canard", "a duck").cache();
        let visual = cache
            .visual(visual_revision())
            .expect("production revision must be valid");
        fs::write(
            visual
                .filepath(SCENE_FILE)
                .expect("scene path must resolve"),
            b"{}",
        )
        .expect("invalid scene written");
        fs::write(
            visual
                .filepath(ILLUSTRATION_FILE)
                .expect("picture path must resolve"),
            b"x",
        )
        .expect("picture written");
        assert_eq!(
            probe_artifacts(home.path(), &pair, "canard", "a duck"),
            [false, false, false, true],
            "status must not report a structurally invalid scene as ready"
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
            status.contains("words:")
                && status.contains("  canard")
                && status.contains("    * 2 a hoax"),
            "an understood session must list candidate senses with the selected one marked"
        );
    }
}
