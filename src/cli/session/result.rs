//! The read-only inspection verbs: `status` (phase + per-card progress), `result`
//! (published paths and card bodies), and `ls` (one line per session).
//!
//! None call Gemini; all project from the session record and the shared cache.

use anyhow::Result;

use crate::cli::error::not_ready;
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardMeta, CardMetaCache, LanguagePair};

use super::args::{LsArgs, ResultArgs, StatusArgs};
use super::store::{DraftRecord, Phase, SessionRecord, SessionStore};
use super::{Render, json, resolve, view};

/// Print a session's phase and per-card progress; `-q` prints just the phase.
pub(super) fn status(args: &StatusArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    if matches!(render, Render::Json) {
        return json::emit_session(&record);
    }
    let root = cache_root(&SystemContext)?;
    if args.quiet {
        println!("{}", view::phase_word(&record, root.as_path()));
    } else {
        println!("{}", view::render_status(&record, root.as_path()));
    }
    Ok(())
}

/// Print a published (or partial) session's deck/pdf/dir paths and card bodies.
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
        return Err(not_ready(format!(
            "session '{}' not ready (phase {})",
            record.id,
            view::phase_label(phase)
        )));
    };
    if matches!(render, Render::Json) {
        return json::emit(&json::ResultDoc::of(
            &record,
            root.as_path(),
            phase,
            &paths,
        )?);
    }
    if args.deck {
        println!("{}", paths.deck);
        return Ok(());
    }
    if args.pdf {
        println!("{}", paths.report);
        return Ok(());
    }
    if args.dir {
        println!("{}", paths.output);
        return Ok(());
    }
    if args.quiet {
        println!("{}", paths.deck);
        println!("{}", paths.report);
        println!("{}", paths.output);
        return Ok(());
    }
    println!("session  {}", record.id);
    println!("pair     {} → {}", record.known, record.learning);
    println!("deck     {}", paths.deck);
    println!("pdf      {}", paths.report);
    println!("dir      {}", paths.output);
    match paths.failed {
        0 if paths.cards > 0 => println!("cards    {} in deck", paths.cards),
        0 => {}
        failed => println!("cards    {} in deck · {failed} failed", paths.cards),
    }
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let cache = CardMetaCache::new(root);
    let total = record.drafts.len();
    for (index, draft) in record.drafts.iter().enumerate() {
        if let Some(meta) = cache.load(draft.term.as_str(), draft.understanding.as_str(), &pair)? {
            println!();
            print_card(index + 1, total, draft, &record, &meta);
        }
    }
    Ok(())
}

/// List every session, one compact line each; `-q` prints just the ids; JSON
/// mode prints one document whose `sessions` array may be empty.
pub(super) fn ls(args: &LsArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let root = cache_root(&SystemContext)?;
    let sessions = store.list()?;
    if matches!(render, Render::Json) {
        return json::emit(&json::LsDoc::of(&sessions, root.as_path()));
    }
    if sessions.is_empty() {
        eprintln!("no sessions");
        return Ok(());
    }
    for record in &sessions {
        if args.quiet {
            println!("{}", record.id);
        } else {
            println!("{}", view::summary_line(record, root.as_path()));
        }
    }
    Ok(())
}

/// Print one resolved card as compact aligned plain text.
fn print_card(
    number: usize,
    total: usize,
    draft: &DraftRecord,
    record: &SessionRecord,
    meta: &CardMeta,
) {
    println!(
        "card {number}/{total}  {}   importance {}",
        draft.term,
        meta.importance()
    );
    println!("  {:<8} {}", "meaning", meta.meaning());
    println!("  {:<8} {}", "say", meta.pronunciation());
    println!("  {:<8} {}", record.learning, meta.target_sentence());
    println!(
        "  {:<8} {}",
        record.known,
        highlighted(meta.source_sentence(), meta.source_highlight())
    );
    println!("  {:<8} {}", "hint", meta.source_hint());
}

fn highlighted(sentence: &str, highlight: &str) -> String {
    if highlight.is_empty() {
        return String::from(sentence);
    }
    sentence.replacen(highlight, &format!("«{highlight}»"), 1)
}
