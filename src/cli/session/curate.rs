//! Curation handlers: pick senses, exclude a card, or ask Gemini to add senses.
//!
//! Each edits one candidate by term and resets the session to `understood`,
//! clearing the committed plan so the next `generate` re-derives it. Every edit
//! runs inside one `store.update` closure, so concurrent curation commands
//! serialize and all apply. The shared preconditions (`resolve`,
//! `refuse_if_live`, `reset_to_understood`, `preflight_key`) live in the parent
//! module.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::console;
use crate::cli::error::usage;
use crate::session::{BulkCorrection, CandidateRecord, LanguagePair, WordCandidate};

use super::args::{CardArg, CorrectArgs, SelectArgs};
use super::store::{SessionRecord, SessionStore};
use super::{Render, json, preflight_key, refuse_if_live, reset_to_understood, resolve};

/// Run one curation edit against a candidate found by term, clearing the
/// committed plan so the next `generate` re-derives it from the new selection.
/// Returns the record as updated, for the JSON document.
fn curate(
    store: &SessionStore,
    id: &str,
    card: &str,
    edit: impl FnOnce(WordCandidate) -> Result<WordCandidate>,
) -> Result<SessionRecord> {
    store.update(id, |record| {
        refuse_if_live(store, record)?;
        let index = record
            .candidates
            .iter()
            .position(|candidate| candidate.term() == card)
            .ok_or_else(|| usage(format!("no card '{card}' in session '{id}'")))?;
        let updated = edit(record.candidates[index].clone().candidate())?;
        record.candidates[index] = CandidateRecord::from_candidate(&updated);
        reset_to_understood(record);
        record.drafts.clear();
        Ok(())
    })
}

/// Print the post-mutation state: the session document in JSON mode, or the
/// verb's confirmation note plus the capturable id in text mode.
fn conclude(render: Render, record: &SessionRecord, note: impl FnOnce()) -> Result<()> {
    if matches!(render, Render::Json) {
        return json::emit_session(record);
    }
    note();
    println!("{}", record.id);
    Ok(())
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
    conclude(render, &updated, || {
        eprintln!(
            "selected sense(s) {} of '{}'",
            args.sense
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
            args.card
        );
    })
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
    conclude(render, &updated, || {
        eprintln!("excluded '{}'", args.card);
    })
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
        record.drafts.clear();
        Ok(())
    })?;
    conclude(render, &updated, || {
        eprintln!("added {appended} sense(s) to '{}'", args.card);
    })
}
