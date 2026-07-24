//! The session-creation verb: `new` (understand `--word`s or import a cards
//! JSON), producing an `understood` session; the shared preconditions and the
//! generation verbs live in the parent module and `generate`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::cli::card_workflow::CardGeneration;
use crate::cli::console::{self, SensePolicy};
use crate::cli::error::{usage, usage_hint};
use crate::cli::gemini_workflow::default_output;
use crate::config::{PreferenceStore, default_store};
use crate::runtime::locations::SystemContext;
use crate::session::{
    CandidateRecord, LanguagePair, RawInputBatch, Understanding, WordCandidate,
    drafts_from_document,
};
use crate::vocabulary::VocabularyDocument;

use super::args::NewArgs;
use super::generate::run_session;
use super::store::{SessionRecord, SessionStore, mint_id, now, valid_id};
use super::{Render, json, preflight_key, validate_language, view};

/// Understand the requested words (or import a cards JSON) and create a session.
pub(super) fn new(args: &NewArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let out = output_dir(args.out.as_deref())?;
    let generator = console::generator(out.clone())?;
    let session = match args.build.as_deref() {
        Some(path) => build_session(&generator, path)?,
        None => {
            let prefs_store = default_store(&SystemContext)?;
            let known = resolve_known(args.known.as_deref(), &prefs_store)?;
            preflight_key()?;
            word_session(&generator, args, known.as_str())?
        }
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
        None => mint_id(session.pair.learning())?,
    };
    if store.exists(id.as_str()) {
        return Err(usage(format!(
            "session '{id}' already exists; pick another --id or remove it first"
        )));
    }
    let record = SessionRecord::understood(
        id,
        now()?,
        String::from(session.pair.known()),
        String::from(session.pair.learning()),
        out.to_string_lossy().into_owned(),
        String::from(senses_label(args.senses)),
        String::from(session.source),
        session.words,
        session.candidates,
    );
    store.create(&record)?;
    if args.generate {
        return run_session(&store, record.id.as_str(), false, render, None);
    }
    if matches!(render, Render::Json) {
        return json::emit_session(&record);
    }
    println!("{}", view::render_understood(&record));
    Ok(())
}

/// The parts of a freshly understood/imported session shared by `new`.
struct Prepared {
    pair: LanguagePair,
    words: Vec<String>,
    candidates: Vec<CandidateRecord>,
    source: &'static str,
}

fn word_session(generator: &impl Understanding, args: &NewArgs, known: &str) -> Result<Prepared> {
    let words = words_lines(args)?;
    let raw = RawInputBatch::new(words.join("\n"));
    if !raw.has_content() {
        return Err(usage("no words to learn: input was empty"));
    }
    let understood = generator.understand(&raw, known)?;
    let learning = args
        .learning
        .clone()
        .unwrap_or_else(|| understood.guess().code().to_string())
        .to_uppercase();
    let pair = LanguagePair::new(learning.as_str(), known.to_uppercase());
    let candidates = understood
        .candidates()
        .iter()
        .map(|candidate| {
            CandidateRecord::from_candidate(&initial_selection(candidate, args.senses))
        })
        .collect();
    Ok(Prepared {
        pair,
        words,
        candidates,
        source: "words",
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
    let (pair, drafts) = drafts_from_document(&document)?;
    let mut candidates = Vec::with_capacity(drafts.len());
    let mut words = Vec::with_capacity(drafts.len());
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
    }
    Ok(Prepared {
        pair,
        words,
        candidates,
        source: "cards",
    })
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

/// Resolve the known (native) language for a word session: an explicit
/// `--known` (validated, used once and never persisted), else the saved
/// preference, else a guided refusal when the user never set one.
fn resolve_known(explicit: Option<&str>, store: &PreferenceStore) -> Result<String> {
    if let Some(code) = explicit {
        validate_language(code)?;
        return Ok(String::from(code));
    }
    let prefs = store.read().unwrap_or_default();
    if prefs.requires_language_choice() {
        return Err(usage_hint(
            "no known language set — can't pick which sense of each word you need",
            "Set it once: kamishibai config --known en (or add --known en just this run)",
        ));
    }
    Ok(prefs.startup_language().to_string())
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
