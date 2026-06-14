//! The read-only inspection verbs: `status` (phase + per-card progress), `result`
//! (published paths and card bodies), and `ls` (one line per session).
//!
//! None call Gemini; all project from the session record and the shared cache.

use std::path::Path;

use anyhow::Result;

use crate::cli::error::not_ready_hint;
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardMeta, CardMetaCache, LanguagePair};

use super::args::{LsArgs, ResultArgs, StatusArgs};
use super::store::{DraftRecord, Phase, SessionRecord, SessionStore};
use super::{Render, json, resolve, view};

/// Print a session's phase and per-card progress (or the understood block).
pub(super) fn status(args: &StatusArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    if matches!(render, Render::Json) {
        return json::emit_session(&record);
    }
    let root = cache_root(&SystemContext)?;
    println!("{}", view::render_status(&record, root.as_path()));
    Ok(())
}

/// Print a published (or partial) session's paths, card cache, and card bodies.
pub(super) fn result(args: &ResultArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    let root = cache_root(&SystemContext)?;
    let (phase, _, _) = view::live_phase(&record, root.as_path());
    let Some(paths) = record
        .result
        .clone()
        .filter(|_| matches!(phase, Phase::Published | Phase::Partial))
    else {
        return Err(not_ready_hint(
            no_deck_message(phase),
            no_deck_hint(phase, record.id.as_str()),
        ));
    };
    if matches!(render, Render::Json) {
        return json::emit(&json::ResultDoc::of(
            &record,
            root.as_path(),
            phase,
            &paths,
        )?);
    }
    let cards = view::cards(&record, root.as_path());
    let ready = cards.iter().filter(|card| card.ready()).count();
    let total = cards.len();
    println!("{}", view::header(&record, phase));
    println!(
        "{}",
        view::committed_summary(phase, total, ready, total - ready, None)
    );
    println!("deck: {}", paths.deck);
    println!("pdf: {}", paths.report);
    println!("dir: {}", paths.output);
    println!("cache: {}", cards_cache(&record, root.as_path()).display());
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let cache = CardMetaCache::new(root);
    for draft in &record.drafts {
        if let Some(meta) = cache.load(draft.term.as_str(), draft.understanding.as_str(), &pair)? {
            print_card(draft, &record, &meta);
        }
    }
    if let Some(next) = view::next_step(phase) {
        println!("{next}");
    }
    Ok(())
}

/// List every session, one compact line each; JSON mode prints one document
/// whose `sessions` array may be empty.
pub(super) fn ls(_args: &LsArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let root = cache_root(&SystemContext)?;
    let sessions = store.list()?;
    if matches!(render, Render::Json) {
        return json::emit(&json::LsDoc::of(&sessions, root.as_path()));
    }
    if sessions.is_empty() {
        println!("no sessions yet — create one: kamishibai new --word <WORD>");
        return Ok(());
    }
    for record in &sessions {
        println!("{}", view::summary_line(record, root.as_path()));
    }
    Ok(())
}

/// The plain-language problem line when `result` finds no deck.
fn no_deck_message(phase: Phase) -> &'static str {
    match phase {
        Phase::Understood => "no deck — the session is still understood",
        Phase::Generating => "no deck — still generating",
        Phase::Failed => "no deck — every card failed",
        Phase::Cancelled => "no deck — the run was cancelled",
        Phase::Interrupted => "no deck — the worker stopped before publishing",
        Phase::Published | Phase::Partial => "no deck yet",
    }
}

/// The next-step hint when `result` finds no deck.
fn no_deck_hint(phase: Phase, id: &str) -> String {
    match phase {
        Phase::Failed | Phase::Partial => {
            format!("Try again: kamishibai regenerate {id} --failed")
        }
        Phase::Generating => format!("Watch it: kamishibai status {id}"),
        Phase::Cancelled | Phase::Interrupted => format!("Resume it: kamishibai generate {id}"),
        Phase::Understood | Phase::Published => {
            format!("Generate it first: kamishibai generate {id}")
        }
    }
}

/// The folder holding this session's per-card cached assets.
fn cards_cache(record: &SessionRecord, root: &Path) -> std::path::PathBuf {
    root.join("cards").join(format!(
        "{}-{}",
        record.known.to_uppercase(),
        record.learning.to_uppercase()
    ))
}

/// Print one resolved card as an indented block under the paths.
fn print_card(draft: &DraftRecord, record: &SessionRecord, meta: &CardMeta) {
    println!("  {} · importance {}", draft.term, meta.importance());
    println!("    meaning: {}", meta.meaning());
    println!("    say: {}", meta.transcription());
    println!(
        "    {}: {}",
        record.learning.to_uppercase(),
        meta.target_sentence()
    );
    println!(
        "    {}: {}",
        record.known.to_uppercase(),
        highlighted(meta.source_sentence(), meta.source_highlight())
    );
    println!("    hint: {}", meta.source_hint());
}

fn highlighted(sentence: &str, highlight: &str) -> String {
    if highlight.is_empty() {
        return String::from(sentence);
    }
    sentence.replacen(highlight, &format!("«{highlight}»"), 1)
}
