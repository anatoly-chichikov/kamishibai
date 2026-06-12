//! The generation verbs: `generate` (commit the plan and run the worker) and
//! `regenerate` (drop cached artifacts so the next generate rebuilds them; with
//! `--note`, Gemini first rewrites the card from the instruction).
//!
//! `run_session` is the shared commit-and-run step `new --generate` reuses.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::card_workflow::CardGeneration;
use crate::cli::console::{self, HumanReporter, QuietReporter, Reporter, drafts_for};
use crate::cli::error::usage;
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardCorrection, CardDraft, LanguagePair, WordCandidate};

use super::args::{GenerateArgs, RegenerateArgs};
use super::store::{DraftRecord, SessionRecord, SessionStore};
use super::{
    drop_artifacts, open_checked, preflight_key, refuse_if_live, reset_to_understood, view, worker,
};

/// Commit the curated plan and start the managed worker that generates+publishes.
pub(super) fn generate(args: &GenerateArgs) -> Result<()> {
    let store = SessionStore::system()?;
    open_checked(&store, args.id.as_str())?;
    run_session(&store, args.id.as_str(), args.wait, args.quiet)
}

/// Commit the plan (deriving it from the curation when none exists) and run the
/// worker, foreground with `--wait` or detached otherwise.
pub(super) fn run_session(store: &SessionStore, id: &str, wait: bool, quiet: bool) -> Result<()> {
    preflight_key()?;
    store.update(id, |record| {
        refuse_if_live(store, record)?;
        ensure_plan(record);
        if record.drafts.is_empty() {
            return Err(usage("nothing to generate: select at least one card first"));
        }
        Ok(())
    })?;
    if wait {
        return worker::run_foreground(store, id, reporter(quiet));
    }
    worker::start_background(store, id)?;
    if !quiet {
        eprintln!("started session {id} (background)");
        eprintln!("poll: kamishibai status {id}");
    }
    println!("{id}");
    Ok(())
}

/// Derive the committed plan from the curated candidates when none is committed.
fn ensure_plan(record: &mut SessionRecord) {
    if !record.drafts.is_empty() {
        return;
    }
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let candidates: Vec<WordCandidate> = record
        .candidates
        .iter()
        .map(|stored| stored.clone().candidate())
        .collect();
    record.drafts = drafts_for(&candidates, &pair)
        .iter()
        .map(record_of)
        .collect();
}

/// Drop committed cards' cached artifacts so the next generate rebuilds them:
/// every unfinished card with `--failed`, or one card by `--card`. With `--note`
/// Gemini first rewrites that card from the instruction (a Gemini call).
pub(super) fn regenerate(args: &RegenerateArgs) -> Result<()> {
    let store = SessionStore::system()?;
    let record = open_checked(&store, args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    if record.drafts.is_empty() {
        return Err(usage(
            "no committed plan to regenerate: generate the session first",
        ));
    }
    match args.note.as_deref() {
        Some(note) => {
            let card = args
                .card
                .as_deref()
                .expect("invariant: clap requires --card with --note");
            rewrite(&store, &record, card, note)?;
            eprintln!("rewrote card '{card}'; generate the session to rebuild it");
        }
        None => {
            let dropped = drop_targets(&store, &record, args)?;
            eprintln!("dropped {dropped} card(s); generate the session to rebuild them");
        }
    }
    println!("{}", args.id);
    Ok(())
}

/// Ask Gemini to rewrite one committed card from a note, drop its artifacts, and
/// swap the rewritten draft into the freshly read plan (a later curation
/// discards the rewrite).
fn rewrite(store: &SessionStore, record: &SessionRecord, card: &str, note: &str) -> Result<()> {
    preflight_key()?;
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let current = record
        .drafts
        .iter()
        .find(|draft| draft.term.as_str() == card)
        .ok_or_else(|| usage(format!("no card '{card}' in session '{}'", record.id)))?
        .clone();
    let draft = CardDraft::new(
        current.term.as_str(),
        current.understanding.as_str(),
        pair.clone(),
    );
    let generator = console::generator(PathBuf::from(record.out.clone()))?;
    let revision = generator.correct_card(&draft, note, &pair)?;
    let (term, understanding, meta) = revision.into_parts();
    drop_artifacts(
        root.as_path(),
        &pair,
        current.term.as_str(),
        current.understanding.as_str(),
        false,
    )?;
    generator.store_card_meta(term.as_str(), understanding.as_str(), &pair, &meta)?;
    store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        let index = fresh
            .drafts
            .iter()
            .position(|draft| draft.term.as_str() == current.term.as_str())
            .ok_or_else(|| {
                usage(format!(
                    "no card '{}' in session '{}'",
                    current.term, fresh.id
                ))
            })?;
        fresh.drafts[index] = DraftRecord {
            term,
            understanding,
        };
        reset_to_understood(fresh);
        Ok(())
    })?;
    Ok(())
}

/// Drop the cached artifacts of the targeted committed cards and reset the
/// session so the next generate rebuilds them.
fn drop_targets(
    store: &SessionStore,
    record: &SessionRecord,
    args: &RegenerateArgs,
) -> Result<usize> {
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let targets: Vec<DraftRecord> = match &args.card {
        Some(term) => record
            .drafts
            .iter()
            .filter(|draft| draft.term.as_str() == term.as_str())
            .cloned()
            .collect(),
        None => view::incomplete_drafts(record, root.as_path())
            .into_iter()
            .cloned()
            .collect(),
    };
    if targets.is_empty() {
        return Err(usage("no matching cards to regenerate"));
    }
    let keep_meta = record.source.as_str() == "cards";
    for draft in &targets {
        drop_artifacts(
            root.as_path(),
            &pair,
            draft.term.as_str(),
            draft.understanding.as_str(),
            keep_meta,
        )?;
    }
    store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        reset_to_understood(fresh);
        Ok(())
    })?;
    Ok(targets.len())
}

fn record_of(draft: &CardDraft) -> DraftRecord {
    DraftRecord {
        term: String::from(draft.term()),
        understanding: String::from(draft.understanding()),
    }
}

fn reporter(quiet: bool) -> Box<dyn Reporter> {
    if quiet {
        return Box::new(QuietReporter);
    }
    Box::new(HumanReporter)
}
