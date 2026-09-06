//! The generation verbs: `generate` (commit the plan and run the worker) and
//! `regenerate` (activate staged adjustments with `--pending`, retry only
//! missing stages with `--failed`, or fully re-roll one card with `--card`;
//! with `--note`, Gemini first rewrites that card).
//!
//! `run_session` is the shared commit-and-run step `new --generate` reuses.

use anyhow::Result;

use crate::cli::console::{HumanReporter, JsonReporter, Reporter, drafts_for};
use crate::cli::error::{json_line, operational_hint, usage, usage_hint};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{
    CardDraft, CardMetaCache, CardRewrite, LanguagePair, MAX_PLAN_CARDS, SentenceLabelSelection,
    WordCandidate,
};

use super::args::{GenerateArgs, RegenerateArgs};
use super::store::{DraftRecord, Phase, SessionRecord, SessionStore};
use super::{
    Render, drop_draft_artifacts, drop_incomplete_draft_artifacts, json, preflight_key,
    refuse_if_live, reset_to_understood, resolve, view, worker,
};

/// Commit the curated plan and start the managed worker that generates+publishes.
pub(super) fn generate(args: &GenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    run_session(&store, record.id.as_str(), args.wait, render, None, true)
}

/// Commit the plan (deriving it from the curation when none exists) and run the
/// worker, foreground with `--wait` or detached otherwise. `intro` is an
/// optional note (regenerate's "re-rolling …") printed under the header in plain
/// mode. In JSON mode the one stdout document is the session as the command
/// leaves it: freshly generating for a detached run, terminal after `--wait`.
/// A direct `generate` may resume an older cancelled session; continuations
/// from another verb preserve a cancellation that raced in after their setup.
pub(super) fn run_session(
    store: &SessionStore,
    id: &str,
    wait: bool,
    render: Render,
    intro: Option<String>,
    resume_cancelled: bool,
) -> Result<()> {
    refuse_staged_rewrites(&store.open(id)?)?;
    preflight_key()?;
    store.update(id, |record| {
        refuse_staged_rewrites(record)?;
        refuse_if_live(store, record)?;
        resume(record, resume_cancelled);
        ensure_plan(record)?;
        if record.drafts.is_empty() {
            return Err(usage(
                "nothing to generate — select at least one card first",
            ));
        }
        Ok(())
    })?;
    if wait {
        return run_wait(store, id, render, intro);
    }
    let record = worker::start_background(store, id)?;
    if matches!(render, Render::Json) {
        return json::emit_session(&record);
    }
    println!("{}", view::header(&record, Phase::Generating));
    println!("Building in the background — run status to watch.");
    println!("out: {}", record.out);
    Ok(())
}

fn resume(record: &mut SessionRecord, permitted: bool) {
    if permitted && matches!(record.phase, Phase::Cancelled) {
        reset_to_understood(record);
    }
}

fn refuse_staged_rewrites(record: &SessionRecord) -> Result<()> {
    if record.drafts.iter().any(|draft| {
        draft
            .rewrite
            .as_ref()
            .is_some_and(|rewrite| !rewrite.started())
    }) {
        return Err(usage_hint(
            format!(
                "session '{}' has staged card changes waiting for regeneration",
                record.id
            ),
            format!("Run: kamishibai regenerate {} --pending", record.id),
        ));
    }
    Ok(())
}

/// Run the foreground `--wait` generation: print the header (and any intro) on
/// stderr, stream the steps, then `done:` + paths on success, or one reshaped
/// operational line + hint on failure. fd 1 is muted for the whole run so the
/// native OCR engine's libc-buffered chatter never corrupts the one JSON
/// document, which is written to the saved real descriptor instead.
fn run_wait(store: &SessionStore, id: &str, render: Render, intro: Option<String>) -> Result<()> {
    let stdout = MutedStdout::capture()?;
    if matches!(render, Render::Text) {
        let record = store.open(id)?;
        eprintln!("{}", view::header(&record, Phase::Generating));
        if let Some(intro) = intro.as_deref() {
            eprintln!("{intro}");
        }
    }
    match worker::run_foreground(store, id, reporter(render)) {
        Ok(record) => {
            if matches!(render, Render::Json) {
                stdout.emit(json::session_line(&record)?.as_str())?;
            }
            Ok(())
        }
        Err(error) => {
            let reshaped = operational_hint(
                format!("{error:#}"),
                format!("Resolve the error above, then: kamishibai generate {id}"),
            );
            if matches!(render, Render::Json) {
                stdout.emit(json_line(&reshaped).as_str())?;
            }
            Err(reshaped)
        }
    }
}

/// Derive the committed plan from the curated candidates when none is committed.
///
/// Refuses a fresh plan larger than the batch ceiling. A plan already committed
/// is left alone, so a batch created before the ceiling existed still generates.
fn ensure_plan(record: &mut SessionRecord) -> Result<()> {
    if !record.drafts.is_empty() {
        return Ok(());
    }
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let candidates: Vec<WordCandidate> = record
        .candidates
        .iter()
        .map(|stored| stored.clone().candidate())
        .collect();
    let drafts = drafts_for(&candidates, &pair);
    if drafts.len() > MAX_PLAN_CARDS {
        return Err(usage_hint(
            format!(
                "too many cards: this plan makes {} cards, at most {MAX_PLAN_CARDS} per batch",
                drafts.len()
            ),
            format!(
                "Run exclude to drop cards, or select --sense to keep fewer senses, until the plan is {MAX_PLAN_CARDS} or fewer"
            ),
        ));
    }
    let requests = record.sentences.selections(drafts.len());
    record.drafts = drafts
        .into_iter()
        .zip(requests)
        .map(|(draft, request)| match request {
            Some(selection) => record_of(&draft.requesting_meta(selection)),
            None => record_of(&draft),
        })
        .collect();
    Ok(())
}

/// Activate every staged adjustment (`--pending`), retry unfinished committed
/// cards from their first missing stage (`--failed`), or fully re-roll one card
/// (`--card`), then immediately regenerate and republish the deck. Returns like
/// `generate`: the id for a detached run, the terminal state after `--wait`.
pub(super) fn regenerate(args: &RegenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    refuse_if_live(&store, &record)?;
    refuse_if_starting(&record)?;
    if record.drafts.is_empty() {
        return Err(usage_hint(
            "no committed plan to regenerate",
            "Generate it first: kamishibai generate",
        ));
    }
    if args.pending {
        refuse_without_staged(&record)?;
    } else {
        refuse_staged_rewrites(&record)?;
    }
    preflight_key()?;
    let intro = if args.pending {
        let updated = activate_pending(&store, record.id.as_str())?;
        pending_note(&updated)
    } else {
        match args.card.as_deref() {
            Some(card) if record.source == "cards" && args.note.is_none() => {
                drop_imported_card(&store, &record, card)?;
                reroll_note(&[String::from(card)], &record, false)
            }
            Some(card) => {
                queue_rewrite(&store, &record, card, args.note.as_deref().unwrap_or(""))?;
                if args.note.is_some() {
                    rewrite_note(card, &record)
                } else {
                    reroll_note(&[String::from(card)], &record, false)
                }
            }
            None => {
                let (_record, targets) = drop_targets(&store, &record)?;
                reroll_note(&targets, &record, true)
            }
        }
    };
    let result = run_session(
        &store,
        record.id.as_str(),
        args.wait,
        render,
        Some(intro),
        false,
    );
    if args.pending && result.is_err() {
        settle_unclaimed_pending(&store, record.id.as_str());
    }
    result
}

fn refuse_without_staged(record: &SessionRecord) -> Result<()> {
    if record.drafts.iter().any(|draft| {
        draft
            .rewrite
            .as_ref()
            .is_some_and(|rewrite| !rewrite.started())
    }) {
        return Ok(());
    }
    Err(usage("no pending card adjustments to regenerate"))
}

fn refuse_if_starting(record: &SessionRecord) -> Result<()> {
    if matches!(record.phase, Phase::Generating) && record.worker.is_none() {
        return Err(usage(format!(
            "session '{}' is starting generation; wait or cancel it first",
            record.id
        )));
    }
    Ok(())
}

fn activate_pending(store: &SessionStore, id: &str) -> Result<SessionRecord> {
    store.update(id, |record| {
        refuse_if_live(store, record)?;
        refuse_without_staged(record)?;
        for draft in &mut record.drafts {
            if draft
                .rewrite
                .as_ref()
                .is_some_and(|rewrite| !rewrite.started())
            {
                draft.rewrite = draft.rewrite.take().map(CardRewrite::activate);
            }
        }
        record.phase = Phase::Generating;
        record.worker = None;
        record.progress = None;
        record.result = None;
        record.error = None;
        Ok(())
    })
}

fn pending_note(record: &SessionRecord) -> String {
    let targets = record
        .drafts
        .iter()
        .filter(|draft| draft.rewrite.as_ref().is_some_and(CardRewrite::started))
        .map(|draft| draft.term.as_str())
        .collect::<Vec<_>>();
    format!(
        "Applying pending sentence changes to {}.",
        targets.join(", ")
    )
}

fn settle_unclaimed_pending(store: &SessionStore, id: &str) {
    let _ = store.update(id, |record| {
        refuse_if_live(store, record)?;
        if matches!(record.phase, Phase::Generating) && record.worker.is_none() {
            record.phase = Phase::Failed;
            record.error = Some(String::from("pending regeneration could not start"));
        }
        Ok(())
    });
}

fn drop_imported_card(
    store: &SessionStore,
    record: &SessionRecord,
    card: &str,
) -> Result<SessionRecord> {
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let (slot, current) = record
        .drafts
        .iter()
        .enumerate()
        .find(|(_slot, draft)| draft.term == card)
        .ok_or_else(|| usage(format!("no card '{card}' in session '{}'", record.id)))?;
    store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        refuse_if_starting(fresh)?;
        refuse_staged_rewrites(fresh)?;
        cancel_imported_rewrite(fresh, slot, current.term.as_str())?;
        drop_draft_artifacts(root.as_path(), &pair, current, true)?;
        reset_to_understood(fresh);
        Ok(())
    })
}

fn cancel_imported_rewrite(record: &mut SessionRecord, slot: usize, term: &str) -> Result<()> {
    let draft = record
        .drafts
        .get_mut(slot)
        .ok_or_else(|| usage(format!("no card slot {slot} in session '{}'", record.id)))?;
    if draft.term != term {
        return Err(usage(format!(
            "card slot {slot} names '{}' instead of '{}' in session '{}'",
            draft.term, term, record.id
        )));
    }
    draft.rewrite = None;
    Ok(())
}

/// The terms of the committed cards left untouched by a regenerate target set.
fn other_terms(record: &SessionRecord, targets: &[String]) -> Vec<String> {
    record
        .drafts
        .iter()
        .map(|draft| draft.term.clone())
        .filter(|term| !targets.contains(term))
        .collect()
}

/// The intro note for a stage retry or full dropped-and-rebuilt reroll.
fn reroll_note(targets: &[String], record: &SessionRecord, failed_only: bool) -> String {
    let kept = other_terms(record, targets);
    if failed_only {
        let mut note = format!("Retrying only missing stages for {}", targets.join(", "));
        if !kept.is_empty() {
            note.push_str(&format!(", keeping {}", kept.join(", ")));
        }
        note.push('.');
        return note;
    }
    let possessive = if targets.len() == 1 { "its" } else { "their" };
    let mut note = format!(
        "Re-rolling {} — dropping {possessive} audio and art",
        targets.join(", ")
    );
    if !kept.is_empty() {
        note.push_str(&format!(", keeping {}", kept.join(", ")));
    }
    note.push('.');
    note
}

/// The intro note for a Gemini-rewritten regenerate run.
fn rewrite_note(card: &str, record: &SessionRecord) -> String {
    let kept = other_terms(record, &[String::from(card)]);
    let mut note = format!("Rewrote {card} from your note — re-rolling it");
    if !kept.is_empty() {
        note.push_str(&format!(", keeping {}", kept.join(", ")));
    }
    note.push('.');
    note
}

/// Queue one full rewrite so the worker runs it through the metadata retry loop.
fn queue_rewrite(
    store: &SessionStore,
    record: &SessionRecord,
    card: &str,
    note: &str,
) -> Result<SessionRecord> {
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let (slot, current) = record
        .drafts
        .iter()
        .enumerate()
        .find(|(_slot, draft)| draft.term.as_str() == card)
        .ok_or_else(|| usage(format!("no card '{card}' in session '{}'", record.id)))?;
    let current = current.clone();
    let cell = super::cell_for_draft(root.as_path(), &pair, &current);
    let previous = CardMetaCache::new(root).load_at(&cell)?;
    let selection = previous
        .as_ref()
        .and_then(|meta| meta.sentence_labels())
        .map(SentenceLabelSelection::from_labels)
        .unwrap_or_default();
    let rewrite = CardRewrite::new(previous, selection, note);
    store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        refuse_if_starting(fresh)?;
        refuse_staged_rewrites(fresh)?;
        let draft = fresh
            .drafts
            .get_mut(slot)
            .ok_or_else(|| usage(format!("no card slot {slot} in session '{}'", fresh.id)))?;
        if draft.term != current.term {
            return Err(usage(format!(
                "card slot {slot} names '{}' instead of '{}' in session '{}'",
                draft.term, current.term, fresh.id
            )));
        }
        draft.rewrite = Some(rewrite);
        reset_to_understood(fresh);
        Ok(())
    })
}

/// Drop only missing stages for `--failed`, or every generated artifact for an
/// explicit `--card`, and reset the session so generation resumes immediately.
/// Returns the updated record plus the terms of the targeted cards.
fn drop_targets(
    store: &SessionStore,
    record: &SessionRecord,
) -> Result<(SessionRecord, Vec<String>)> {
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let mut terms = Vec::new();
    let updated = store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        refuse_if_starting(fresh)?;
        refuse_staged_rewrites(fresh)?;
        let targets: Vec<DraftRecord> = view::incomplete_drafts(fresh, root.as_path())
            .into_iter()
            .cloned()
            .collect();
        if targets.is_empty() {
            return Err(usage("no matching cards to regenerate"));
        }
        for draft in &targets {
            drop_incomplete_draft_artifacts(root.as_path(), &pair, draft)?;
        }
        terms = targets.iter().map(|draft| draft.term.clone()).collect();
        reset_to_understood(fresh);
        Ok(())
    })?;
    Ok((updated, terms))
}

fn record_of(draft: &CardDraft) -> DraftRecord {
    DraftRecord::from_draft(draft)
}

fn reporter(render: Render) -> Box<dyn Reporter> {
    match render {
        Render::Json => Box::new(JsonReporter),
        Render::Text => Box::new(HumanReporter),
    }
}

/// A muted process stdout: fd 1 is redirected to `/dev/null` and never restored,
/// so a foreground generation's native OCR chatter (which printf's into the libc
/// buffer on fd 1) drains there instead of corrupting the single-document
/// `--json` stdout. The one document the command owes stdout is written to the
/// saved real descriptor with `emit`. Only stdout is muted — stderr (live
/// progress) is untouched.
#[cfg(unix)]
struct MutedStdout {
    real: std::fs::File,
    _sink: std::fs::File,
}

#[cfg(unix)]
impl MutedStdout {
    /// Flush, dup the real stdout aside, and point fd 1 at `/dev/null`.
    fn capture() -> Result<Self> {
        use std::io::Write as _;
        std::io::stdout().flush()?;
        let real = std::fs::File::from(rustix::io::dup(std::io::stdout())?);
        let sink = std::fs::File::options()
            .read(true)
            .write(true)
            .open("/dev/null")?;
        rustix::stdio::dup2_stdout(&sink)?;
        Ok(Self { real, _sink: sink })
    }

    /// Write one line to the saved real stdout, bypassing the muted fd 1.
    fn emit(&self, line: &str) -> Result<()> {
        use std::io::Write as _;
        let mut real = &self.real;
        writeln!(real, "{line}")?;
        real.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{cancel_imported_rewrite, ensure_plan, resume};
    use crate::cli::session::store::{DraftRecord, Phase, SessionRecord};
    use crate::session::{
        ArtifactCosts, CandidateRecord, CardRewrite, SentenceBatchSettings, SentenceLabelSelection,
        SentenceLevel, SentenceTypeMix, WordCandidate,
    };

    #[test]
    fn committing_a_plan_persists_each_allocated_meta_request() {
        let mut record = SessionRecord::understood(
            String::from("fr-settings"),
            String::from("created"),
            String::from("EN"),
            String::from("FR"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard"), String::from("chouette")],
            vec![
                CandidateRecord::from_candidate(&WordCandidate::new("canard", "a duck", true)),
                CandidateRecord::from_candidate(&WordCandidate::new("chouette", "an owl", true)),
            ],
        )
        .with_sentences(SentenceBatchSettings::new(
            Some(SentenceLevel::B1),
            SentenceTypeMix::Mixed,
        ));
        let expected = record.sentences.selections(2);
        ensure_plan(&mut record).expect("a two-card plan must commit");
        assert_eq!(
            record
                .drafts
                .iter()
                .map(|draft| draft.meta_request.clone())
                .collect::<Vec<_>>(),
            expected,
            "committing a configured batch lost its allocated initial metadata requests"
        );
    }

    #[test]
    fn an_imported_reroll_without_a_note_cancels_the_failed_rewrite() {
        let mut record = SessionRecord::understood(
            String::from("fr-imported"),
            String::from("created"),
            String::from("EN"),
            String::from("FR"),
            String::from("/out"),
            String::from("primary"),
            String::from("cards"),
            Vec::new(),
            Vec::new(),
        );
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            reviewed_senses: Vec::new(),
            costs: ArtifactCosts::default(),
            rewrite: Some(CardRewrite::new(
                None,
                SentenceLabelSelection::empty(),
                "make it formal",
            )),
            meta_request: None,
        }];
        cancel_imported_rewrite(&mut record, 0, "canard")
            .expect("the imported reroll must be reset");
        assert_eq!(
            record.drafts[0].rewrite, None,
            "an imported no-note reroll repeated an earlier failed correction"
        );
    }

    #[test]
    fn a_regeneration_continuation_preserves_a_racing_cancellation() {
        let mut record = SessionRecord::understood(
            String::from("fr-cancelled"),
            String::from("created"),
            String::from("EN"),
            String::from("FR"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            Vec::new(),
            Vec::new(),
        );
        record.phase = Phase::Cancelled;
        resume(&mut record, false);
        assert_eq!(
            record.phase,
            Phase::Cancelled,
            "a pending regeneration continuation resurrected a cancelled session"
        );
    }
}

/// On non-Unix the OCR redirect is a no-op, so muting is unnecessary; `emit`
/// writes straight to stdout.
#[cfg(not(unix))]
struct MutedStdout;

#[cfg(not(unix))]
impl MutedStdout {
    fn capture() -> Result<Self> {
        Ok(Self)
    }

    fn emit(&self, line: &str) -> Result<()> {
        println!("{line}");
        Ok(())
    }
}
