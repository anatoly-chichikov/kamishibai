//! The generation verbs: `generate` (commit the plan and run the worker) and
//! `regenerate` (drop cached artifacts so the next generate rebuilds them; with
//! `--note`, Gemini first rewrites the card from the instruction).
//!
//! `run_session` is the shared commit-and-run step `new --generate` reuses.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::card_workflow::CardGeneration;
use crate::cli::console::{self, HumanReporter, JsonReporter, QuietReporter, Reporter, drafts_for};
use crate::cli::error::{json_line, usage};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardCorrection, CardDraft, LanguagePair, WordCandidate};

use super::args::{GenerateArgs, RegenerateArgs};
use super::store::{DraftRecord, SessionRecord, SessionStore};
use super::{
    Render, drop_artifacts, json, preflight_key, refuse_if_live, reset_to_understood, resolve,
    view, worker,
};

/// Commit the curated plan and start the managed worker that generates+publishes.
pub(super) fn generate(args: &GenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    run_session(&store, record.id.as_str(), args.wait, args.quiet, render)
}

/// Commit the plan (deriving it from the curation when none exists) and run the
/// worker, foreground with `--wait` or detached otherwise. In JSON mode the one
/// stdout document is the session as the command leaves it: freshly generating
/// for a detached run, terminal after a `--wait` run.
pub(super) fn run_session(
    store: &SessionStore,
    id: &str,
    wait: bool,
    quiet: bool,
    render: Render,
) -> Result<()> {
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
        // Foreground generation runs OCR in-process; the native engine printf's
        // into the libc stdout buffer, which would corrupt the single-document
        // `--json` contract. Mute fd 1 for the run and write the one document we
        // owe stdout to the saved real descriptor. The muted fd is never
        // restored, so the buffered native bytes drain into /dev/null at exit —
        // as does cli.rs's own json error line printed after we return here,
        // which is why an error is emitted to the real stdout below first.
        let stdout = MutedStdout::capture()?;
        let outcome = worker::run_foreground(store, id, reporter(render, quiet));
        if matches!(render, Render::Json) {
            let line = match &outcome {
                Ok(record) => json::session_line(record)?,
                Err(error) => json_line(error),
            };
            stdout.emit(line.as_str())?;
        }
        return outcome.map(|_| ());
    }
    let record = worker::start_background(store, id)?;
    if matches!(render, Render::Json) {
        return json::emit_session(&record);
    }
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
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
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

/// Re-roll committed cards: drop the cached artifacts of every unfinished card
/// (`--failed`) or one card (`--card`), optionally rewriting it from `--note`
/// first, then immediately regenerate and republish the deck. Returns like
/// `generate`: the id for a detached run, the terminal state after `--wait`.
pub(super) fn regenerate(args: &RegenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
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
            if matches!(render, Render::Text) {
                eprintln!("rewrote card '{card}'; regenerating");
            }
        }
        None => {
            let (_record, dropped) = drop_targets(&store, &record, args)?;
            if matches!(render, Render::Text) {
                eprintln!("dropped {dropped} card(s); regenerating");
            }
        }
    }
    run_session(&store, record.id.as_str(), args.wait, args.quiet, render)
}

/// Ask Gemini to rewrite one committed card from a note, drop its artifacts, and
/// swap the rewritten draft into the freshly read plan (a later curation
/// discards the rewrite). Returns the record as updated.
fn rewrite(
    store: &SessionStore,
    record: &SessionRecord,
    card: &str,
    note: &str,
) -> Result<SessionRecord> {
    preflight_key()?;
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
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
    })
}

/// Drop the cached artifacts of the targeted committed cards and reset the
/// session so the next generate rebuilds them. Returns the record as updated
/// plus how many cards were dropped.
fn drop_targets(
    store: &SessionStore,
    record: &SessionRecord,
    args: &RegenerateArgs,
) -> Result<(SessionRecord, usize)> {
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
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
    let updated = store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        reset_to_understood(fresh);
        Ok(())
    })?;
    Ok((updated, targets.len()))
}

fn record_of(draft: &CardDraft) -> DraftRecord {
    DraftRecord {
        term: String::from(draft.term()),
        understanding: String::from(draft.understanding()),
    }
}

fn reporter(render: Render, quiet: bool) -> Box<dyn Reporter> {
    match render {
        Render::Json => Box::new(JsonReporter),
        Render::Text if quiet => Box::new(QuietReporter),
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
