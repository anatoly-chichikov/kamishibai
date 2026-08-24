//! The session-creation verb: `new` (understand `--word`s or import a cards
//! JSON), producing an `understood` session; the shared preconditions and the
//! generation verbs live in the parent module and `generate`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::application::{CardProduction, LearningTarget, WordUnderstanding};
use crate::cli::console::{self, SensePolicy};
use crate::cli::error::{usage, usage_hint};
use crate::config::{PreferenceStore, default_store};
use crate::runtime::locations::{LocationArgs, Locations, OutputUnavailable, SystemContext};
use crate::session::{
    CandidateRecord, LanguagePair, MAX_INTAKE_WORDS, MAX_PLAN_CARDS, RawInputBatch,
    SentenceBatchSettings, SentenceLevel, SentenceTypeMix, WordCandidate, drafts_from_document,
};
use crate::vocabulary::VocabularyDocument;

use super::args::{BatchLevel, BatchTypes, NewArgs};
use super::generate::run_session;
use super::store::{SessionRecord, SessionStore, mint_id, now, valid_id};
use super::{Render, json, preflight_key, resolve_language, validate_language, view};

/// Understand the requested words (or import a cards JSON) and create a session.
pub(super) fn new(args: &NewArgs, render: Render) -> Result<()> {
    let target = learning_target(args.learning.as_deref())?;
    let words = match args.build {
        Some(_) => Vec::new(),
        None => intake_words(args)?,
    };
    let out = output_dir(args.out.as_deref())?;
    let store = SessionStore::system()?;
    let workflow = console::workflow(out.clone())?;
    let session = match args.build.as_deref() {
        Some(path) => build_session(&workflow, path)?,
        None => {
            let prefs_store = default_store(&SystemContext)?;
            let known = resolve_known(args.known.as_deref(), &prefs_store)?;
            preflight_key()?;
            word_session(&workflow, args, known.as_str(), &target, words)?
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
    )
    .with_sentences(sentence_settings(args));
    store.create(&record)?;
    if args.generate {
        return run_session(&store, record.id.as_str(), args.wait, render, None, false);
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

fn word_session(
    workflow: &impl WordUnderstanding,
    args: &NewArgs,
    known: &str,
    target: &LearningTarget,
    words: Vec<String>,
) -> Result<Prepared> {
    let raw = RawInputBatch::new(words.join("\n"));
    let understood = workflow.understand(&raw, known, target)?;
    let learning = understood.guess().code().to_uppercase();
    let pair = LanguagePair::new(learning.as_str(), known.to_uppercase());
    let selected = understood
        .candidates()
        .iter()
        .map(|candidate| initial_selection(candidate, args.senses))
        .collect::<Vec<_>>();
    let cards: usize = selected
        .iter()
        .filter(|candidate| candidate.ok())
        .map(WordCandidate::selected_count)
        .sum();
    if cards > MAX_PLAN_CARDS {
        return Err(usage_hint(
            format!(
                "too many cards: {} words with the chosen senses make {cards} cards, at most {MAX_PLAN_CARDS} per batch",
                words.len()
            ),
            format!(
                "Use --senses primary, or pass fewer words, so the plan is {MAX_PLAN_CARDS} cards or fewer"
            ),
        ));
    }
    let candidates = selected
        .iter()
        .map(CandidateRecord::from_candidate)
        .collect();
    Ok(Prepared {
        pair,
        words,
        candidates,
        source: "words",
    })
}

fn learning_target(explicit: Option<&str>) -> Result<LearningTarget> {
    let Some(code) = explicit else {
        return Ok(LearningTarget::Detect);
    };
    resolve_language(code).map(LearningTarget::Explicit)
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

fn build_session(workflow: &impl CardProduction, path: &Path) -> Result<Prepared> {
    let document = read_document(path.to_string_lossy().as_ref())?;
    let (pair, drafts) = drafts_from_document(&document)?;
    if drafts.len() > MAX_PLAN_CARDS {
        return Err(usage_hint(
            format!(
                "too many cards: {} entries, at most {MAX_PLAN_CARDS} per batch",
                drafts.len()
            ),
            format!(
                "Split the cards JSON into files of {MAX_PLAN_CARDS} entries or fewer and run new --build once per file"
            ),
        ));
    }
    let mut candidates = Vec::with_capacity(drafts.len());
    let mut words = Vec::with_capacity(drafts.len());
    for draft in &drafts {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("internal: an imported card has no meta"))?;
        workflow.store_card_meta(draft.term(), draft.understanding(), draft.pair(), meta)?;
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

/// Read the requested vocabulary lines and refuse a batch that is too large
/// for one understanding pass.
///
/// Runs before the preference store, the credential preflight, and any Gemini
/// or cache work, so an oversized list costs nothing.
fn intake_words(args: &NewArgs) -> Result<Vec<String>> {
    let raw = RawInputBatch::new(words_lines(args)?.join("\n"));
    if !raw.has_content() {
        return Err(usage("no words to learn: input was empty"));
    }
    let count = raw.word_count();
    if count > MAX_INTAKE_WORDS {
        return Err(usage_hint(
            format!("too many words: {count} lines, at most {MAX_INTAKE_WORDS} per batch"),
            format!(
                "Split the list into batches of {MAX_INTAKE_WORDS} or fewer and run new once per batch"
            ),
        ));
    }
    Ok(raw.lines().map(String::from).collect())
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
        return Ok(code.to_uppercase());
    }
    let prefs = store.read()?;
    if prefs.requires_language_choice() {
        return Err(usage_hint(
            "no known language set — can't pick which sense of each word you need",
            "Run: kamishibai config --known EN --json (or add --known EN just this run)",
        ));
    }
    Ok(prefs.startup_language().to_uppercase())
}

fn output_dir(out: Option<&Path>) -> Result<PathBuf> {
    let output = Locations::new(
        LocationArgs {
            path: None,
            output: out.map(Path::to_path_buf),
            cache: None,
        },
        SystemContext,
    )
    .output();
    output.map_err(|error| {
        if error.downcast_ref::<OutputUnavailable>().is_some() {
            return usage_hint(
                "default output directory cannot be determined",
                "Pass --out DIR or set KAMISHIBAI_OUTPUT",
            );
        }
        error
    })
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

fn sentence_settings(args: &NewArgs) -> SentenceBatchSettings {
    let level = args.level.map(|level| match level {
        BatchLevel::A1 => SentenceLevel::A1,
        BatchLevel::A2 => SentenceLevel::A2,
        BatchLevel::B1 => SentenceLevel::B1,
        BatchLevel::B2 => SentenceLevel::B2,
        BatchLevel::C1 => SentenceLevel::C1,
        BatchLevel::C2 => SentenceLevel::C2,
    });
    let types = match args.types {
        BatchTypes::BestFit => SentenceTypeMix::BestFit,
        BatchTypes::Statements => SentenceTypeMix::Statements,
        BatchTypes::Questions => SentenceTypeMix::Questions,
        BatchTypes::Dialogue => SentenceTypeMix::Dialogue,
        BatchTypes::Mixed => SentenceTypeMix::Mixed,
    };
    SentenceBatchSettings::new(level, types)
}
