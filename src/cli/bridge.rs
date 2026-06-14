//! The TUI side of the session contract: projection between the interactive
//! `App` and the persistent `SessionRecord`, plus the [`SessionOpener`] port
//! implementation the console hands an `open`ed session to.
//!
//! The TUI and the console share one session: the cache holds every card's
//! artifacts, and this bridge keeps the `session.json` index in step with the
//! live `App`. Readiness is never projected — it stays cache-derived (see
//! `session::view`); only the durable subset (language pair, typed words,
//! curated candidates, committed plan, phase, and published result) crosses
//! over. The dependency is one-way: this file links the TUI to the console's
//! session model, and nothing under `session/` links back.

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::{Result, bail};

use crate::session::{CandidateRecord, CardDraft, LanguagePair, WordCandidate};
use crate::tui::{App, Screen};

use super::session::{
    DraftRecord, Phase, ResultRecord, SessionOpener, SessionRecord, SessionStore, WorkerHandle,
    mint_id, now,
};
use super::terminal::run_tui;

/// The TUI-side implementation of the console's [`SessionOpener`] port: resume
/// the stored session in the interactive terminal.
pub(super) struct TuiOpener;

impl SessionOpener for TuiOpener {
    fn open(&self, record: &SessionRecord) -> Result<()> {
        let resume = TuiSession::resuming(record)?;
        let (app, startup) = record_to_app(record);
        run_tui(app, startup, Some(resume))
    }
}

/// Project the live app into a persistable record under one identity. A live
/// `worker_pid` marks the session as generating; otherwise a populated Done
/// screen marks it published and everything else is understood.
fn app_to_record(
    app: &App,
    id: String,
    created: String,
    source: &str,
    senses: &str,
    output: &str,
    worker_pid: Option<i32>,
) -> SessionRecord {
    let words = app
        .blob()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    let candidates = app
        .candidates()
        .iter()
        .map(CandidateRecord::from_candidate)
        .collect();
    let mut record = SessionRecord::understood(
        id,
        created.clone(),
        app.pair().known().to_string(),
        app.pair().learning().to_string(),
        output.to_string(),
        senses.to_string(),
        source.to_string(),
        words,
        candidates,
    );
    record.drafts = app
        .cards()
        .iter()
        .map(|draft| DraftRecord {
            term: draft.term().to_string(),
            understanding: draft.understanding().to_string(),
        })
        .collect();
    let done = app.done_artifacts();
    if let Some(pid) = worker_pid {
        record.phase = Phase::Generating;
        record.worker = Some(WorkerHandle {
            pid,
            started: created,
        });
    } else if !done.deck.is_empty() {
        record.phase = Phase::Published;
        record.result = Some(ResultRecord {
            deck: done.deck.clone(),
            report: done.report.clone(),
            output: done.output.clone(),
            cards: record.drafts.len(),
            failed: 0,
        });
    }
    record
}

/// Rebuild the app and optional startup batch from a stored record. A published
/// session reopens on its finished cards; a session with a committed plan reopens
/// generating from cache; a curatable one reopens on the understanding.
fn record_to_app(record: &SessionRecord) -> (App, Option<Vec<CardDraft>>) {
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let candidates: Vec<WordCandidate> = record
        .candidates
        .iter()
        .map(|stored| stored.clone().candidate())
        .collect();
    let mut app = App::new(pair.clone())
        .seeded_blob(record.words.join("\n"))
        .confirmed_learning(record.learning.clone());
    if !candidates.is_empty() {
        app = app
            .with_screen(Screen::WhatIUnderstood)
            .understood(candidates);
    }
    if record.drafts.is_empty() {
        return (app, None);
    }
    let drafts: Vec<CardDraft> = record
        .drafts
        .iter()
        .map(|draft| {
            CardDraft::new(
                draft.term.as_str(),
                draft.understanding.as_str(),
                pair.clone(),
            )
        })
        .collect();
    if let (Phase::Published, Some(result)) = (record.phase, record.result.as_ref()) {
        let app = app
            .cards_started(drafts)
            .done_published(
                result.deck.clone(),
                result.report.clone(),
                result.output.clone(),
            )
            .with_screen(Screen::Done);
        return (app, None);
    }
    let app = app
        .with_screen(Screen::YourCards)
        .cards_started(drafts.clone());
    if matches!(
        record.phase,
        Phase::Failed | Phase::Cancelled | Phase::Partial
    ) {
        return (app, None);
    }
    (app, Some(drafts))
}

/// The on-disk session one interactive run reads and writes as it advances.
///
/// Holds the store and the session identity so the shell can persist the live
/// app at every meaningful transition without re-minting an id each time. Saves
/// are debounced on a fingerprint of the durable subset so a busy generation
/// loop writes once, not once per frame. `written` remembers the record this
/// window last wrote (or resumed from), so a save refuses to clobber a session
/// another process edited meanwhile.
pub(super) struct TuiSession {
    store: SessionStore,
    id: Option<String>,
    created: Option<String>,
    source: String,
    senses: String,
    written: Option<SessionRecord>,
    fingerprint: Option<u64>,
    lock: Option<File>,
}

impl TuiSession {
    /// Begin a fresh session whose id is minted on the first save.
    pub(super) fn fresh() -> Result<Self> {
        Ok(Self {
            store: SessionStore::system()?,
            id: None,
            created: None,
            source: String::from("tui"),
            senses: String::from("custom"),
            written: None,
            fingerprint: None,
            lock: None,
        })
    }

    /// Resume an existing on-disk session under its original identity, keeping
    /// the record as read so a later save detects outside edits.
    fn resuming(record: &SessionRecord) -> Result<Self> {
        Ok(Self {
            store: SessionStore::system()?,
            id: Some(record.id.clone()),
            created: Some(record.created.clone()),
            source: record.source.clone(),
            senses: record.senses.clone(),
            written: Some(record.clone()),
            fingerprint: None,
            lock: None,
        })
    }

    /// Claim the right to generate this session by taking its liveness lock
    /// BEFORE any record naming this process as the worker is written. Returns
    /// false when another live worker already holds it; idempotent while held.
    pub(super) fn claim(&mut self, app: &App) -> Result<bool> {
        if self.lock.is_some() {
            return Ok(true);
        }
        let id = self.ensure_id(app)?;
        self.lock = self.store.hold(id.as_str())?;
        Ok(self.lock.is_some())
    }

    /// Persist the live app if its durable subset changed since the last save.
    ///
    /// A no-op until the run has something to persist (understood candidates or
    /// a started card batch), so Welcome and Your-words never write a file. A
    /// failed save records the fingerprint anyway, so the event loop surfaces
    /// the error once instead of hammering the disk; the next edit retries.
    pub(super) fn save(&mut self, app: &App, output: &Path, generating: bool) -> Result<()> {
        if app.candidates().is_empty() && app.cards().is_empty() {
            return Ok(());
        }
        let print = fingerprint(app, generating);
        if self.fingerprint == Some(print) {
            return Ok(());
        }
        self.fingerprint = Some(print);
        let id = self.ensure_id(app)?;
        let created = self.ensure_created()?;
        if generating && self.lock.is_none() {
            self.lock = self.store.hold(id.as_str())?;
            if self.lock.is_none() {
                bail!("session '{id}' is being generated by another process");
            }
        }
        if !generating {
            self.lock = None;
        }
        let worker_pid = if generating {
            Some(i32::try_from(std::process::id())?)
        } else {
            None
        };
        let projected = app_to_record(
            app,
            id.clone(),
            created,
            self.source.as_str(),
            self.senses.as_str(),
            output.to_string_lossy().as_ref(),
            worker_pid,
        );
        match &self.written {
            None => self.store.create(&projected)?,
            Some(expected) => {
                let expected = expected.clone();
                self.store.update(id.as_str(), |on_disk| {
                    if *on_disk != expected {
                        bail!("session '{id}' changed outside this window; reopen it to continue");
                    }
                    *on_disk = projected.clone();
                    Ok(())
                })?;
            }
        }
        self.written = Some(projected);
        Ok(())
    }

    fn ensure_id(&mut self, app: &App) -> Result<String> {
        match &self.id {
            Some(id) => Ok(id.clone()),
            None => {
                let minted = mint_id(app.pair().learning())?;
                self.id = Some(minted.clone());
                Ok(minted)
            }
        }
    }

    fn ensure_created(&mut self) -> Result<String> {
        match &self.created {
            Some(created) => Ok(created.clone()),
            None => {
                let stamped = now()?;
                self.created = Some(stamped.clone());
                Ok(stamped)
            }
        }
    }
}

/// Hash the durable subset that decides whether a save is needed. Artifact
/// readiness is deliberately excluded — it lives in the cache, so a generation
/// loop does not rewrite the index on every per-card step.
fn fingerprint(app: &App, generating: bool) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    generating.hash(&mut hasher);
    screen_tag(app.screen()).hash(&mut hasher);
    app.blob().hash(&mut hasher);
    app.pair().known().hash(&mut hasher);
    app.pair().learning().hash(&mut hasher);
    for candidate in app.candidates() {
        candidate.term().hash(&mut hasher);
        candidate.ok().hash(&mut hasher);
        candidate.selected_senses().hash(&mut hasher);
        candidate.senses().len().hash(&mut hasher);
    }
    for draft in app.cards() {
        draft.term().hash(&mut hasher);
        draft.understanding().hash(&mut hasher);
    }
    app.done_artifacts().deck.hash(&mut hasher);
    hasher.finish()
}

/// Map one screen to a stable tag for fingerprinting, avoiding a numeric cast.
fn screen_tag(screen: Screen) -> u8 {
    match screen {
        Screen::Welcome => 0,
        Screen::YourWords => 1,
        Screen::WhatIUnderstood => 2,
        Screen::YourCards => 3,
        Screen::Done => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Sense;

    fn understood_app() -> App {
        App::new(LanguagePair::new("fr", "en"))
            .seeded_blob("canard\nflaner")
            .confirmed_learning("fr")
            .with_screen(Screen::WhatIUnderstood)
            .understood(vec![
                WordCandidate::with_selected_senses(
                    "canard",
                    vec![Sense::plain("a duck"), Sense::plain("a hoax")],
                    vec![1],
                    true,
                ),
                WordCandidate::new("flaner", "to stroll", false),
            ])
    }

    fn published_app() -> App {
        let pair = LanguagePair::new("fr", "en");
        App::new(pair.clone())
            .confirmed_learning("fr")
            .with_screen(Screen::YourCards)
            .cards_started(vec![CardDraft::new("canard", "a duck", pair)])
            .done_published("/o/deck.apkg", "/o/deck.pdf", "/o")
    }

    #[test]
    fn an_understood_app_round_trips_with_its_curation_intact() {
        let record = app_to_record(
            &understood_app(),
            String::from("fr-1"),
            String::from("t"),
            "tui",
            "primary",
            "/out",
            None,
        );
        let (app, startup) = record_to_app(&record);
        assert_eq!(
            (
                app.pair().known().to_string(),
                app.pair().learning().to_string(),
                app.screen(),
                app.candidates()[0].selected_senses().to_vec(),
                app.candidates()[1].ok(),
                startup.is_none(),
            ),
            (
                String::from("en"),
                String::from("fr"),
                Screen::WhatIUnderstood,
                vec![1],
                false,
                true,
            ),
            "an understood app must survive the record round-trip with its curation intact"
        );
    }

    #[test]
    fn a_published_app_reopens_on_its_finished_cards() {
        let record = app_to_record(
            &published_app(),
            String::from("fr-1"),
            String::from("t"),
            "tui",
            "primary",
            "/o",
            None,
        );
        let (app, startup) = record_to_app(&record);
        assert_eq!(
            (
                record.phase,
                startup.is_none(),
                app.done_artifacts().deck.clone(),
                app.screen(),
            ),
            (
                Phase::Published,
                true,
                String::from("/o/deck.apkg"),
                Screen::Done,
            ),
            "a published session must reopen on the done summary with no startup batch"
        );
    }

    #[test]
    fn a_generating_app_records_this_process_as_the_worker() {
        let record = app_to_record(
            &published_app(),
            String::from("fr-1"),
            String::from("t"),
            "tui",
            "primary",
            "/o",
            Some(4321),
        );
        assert_eq!(
            (record.phase, record.worker.map(|worker| worker.pid)),
            (Phase::Generating, Some(4321)),
            "a live worker pid must mark the session generating under that pid"
        );
    }
}
