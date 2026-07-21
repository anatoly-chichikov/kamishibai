//! The generation verbs: `generate` (commit the plan and run the worker) and
//! `regenerate` (retry only missing stages with `--failed`, or fully re-roll one
//! card with `--card`; with `--note`, Gemini first rewrites that card).
//!
//! `run_session` is the shared commit-and-run step `new --generate` reuses.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::card_workflow::CardGeneration;
use crate::cli::console::{self, HumanReporter, JsonReporter, Reporter, drafts_for};
use crate::cli::error::{json_line, operational_hint, usage, usage_hint};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{CardCorrection, CardDraft, LanguagePair, WordCandidate};

use super::args::{GenerateArgs, RegenerateArgs};
use super::store::{DraftRecord, Phase, SessionRecord, SessionStore};
use super::{
    Render, drop_artifacts, drop_incomplete_artifacts, json, preflight_key, refuse_if_live,
    reset_to_understood, resolve, view, worker,
};

/// Commit the curated plan and start the managed worker that generates+publishes.
pub(super) fn generate(args: &GenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    run_session(&store, record.id.as_str(), args.wait, render, None)
}

/// Commit the plan (deriving it from the curation when none exists) and run the
/// worker, foreground with `--wait` or detached otherwise. `intro` is an
/// optional note (regenerate's "re-rolling …") printed under the header in plain
/// mode. In JSON mode the one stdout document is the session as the command
/// leaves it: freshly generating for a detached run, terminal after `--wait`.
pub(super) fn run_session(
    store: &SessionStore,
    id: &str,
    wait: bool,
    render: Render,
    intro: Option<String>,
) -> Result<()> {
    preflight_key()?;
    store.update(id, |record| {
        refuse_if_live(store, record)?;
        ensure_plan(record);
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
        Err(_) => {
            let reshaped = operational_hint(
                "couldn't build any card — nothing published",
                format!("Check your connection and key, then: kamishibai generate {id}"),
            );
            if matches!(render, Render::Json) {
                stdout.emit(json_line(&reshaped).as_str())?;
            }
            Err(reshaped)
        }
    }
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

/// Retry unfinished committed cards from their first missing stage (`--failed`)
/// or fully re-roll one card (`--card`), optionally rewriting it from `--note`
/// first, then immediately regenerate and republish the deck. Returns like
/// `generate`: the id for a detached run, the terminal state after `--wait`.
pub(super) fn regenerate(args: &RegenerateArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    refuse_if_live(&store, &record)?;
    if record.drafts.is_empty() {
        return Err(usage_hint(
            "no committed plan to regenerate",
            "Generate it first: kamishibai generate",
        ));
    }
    let intro = match args.note.as_deref() {
        Some(note) => {
            let card = args
                .card
                .as_deref()
                .expect("invariant: clap requires --card with --note");
            rewrite(&store, &record, card, note)?;
            rewrite_note(card, &record)
        }
        None => {
            let (_record, targets) = drop_targets(&store, &record, args)?;
            reroll_note(&targets, &record, args.failed)
        }
    };
    run_session(&store, record.id.as_str(), args.wait, render, Some(intro))
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

/// Drop only missing stages for `--failed`, or every generated artifact for an
/// explicit `--card`, and reset the session so generation resumes immediately.
/// Returns the updated record plus the terms of the targeted cards.
fn drop_targets(
    store: &SessionStore,
    record: &SessionRecord,
    args: &RegenerateArgs,
) -> Result<(SessionRecord, Vec<String>)> {
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
        match args.card {
            Some(_) => drop_artifacts(
                root.as_path(),
                &pair,
                draft.term.as_str(),
                draft.understanding.as_str(),
                keep_meta,
            )?,
            None => drop_incomplete_artifacts(
                root.as_path(),
                &pair,
                draft.term.as_str(),
                draft.understanding.as_str(),
            )?,
        }
    }
    let updated = store.update(record.id.as_str(), |fresh| {
        refuse_if_live(store, fresh)?;
        reset_to_understood(fresh);
        Ok(())
    })?;
    let terms = targets.iter().map(|draft| draft.term.clone()).collect();
    Ok((updated, terms))
}

fn record_of(draft: &CardDraft) -> DraftRecord {
    DraftRecord {
        term: String::from(draft.term()),
        understanding: String::from(draft.understanding()),
    }
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
