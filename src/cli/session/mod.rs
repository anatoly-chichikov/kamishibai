//! Persistent asynchronous generation sessions — the non-interactive flow.
//!
//! A session is a directory under the cache that an agent drives across separate
//! invocations: `new` (understand the `--word`s) → curate the understanding with
//! `select`/`exclude`/`correct` → `generate` (a managed background worker
//! generates and publishes) → `status`/`result`, with `regenerate`/`fix` to push
//! corrections. Output is plain text only — never JSON. The one machine-relevant
//! value of each command (a session id, a path) is printed bare on stdout so it
//! is captured with `$(...)`; everything else goes to stderr. The clap grammar
//! lives in `args`, the curation handlers in `curate`, and `cli.rs` only routes a
//! parsed `Command`.

mod args;
mod bridge;
mod curate;
mod liveness;
mod store;
mod view;
mod worker;

pub(super) use args::Command;
pub(super) use bridge::TuiSession;

use std::fmt::Write;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::config::default_store;
use crate::generation::artifact_cache::{ILLUSTRATION_FILE, META_FILE, SCENE_FILE, VOICE_FILE};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{
    CandidateRecord, CardCell, CardCorrection, CardDraft, CardMeta, CardMetaCache, LanguagePair,
    RawInputBatch, Understanding, Understood, WordCandidate,
};
use crate::vocabulary::VocabularyDocument;

use super::batch::StartupCards;
use super::card_workflow::CardGeneration;
use super::console::{self, HumanReporter, QuietReporter, Reporter, SensePolicy, drafts_for};
use super::live_generator::default_output;
use args::{
    FixArgs, GenerateArgs, IdArg, LsArgs, NewArgs, RegenerateArgs, ResultArgs, RmArgs, StatusArgs,
};

use store::{DraftRecord, Phase, SessionRecord, SessionStore, mint_id, now, valid_id};

/// Route one parsed command to its handler, returning a process exit code.
pub(super) fn handle(command: &Command) -> Result<u8> {
    match command {
        Command::New(args) => new(args),
        Command::Open(args) => open(args),
        Command::Select(args) => curate::select(args),
        Command::Exclude(args) => curate::exclude(args),
        Command::Correct(args) => curate::correct(args),
        Command::Generate(args) => generate(args),
        Command::Status(args) => status(args),
        Command::Regenerate(args) => regenerate(args),
        Command::Fix(args) => fix(args),
        Command::Result(args) => result(args),
        Command::Cancel(args) => cancel(args),
        Command::Ls(args) => ls(args),
        Command::Rm(args) => rm(args),
        Command::CachePath => cache_path(),
        Command::Worker(args) => {
            worker::run_detached_entry(args.id.as_str())?;
            Ok(0)
        }
    }
}

fn new(args: &NewArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    let support = support_language(args.from.as_deref());
    let out = output_dir(args.out.as_deref())?;
    let generator = console::generator(out.clone())?;
    let session = match args.build.as_deref() {
        Some(path) => build_session(&generator, path)?,
        None => word_session(&generator, args, support.as_str())?,
    };
    if session.candidates.is_empty() {
        bail!("nothing understood: no usable words");
    }
    let id = match args.id.as_deref() {
        Some(name) if valid_id(name) => String::from(name),
        Some(name) => bail!("invalid --id '{name}': use letters, digits, '-', '_' or '.'"),
        None => mint_id(session.pair.target())?,
    };
    if store.exists(id.as_str()) {
        bail!("session '{id}' already exists; pick another --id or remove it first");
    }
    let mut record = SessionRecord::understood(
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
    store.save(&mut record)?;
    if args.generate {
        return run_session(&store, record, false, args.quiet);
    }
    if !args.quiet {
        eprint!("{}", session.preview);
        eprintln!(
            "session {} · senses={} · generate: kamishibai generate {}",
            record.id, record.senses, record.id
        );
    }
    println!("{}", record.id);
    Ok(0)
}

fn open(args: &IdArg) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let record = store.open(args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    let resume = TuiSession::resuming(
        record.id.clone(),
        record.created.clone(),
        record.source.clone(),
        record.senses.clone(),
        record.rev,
    )?;
    let (app, startup) = bridge::record_to_app(&record);
    super::terminal::run_tui(app, startup, Some(resume))?;
    Ok(0)
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
        bail!("no words to learn: input was empty");
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
        .ok_or_else(|| anyhow!("the cards JSON contains no entries"))?;
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

fn generate(args: &GenerateArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let record = store.open(args.id.as_str())?;
    run_session(&store, record, args.wait, args.quiet)
}

fn run_session(
    store: &SessionStore,
    mut record: SessionRecord,
    wait: bool,
    quiet: bool,
) -> Result<u8> {
    refuse_if_live(store, &record)?;
    preflight_key()?;
    ensure_plan(&mut record);
    if record.drafts.is_empty() {
        bail!("nothing to generate: select at least one card first");
    }
    store.save(&mut record)?;
    if wait {
        worker::run_foreground(store, record.id.as_str(), reporter(quiet))?;
        return Ok(0);
    }
    let id = record.id.clone();
    worker::start_background(store, record)?;
    if !quiet {
        eprintln!("started session {id} (background)");
        eprintln!("poll: kamishibai status {id}");
    }
    println!("{id}");
    Ok(0)
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
    record.drafts = drafts_for(&candidates, &pair, SensePolicy::Primary)
        .iter()
        .map(record_of)
        .collect();
}

fn status(args: &StatusArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let record = store.open(args.id.as_str())?;
    let root = cache_root(&SystemContext)?;
    if args.quiet {
        println!("{}", view::phase_word(&record, root.as_path()));
    } else {
        println!("{}", view::render_status(&record, root.as_path()));
    }
    Ok(0)
}

fn result(args: &ResultArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let record = store.open(args.id.as_str())?;
    let root = cache_root(&SystemContext)?;
    let (phase, _, _) = view::live_phase(&record, root.as_path());
    let Some(paths) = record
        .result
        .clone()
        .filter(|_| matches!(phase, Phase::Published))
    else {
        eprintln!("not ready (phase {})", view::phase_label(phase));
        return Ok(4);
    };
    if args.deck {
        println!("{}", paths.deck);
        return Ok(0);
    }
    if args.pdf {
        println!("{}", paths.report);
        return Ok(0);
    }
    if args.dir {
        println!("{}", paths.output);
        return Ok(0);
    }
    if args.quiet {
        println!("{}", paths.deck);
        println!("{}", paths.report);
        println!("{}", paths.output);
        return Ok(0);
    }
    println!("session  {}", record.id);
    println!("pair     {} → {}", record.from, record.to);
    println!("deck     {}", paths.deck);
    println!("pdf      {}", paths.report);
    println!("dir      {}", paths.output);
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let cache = CardMetaCache::new(root);
    let total = record.drafts.len();
    for (index, draft) in record.drafts.iter().enumerate() {
        if let Some(meta) = cache.load(draft.term.as_str(), draft.understanding.as_str(), &pair)? {
            println!();
            print_card(index + 1, total, draft, &record, &meta);
        }
    }
    Ok(0)
}

fn regenerate(args: &RegenerateArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let mut record = store.open(args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    if record.drafts.is_empty() {
        bail!("no committed plan to regenerate: generate the session first");
    }
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let targets: Vec<DraftRecord> = match &args.card {
        Some(term) => record
            .drafts
            .iter()
            .filter(|draft| draft.term.as_str() == term.as_str())
            .cloned()
            .collect(),
        None if args.failed => view::incomplete_drafts(&record, root.as_path())
            .into_iter()
            .cloned()
            .collect(),
        None => bail!("regenerate needs --failed or --card <term>"),
    };
    if targets.is_empty() {
        bail!("no matching cards to regenerate");
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
    reset_to_understood(&mut record);
    store.save(&mut record)?;
    eprintln!(
        "dropped {} card(s); generate the session to rebuild them",
        targets.len()
    );
    Ok(0)
}

fn fix(args: &FixArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let mut record = store.open(args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    preflight_key()?;
    if record.drafts.is_empty() {
        bail!("no committed plan to fix: generate the session first");
    }
    let root = cache_root(&SystemContext)?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let index = record
        .drafts
        .iter()
        .position(|draft| draft.term.as_str() == args.card.as_str())
        .ok_or_else(|| anyhow!("no card '{}' in session '{}'", args.card, args.id))?;
    let current_term = record.drafts[index].term.clone();
    let current_understanding = record.drafts[index].understanding.clone();
    let draft = CardDraft::new(
        current_term.as_str(),
        current_understanding.as_str(),
        pair.clone(),
    );
    let generator = console::generator(PathBuf::from(record.out.clone()))?;
    let revision = generator.correct_card(&draft, args.note.as_str(), &pair)?;
    let (term, understanding, meta) = revision.into_parts();
    drop_artifacts(
        root.as_path(),
        &pair,
        current_term.as_str(),
        current_understanding.as_str(),
        false,
    )?;
    generator.store_card_meta(term.as_str(), understanding.as_str(), &pair, &meta)?;
    record.drafts[index] = DraftRecord {
        term,
        understanding,
    };
    reset_to_understood(&mut record);
    store.save(&mut record)?;
    eprintln!(
        "rewrote card '{}'; generate the session to rebuild it",
        args.card
    );
    Ok(0)
}

fn cancel(args: &IdArg) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let opened = store.open(args.id.as_str())?;
    // Only signal the pid when the lock proves a live worker actually owns it, so
    // a stale (possibly reused) pid is never sent a signal.
    if let Some(worker) = &opened.worker
        && liveness::is_held(&store.lock_path(args.id.as_str()))
    {
        liveness::terminate(worker.pid);
    }
    // Re-open after terminating so the worker's last write (it may have finished
    // and saved `published` as it died) is the base for the compare-and-swap.
    let mut record = store.open(args.id.as_str())?;
    record.worker = None;
    record.progress = None;
    if !matches!(
        record.phase,
        Phase::Published | Phase::Failed | Phase::Cancelled
    ) {
        record.phase = Phase::Cancelled;
    }
    store.save(&mut record)?;
    eprintln!("cancelled session {}", args.id);
    Ok(0)
}

fn ls(args: &LsArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    let root = cache_root(&SystemContext)?;
    let sessions = store.list()?;
    if sessions.is_empty() {
        eprintln!("no sessions");
        return Ok(0);
    }
    for record in &sessions {
        if args.quiet {
            println!("{}", record.id);
        } else {
            println!("{}", view::summary_line(record, root.as_path()));
        }
    }
    Ok(0)
}

fn rm(args: &RmArgs) -> Result<u8> {
    let store = SessionStore::system()?;
    if let Some(code) = missing(&store, args.id.as_str()) {
        return Ok(code);
    }
    let record = store.open(args.id.as_str())?;
    refuse_if_live(&store, &record)?;
    if args.cache {
        let root = cache_root(&SystemContext)?;
        let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
        for (term, understanding) in cached_cells(&record) {
            drop_artifacts(
                root.as_path(),
                &pair,
                term.as_str(),
                understanding.as_str(),
                false,
            )?;
        }
    }
    store.remove(args.id.as_str())?;
    eprintln!("removed session {}", args.id);
    Ok(0)
}

fn cache_path() -> Result<u8> {
    println!("{}", cache_root(&SystemContext)?.display());
    Ok(0)
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
    println!("  {:<8} {}", record.to, meta.target_sentence());
    println!(
        "  {:<8} {}",
        record.from,
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

/// Return the not-found exit code (3) after a stderr note, or None if it exists.
pub(in crate::cli::session) fn missing(store: &SessionStore, id: &str) -> Option<u8> {
    if store.exists(id) {
        return None;
    }
    eprintln!("no session '{id}'");
    Some(3)
}

pub(in crate::cli::session) fn refuse_if_live(
    store: &SessionStore,
    record: &SessionRecord,
) -> Result<()> {
    if let Some(worker) = &record.worker
        && liveness::is_held(&store.lock_path(&record.id))
    {
        bail!(
            "session '{}' has a running worker (pid {}); cancel it first",
            record.id,
            worker.pid
        );
    }
    Ok(())
}

pub(super) fn preflight_key() -> Result<()> {
    let saved = default_store(&SystemContext)
        .ok()
        .and_then(|store| store.read().ok())
        .and_then(|prefs| prefs.api_key)
        .filter(|key| !key.is_empty());
    let env = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());
    if env.is_none() && saved.is_none() {
        bail!("no Gemini API key found in GEMINI_API_KEY or saved preferences; set GEMINI_API_KEY");
    }
    Ok(())
}

fn drop_artifacts(
    root: &Path,
    pair: &LanguagePair,
    term: &str,
    understanding: &str,
    keep_meta: bool,
) -> Result<()> {
    let cache = CardCell::new(root.to_path_buf(), pair, term, understanding).cache();
    let folder = cache.path();
    for file in [VOICE_FILE, SCENE_FILE, ILLUSTRATION_FILE] {
        let path = folder.join(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    if !keep_meta {
        let path = folder.join(META_FILE);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Return every (term, understanding) cache cell a session may own: the committed
/// plan when one exists, otherwise each candidate sense (so an imported or
/// understood session still has its pre-stored meta cells removed).
fn cached_cells(record: &SessionRecord) -> Vec<(String, String)> {
    if !record.drafts.is_empty() {
        return record
            .drafts
            .iter()
            .map(|draft| (draft.term.clone(), draft.understanding.clone()))
            .collect();
    }
    record
        .candidates
        .iter()
        .flat_map(|stored| {
            let candidate = stored.clone().candidate();
            candidate
                .senses()
                .iter()
                .map(|sense| {
                    (
                        candidate.term().to_string(),
                        sense.understanding().to_string(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(in crate::cli::session) fn reset_to_understood(record: &mut SessionRecord) {
    record.phase = Phase::Understood;
    record.result = None;
    record.error = None;
    record.progress = None;
    record.worker = None;
}

fn record_of(draft: &CardDraft) -> DraftRecord {
    DraftRecord {
        term: String::from(draft.term()),
        understanding: String::from(draft.understanding()),
    }
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
        None => bail!("no words: pass --word <WORD> (repeatable) or --words FILE"),
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

fn reporter(quiet: bool) -> Box<dyn Reporter> {
    if quiet {
        return Box::new(QuietReporter);
    }
    Box::new(HumanReporter)
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
