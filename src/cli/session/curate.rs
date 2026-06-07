//! Curation handlers: pick senses, exclude a card, or ask Gemini to add senses.
//!
//! Each edits one candidate by term and resets the session to `understood`,
//! clearing the committed plan so the next `generate` re-derives it. The shared
//! preconditions (`missing`, `refuse_if_live`, `reset_to_understood`,
//! `preflight_key`) live in the parent module.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::cli::console;
use crate::session::{BulkCorrection, CandidateRecord, LanguagePair, WordCandidate};

use super::args::{CardArg, CorrectArgs, SelectArgs};
use super::store::SessionStore;
use super::{missing, preflight_key, refuse_if_live, reset_to_understood};

/// Run one curation edit against a candidate found by term, clearing the
/// committed plan so the next `generate` re-derives it from the new selection.
fn curate(
    id: &str,
    card: &str,
    edit: impl FnOnce(WordCandidate) -> Result<WordCandidate>,
) -> Result<bool> {
    let store = SessionStore::system()?;
    if missing(&store, id).is_some() {
        return Ok(false);
    }
    let mut record = store.open(id)?;
    refuse_if_live(&store, &record)?;
    let index = record
        .candidates
        .iter()
        .position(|candidate| candidate.term() == card)
        .ok_or_else(|| anyhow!("no card '{card}' in session '{id}'"))?;
    let updated = edit(record.candidates[index].clone().candidate())?;
    record.candidates[index] = CandidateRecord::from_candidate(&updated);
    reset_to_understood(&mut record);
    record.drafts.clear();
    store.save(&mut record)?;
    Ok(true)
}

/// Pick which 1-based senses of a card become cards, re-including it.
pub(super) fn select(args: &SelectArgs) -> Result<u8> {
    if args.sense.is_empty() {
        bail!("select needs --sense <N[,N...]> (1-based sense numbers)");
    }
    let senses = args.sense.clone();
    let done = curate(args.id.as_str(), args.card.as_str(), |candidate| {
        let count = candidate.senses().len();
        let mut zero = Vec::with_capacity(senses.len());
        for number in &senses {
            if *number < 1 || *number > count {
                bail!(
                    "sense {number} out of range (1..={count}) for '{}'",
                    candidate.term()
                );
            }
            zero.push(number - 1);
        }
        Ok(candidate.selecting_senses(zero).with_ok(true))
    })?;
    if !done {
        return Ok(3);
    }
    eprintln!(
        "selected sense(s) {} of '{}'",
        senses
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(","),
        args.card
    );
    println!("{}", args.id);
    Ok(0)
}

/// Drop one card from the plan while keeping it visible in the understanding.
pub(super) fn exclude(args: &CardArg) -> Result<u8> {
    let done = curate(args.id.as_str(), args.card.as_str(), |candidate| {
        Ok(candidate.with_ok(false))
    })?;
    if !done {
        return Ok(3);
    }
    eprintln!("excluded '{}'", args.card);
    println!("{}", args.id);
    Ok(0)
}

/// Ask Gemini to add senses to one card from a note, keeping the prior selection.
pub(super) fn correct(args: &CorrectArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let mut record = store.open(args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    preflight_key()?;
    let index = record
        .candidates
        .iter()
        .position(|candidate| candidate.term() == args.card.as_str())
        .ok_or_else(|| anyhow!("no card '{}' in session '{}'", args.card, args.id))?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let candidate = record.candidates[index].clone().candidate();
    let prior_selected = candidate.selected_senses().to_vec();
    let prior_len = candidate.senses().len();
    let generator = console::generator(PathBuf::from(record.out.clone()))?;
    let correction = generator.correct_bulk(&candidate, args.note.as_str(), &pair)?;
    let (added, _) = candidate.with_added_senses(correction.senses().to_vec());
    let new_len = added.senses().len();
    let appended = new_len - prior_len;
    let mut selection = prior_selected;
    selection.extend(prior_len..new_len);
    record.candidates[index] = CandidateRecord::from_candidate(&added.selecting_senses(selection));
    reset_to_understood(&mut record);
    record.drafts.clear();
    store.save(&mut record)?;
    eprintln!("added {appended} sense(s) to '{}'", args.card);
    println!("{}", args.id);
    Ok(0)
}
