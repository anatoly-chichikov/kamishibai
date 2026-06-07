//! Projection between the interactive `App` and the persistent `SessionRecord`.
//!
//! The TUI and the console share one session: the cache holds every card's
//! artifacts, and this bridge keeps the `session.json` index in step with the
//! live `App`. Readiness is never projected — it stays cache-derived (see
//! `view`); only the durable subset (language pair, typed words, curated
//! candidates, committed plan, phase, and published result) crosses over.

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::Result;

use crate::session::{CandidateRecord, CardDraft, LanguagePair, WordCandidate};
use crate::tui::{App, Screen};

use super::liveness;
use super::store::{
    DraftRecord, Phase, ResultRecord, SessionRecord, SessionStore, WorkerHandle, mint_id, now,
};

/// Project the live app into a persistable record under one identity. A live
/// `worker_pid` marks the session as generating; otherwise a populated Done
/// screen marks it published and everything else is understood.
pub(super) fn app_to_record(
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
        app.pair().support().to_string(),
        app.pair().target().to_string(),
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
        });
    }
    record
}

/// Rebuild the app and optional startup batch from a stored record. A published
/// session reopens on its finished cards; a session with a committed plan reopens
/// generating from cache; a curatable one reopens on the understanding.
pub(super) fn record_to_app(record: &SessionRecord) -> (App, Option<Vec<CardDraft>>) {
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let candidates: Vec<WordCandidate> = record
        .candidates
        .iter()
        .map(|stored| stored.clone().candidate())
        .collect();
    let mut app = App::new(pair.clone())
        .seeded_blob(record.words.join("\n"))
        .confirmed_target(record.to.clone());
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
    if matches!(record.phase, Phase::Failed | Phase::Cancelled) {
        return (app, None);
    }
    (app, Some(drafts))
}

/// The on-disk session one interactive run reads and writes as it advances.
///
/// Holds the store and the session identity so the shell can persist the live
/// app at every meaningful transition without re-minting an id each time. Saves
/// are debounced on a fingerprint of the durable subset so a busy generation
/// loop writes once, not once per frame.
pub(in crate::cli) struct TuiSession {
    store: SessionStore,
    id: Option<String>,
    created: Option<String>,
    source: String,
    senses: String,
    rev: u64,
    fingerprint: Option<u64>,
    lock: Option<File>,
}

impl TuiSession {
    /// Begin a fresh session whose id is minted on the first save.
    pub(in crate::cli) fn fresh() -> Result<Self> {
        Ok(Self {
            store: SessionStore::system()?,
            id: None,
            created: None,
            source: String::from("tui"),
            senses: String::from("custom"),
            rev: 0,
            fingerprint: None,
            lock: None,
        })
    }

    /// Resume an existing on-disk session under its original identity, keeping its
    /// recorded source, senses label, and revision so a save extends the existing
    /// compare-and-swap chain rather than re-basing onto whatever is on disk.
    pub(in crate::cli) fn resuming(
        id: String,
        created: String,
        source: String,
        senses: String,
        rev: u64,
    ) -> Result<Self> {
        Ok(Self {
            store: SessionStore::system()?,
            id: Some(id),
            created: Some(created),
            source,
            senses,
            rev,
            fingerprint: None,
            lock: None,
        })
    }

    /// Persist the live app if its durable subset changed since the last save.
    ///
    /// A no-op until the run has something to persist (understood candidates or
    /// a started card batch), so Welcome and Your-words never write a file.
    pub(in crate::cli) fn save(
        &mut self,
        app: &App,
        output: &Path,
        generating: bool,
    ) -> Result<()> {
        if app.candidates().is_empty() && app.cards().is_empty() {
            return Ok(());
        }
        let print = fingerprint(app, generating);
        if self.fingerprint == Some(print) {
            return Ok(());
        }
        let id = match &self.id {
            Some(id) => id.clone(),
            None => {
                let minted = mint_id(app.pair().target())?;
                self.id = Some(minted.clone());
                minted
            }
        };
        let created = match &self.created {
            Some(created) => created.clone(),
            None => {
                let stamped = now()?;
                self.created = Some(stamped.clone());
                stamped
            }
        };
        let worker_pid = if generating {
            Some(i32::try_from(std::process::id())?)
        } else {
            None
        };
        let mut record = app_to_record(
            app,
            id.clone(),
            created,
            self.source.as_str(),
            self.senses.as_str(),
            output.to_string_lossy().as_ref(),
            worker_pid,
        );
        // Real CAS against our own last-written revision: if another process
        // changed the session, the save is refused (best-effort here) rather than
        // re-basing and clobbering that newer state.
        record.rev = self.rev;
        self.store.save(&mut record)?;
        self.rev = record.rev;
        // Hold the advisory lock while generating so concurrent CLI commands see a
        // live worker; release it once generation stops. The session dir already
        // exists (the save above created it), so the lock file can be created.
        if generating {
            if self.lock.is_none() {
                self.lock = liveness::hold(&self.store.lock_path(id.as_str()))?;
            }
        } else {
            self.lock = None;
        }
        self.fingerprint = Some(print);
        Ok(())
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
    app.pair().support().hash(&mut hasher);
    app.pair().target().hash(&mut hasher);
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
            .confirmed_target("fr")
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
            .confirmed_target("fr")
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
                app.pair().support().to_string(),
                app.pair().target().to_string(),
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
