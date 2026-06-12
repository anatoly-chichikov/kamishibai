//! The session-creation verbs: `new` (understand `--word`s or import a cards
//! JSON) and `open` (resume an existing session in the interactive TUI).
//!
//! Both produce or reopen an `understood` session; the shared preconditions and
//! the generation verbs live in the parent module and `generate`.

use std::fmt::Write;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::cli::batch::StartupCards;
use crate::cli::card_workflow::CardGeneration;
use crate::cli::console::{self, SensePolicy};
use crate::cli::error::usage;
use crate::cli::live_generator::default_output;
use crate::cli::terminal::run_tui;
use crate::config::default_store;
use crate::runtime::locations::SystemContext;
use crate::session::{
    CandidateRecord, LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate,
};
use crate::vocabulary::VocabularyDocument;

use super::args::{IdArg, NewArgs};
use super::generate::run_session;
use super::store::{SessionRecord, SessionStore, mint_id, now, valid_id};
use super::{TuiSession, bridge, open_checked, refuse_if_live};

/// Understand the requested words (or import a cards JSON) and create a session.
pub(super) fn new(args: &NewArgs) -> Result<()> {
    let store = SessionStore::system()?;
    let support = support_language(args.from.as_deref());
    let out = output_dir(args.out.as_deref())?;
    let generator = console::generator(out.clone())?;
    let session = match args.build.as_deref() {
        Some(path) => build_session(&generator, path)?,
        None => word_session(&generator, args, support.as_str())?,
    };
    if session.candidates.is_empty() {
        return Err(usage("nothing understood: no usable words"));
    }
    let id = match args.id.as_deref() {
        Some(name) if valid_id(name) => String::from(name),
        Some(name) => {
            return Err(usage(format!(
                "invalid --id '{name}': use letters, digits, '-', '_' or '.'"
            )));
        }
        None => mint_id(session.pair.target())?,
    };
    if store.exists(id.as_str()) {
        return Err(usage(format!(
            "session '{id}' already exists; pick another --id or remove it first"
        )));
    }
    let record = SessionRecord::understood(
        id,
        now()?,
        String::from(session.pair.support()),
        String::from(session.pair.target()),
        out.to_string_lossy().into_owned(),
        String::from(senses_label(args.senses)),
        String::from(session.source),
        session.words,
        session.candidates,
    );
    store.create(&record)?;
    if args.generate {
        return run_session(&store, record.id.as_str(), false, args.quiet);
    }
    if !args.quiet {
        eprint!("{}", session.preview);
        eprintln!(
            "session {} · senses={} · generate: kamishibai generate {}",
            record.id, record.senses, record.id
        );
    }
    println!("{}", record.id);
    Ok(())
}

/// Reopen an existing session in the interactive TUI, resuming from the cache.
pub(super) fn open(args: &IdArg) -> Result<()> {
    let store = SessionStore::system()?;
    let record = open_checked(&store, args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    let resume = TuiSession::resuming(&record)?;
    let (app, startup) = bridge::record_to_app(&record);
    run_tui(app, startup, Some(resume))
}

/// The parts of a freshly understood/imported session shared by `new`.
struct Prepared {
    pair: LanguagePair,
    words: Vec<String>,
    candidates: Vec<CandidateRecord>,
    source: &'static str,
    preview: String,
}

fn word_session(generator: &impl Understanding, args: &NewArgs, support: &str) -> Result<Prepared> {
    let words = words_lines(args)?;
    let raw = RawInputBatch::new(words.join("\n"));
    if !raw.has_content() {
        return Err(usage("no words to learn: input was empty"));
    }
    let understood = generator.understand(&raw, support)?;
    let target = args
        .to
        .clone()
        .unwrap_or_else(|| understood.guess().code().to_string());
    let pair = LanguagePair::new(target.as_str(), support);
    let candidates = understood
        .candidates()
        .iter()
        .map(|candidate| {
            CandidateRecord::from_candidate(&initial_selection(candidate, args.senses))
        })
        .collect();
    let preview = preview_words(&understood, support, target.as_str(), args.senses);
    Ok(Prepared {
        pair,
        words,
        candidates,
        source: "words",
        preview,
    })
}

/// Apply the `--senses` policy as the candidate's initial sense selection so
/// later `select` curation always overrides a concrete starting point.
fn initial_selection(candidate: &WordCandidate, senses: SensePolicy) -> WordCandidate {
    match senses {
        SensePolicy::Primary => candidate.clone(),
        SensePolicy::All => candidate
            .clone()
            .selecting_senses((0..candidate.senses().len()).collect()),
    }
}

fn build_session(generator: &impl CardGeneration, path: &Path) -> Result<Prepared> {
    let document = read_document(path.to_string_lossy().as_ref())?;
    let (_, drafts) = StartupCards::from_document(&document)?.into_parts();
    let pair = drafts
        .first()
        .map(|draft| draft.pair().clone())
        .ok_or_else(|| usage("the cards JSON contains no entries"))?;
    let mut candidates = Vec::with_capacity(drafts.len());
    let mut words = Vec::with_capacity(drafts.len());
    let mut preview = String::new();
    let _ = writeln!(
        preview,
        "imported {} card(s) · {} → {}",
        drafts.len(),
        pair.support(),
        pair.target()
    );
    for draft in &drafts {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("internal: an imported card has no meta"))?;
        generator.store_card_meta(draft.term(), draft.understanding(), draft.pair(), meta)?;
        words.push(String::from(draft.term()));
        candidates.push(CandidateRecord::from_candidate(&WordCandidate::new(
            draft.term(),
            draft.understanding(),
            true,
        )));
        let _ = writeln!(
            preview,
            "{}\n  card    {}",
            draft.term(),
            draft.understanding()
        );
    }
    Ok(Prepared {
        pair,
        words,
        candidates,
        source: "cards",
        preview,
    })
}

fn preview_words(
    understood: &Understood,
    support: &str,
    target: &str,
    senses: SensePolicy,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "understanding {} word(s) · {support} → {target}",
        understood.candidates().len()
    );
    for candidate in understood.candidates() {
        let _ = writeln!(out, "{}", candidate.term());
        for (index, sense) in candidate.senses().iter().enumerate() {
            let kept = candidate.ok()
                && (matches!(senses, SensePolicy::All)
                    || candidate.selected_senses().contains(&index));
            let status = if kept { "card" } else { "skip" };
            let _ = writeln!(
                out,
                "  {status}  {:<7} {}",
                sense.tag().unwrap_or(""),
                sense.understanding()
            );
        }
    }
    out
}

fn words_lines(args: &NewArgs) -> Result<Vec<String>> {
    if !args.word.is_empty() {
        return Ok(args.word.clone());
    }
    match &args.words {
        Some(source) => Ok(read_input(source.as_str()).map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect()
        })?),
        None => Err(usage(
            "no words: pass --word <WORD> (repeatable) or --words FILE",
        )),
    }
}

fn support_language(explicit: Option<&str>) -> String {
    if let Some(code) = explicit {
        return String::from(code);
    }
    default_store(&SystemContext)
        .ok()
        .and_then(|store| store.read().ok())
        .map(|prefs| prefs.startup_language().to_string())
        .unwrap_or_else(|| String::from("en"))
}

fn output_dir(out: Option<&Path>) -> Result<PathBuf> {
    match out {
        Some(dir) => Ok(dir.to_path_buf()),
        None => default_output(),
    }
}

fn read_input(source: &str) -> Result<String> {
    if source == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("reading input from stdin")?;
        return Ok(buffer);
    }
    fs::read_to_string(source).with_context(|| format!("reading input from '{source}'"))
}

fn read_document(source: &str) -> Result<VocabularyDocument> {
    if source == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("reading cards JSON from stdin")?;
        return VocabularyDocument::parse(buffer.as_str());
    }
    VocabularyDocument::load(source)
}

fn senses_label(policy: SensePolicy) -> &'static str {
    match policy {
        SensePolicy::Primary => "primary",
        SensePolicy::All => "all",
    }
}
