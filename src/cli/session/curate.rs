//! Curation handlers: pick senses, exclude a card, or ask Gemini to add senses.
//!
//! Each edits one candidate by term before generation commits a plan. Every
//! edit runs inside one `store.update` closure, so concurrent curation commands
//! serialize and all apply. A nonempty draft plan is immutable here; generated
//! cards are changed only through `regenerate`, preserving stable cost-journal
//! slots. The shared preconditions (`resolve`, `refuse_if_live`,
//! `reset_to_understood`, `preflight_key`) live in the parent module.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::console;
use crate::cli::error::usage;
use crate::session::{BulkCorrection, CandidateRecord, LanguagePair, WordCandidate};

use super::args::{CardArg, CorrectArgs, SelectArgs};
use super::store::{Phase, SessionRecord, SessionStore};
use super::{Render, json, preflight_key, refuse_if_live, reset_to_understood, resolve, view};

/// Run one pre-generation curation edit against a candidate found by term.
/// Returns the record as updated, for the JSON document.
fn curate(
    store: &SessionStore,
    id: &str,
    card: &str,
    edit: impl FnOnce(WordCandidate) -> Result<WordCandidate>,
) -> Result<SessionRecord> {
    store.update(id, |record| {
        refuse_if_live(store, record)?;
        refuse_if_committed(record)?;
        let index = record
            .candidates
            .iter()
            .position(|candidate| candidate.term() == card)
            .ok_or_else(|| usage(format!("no card '{card}' in session '{id}'")))?;
        let updated = edit(record.candidates[index].clone().candidate())?;
        record.candidates[index] = CandidateRecord::from_candidate(&updated);
        reset_to_understood(record);
        Ok(())
    })
}

/// Refuse curation after a stable generation plan has acquired journal slots.
fn refuse_if_committed(record: &SessionRecord) -> Result<()> {
    if record.drafts.is_empty() {
        return Ok(());
    }
    Err(usage(format!(
        "session '{}' already has a committed plan — use 'kamishibai regenerate {} --card <term>' to change a generated card",
        record.id, record.id
    )))
}

/// Print the post-mutation state: the session document in JSON mode, or the
/// understood header plus the verb's one-line confirmation note in plain mode.
fn conclude(render: Render, record: &SessionRecord, note: String) -> Result<()> {
    if matches!(render, Render::Json) {
        return json::emit_session(record);
    }
    println!("{}", view::header(record, Phase::Understood));
    println!("{note}");
    Ok(())
}

/// Find one candidate in a record by its term, rebuilt for reading its senses.
fn candidate_of(record: &SessionRecord, term: &str) -> Option<WordCandidate> {
    record
        .candidates
        .iter()
        .find(|candidate| candidate.term() == term)
        .map(|candidate| candidate.clone().candidate())
}

/// Pick which 1-based senses of a card become cards, re-including it.
pub(super) fn select(args: &SelectArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let target = resolve(&store, args.id.as_deref(), render)?;
    let senses = args.sense.clone();
    let updated = curate(
        &store,
        target.id.as_str(),
        args.card.as_str(),
        |candidate| {
            let count = candidate.senses().len();
            let mut zero = Vec::with_capacity(senses.len());
            for number in &senses {
                if *number < 1 || *number > count {
                    return Err(usage(format!(
                        "sense {number} out of range (1..={count}) for '{}'",
                        candidate.term()
                    )));
                }
                zero.push(number - 1);
            }
            Ok(candidate.selecting_senses(zero).with_ok(true))
        },
    )?;
    let note = select_note(&updated, args);
    conclude(render, &updated, note)
}

/// The confirmation line for a `select`: the kept sense quoted for one pick, the
/// list of numbers for several.
fn select_note(record: &SessionRecord, args: &SelectArgs) -> String {
    if let [number] = args.sense.as_slice() {
        let understanding = candidate_of(record, args.card.as_str())
            .and_then(|candidate| {
                candidate
                    .senses()
                    .get(number - 1)
                    .map(|sense| sense.understanding().to_string())
            })
            .unwrap_or_default();
        return format!(
            "Kept sense {number} of {} — \"{understanding}\".",
            args.card
        );
    }
    let list = args
        .sense
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("Kept senses {list} of {}.", args.card)
}

/// Drop one card from the plan while keeping it visible in the understanding.
pub(super) fn exclude(args: &CardArg, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let target = resolve(&store, args.id.as_deref(), render)?;
    let updated = curate(
        &store,
        target.id.as_str(),
        args.card.as_str(),
        |candidate| Ok(candidate.with_ok(false)),
    )?;
    let remaining = view::selected_cards(&updated);
    let note = format!(
        "Dropped {} — stays in the understanding, won't become a card. Now {remaining} {}.",
        args.card,
        if remaining == 1 { "card" } else { "cards" }
    );
    conclude(render, &updated, note)
}

/// Ask Gemini to add senses to one card from a note, keeping the prior selection.
///
/// The Gemini call runs outside the session's write lock; the correction is
/// merged onto the freshly read candidate inside the `update` closure.
pub(super) fn correct(args: &CorrectArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    let id = record.id.clone();
    refuse_if_live(&store, &record)?;
    refuse_if_committed(&record)?;
    preflight_key()?;
    let snapshot = record
        .candidates
        .iter()
        .find(|candidate| candidate.term() == args.card.as_str())
        .ok_or_else(|| usage(format!("no card '{}' in session '{id}'", args.card)))?
        .clone()
        .candidate();
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let generator = console::generator(PathBuf::from(record.out.clone()))?;
    let correction = generator.correct_bulk(&snapshot, args.note.as_str(), &pair)?;
    let mut appended = 0;
    let updated = store.update(id.as_str(), |record| {
        refuse_if_live(&store, record)?;
        refuse_if_committed(record)?;
        let index = record
            .candidates
            .iter()
            .position(|candidate| candidate.term() == args.card.as_str())
            .ok_or_else(|| usage(format!("no card '{}' in session '{id}'", args.card)))?;
        let candidate = record.candidates[index].clone().candidate();
        let prior_selected = candidate.selected_senses().to_vec();
        let prior_len = candidate.senses().len();
        let (added, _) = candidate.with_added_senses(correction.senses().to_vec());
        let new_len = added.senses().len();
        appended = new_len - prior_len;
        let mut selection = prior_selected;
        selection.extend(prior_len..new_len);
        record.candidates[index] =
            CandidateRecord::from_candidate(&added.selecting_senses(selection));
        reset_to_understood(record);
        Ok(())
    })?;
    let note = correct_note(&updated, args.card.as_str(), appended);
    conclude(render, &updated, note)
}

/// The confirmation line for a `correct`: the added sense quoted when exactly
/// one landed, a count otherwise.
fn correct_note(record: &SessionRecord, card: &str, appended: usize) -> String {
    match appended {
        0 => format!("Gemini added no new sense to {card} — it was already covered."),
        1 => {
            let understanding = candidate_of(record, card)
                .and_then(|candidate| {
                    candidate
                        .senses()
                        .last()
                        .map(|sense| sense.understanding().to_string())
                })
                .unwrap_or_default();
            format!("Gemini added a sense to {card} — \"{understanding}\". Back in the plan.")
        }
        count => format!("Gemini added {count} senses to {card}. Back in the plan."),
    }
}
