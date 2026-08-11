//! The TUI side of the session contract: projection between the interactive
//! `App` and the persistent `SessionRecord`, plus the [`SessionOpener`] port
//! implementation the console hands an `open`ed session to.
//!
//! The TUI and the console share one session: the cache holds every card's
//! artifacts, and this bridge keeps the `session.json` index in step with the
//! live `App`. Readiness is never projected — it stays cache-derived (see
//! `session::view`); only the durable subset (language pair, typed words,
//! curated candidates, committed plan, session-scoped provider spend, phase,
//! and published result) crosses over. The dependency is one-way: this file
//! links the TUI to the console's session model, and nothing under `session/`
//! links back.

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::session::{CandidateRecord, CardDraft, LanguagePair, WordCandidate};
use crate::tui::{App, Screen};

use super::session::{
    DraftRecord, Phase, ResultRecord, SessionCostScope, SessionOpener, SessionRecord, SessionStore,
    WorkerHandle, mint_id, now,
};
use super::terminal::run_tui;

/// The TUI-side implementation of the console's [`SessionOpener`] port: resume
/// the stored session in the interactive terminal.
pub(super) struct TuiOpener;

impl SessionOpener for TuiOpener {
    fn open(&self, record: &SessionRecord) -> Result<()> {
        let resume = TuiSession::resuming(record)?;
        let hydrated = resume.hydrate(record)?;
        let (app, startup) = record_to_app(&hydrated);
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
    )
    .with_sentences(app.sentence_settings());
    record.drafts = app
        .cards()
        .iter()
        .map(|draft| DraftRecord {
            term: draft.term().to_string(),
            understanding: draft.understanding().to_string(),
            costs: crate::session::ArtifactCosts::from_artifacts(draft.artifacts()),
            rewrite: draft.rewrite().cloned(),
            meta_request: draft.meta_request().cloned(),
        })
        .collect();
    let done = app.done_artifacts();
    if !done.deck.is_empty() {
        record.result = Some(ResultRecord {
            deck: done.deck.clone(),
            report: done.report.clone(),
            output: done.output.clone(),
            cards: done.cards,
            failed: done.failed,
        });
    }
    if let Some(pid) = worker_pid {
        record.phase = Phase::Generating;
        record.worker = Some(WorkerHandle {
            pid,
            started: created,
        });
    } else if let Some(result) = record.result.as_ref() {
        record.phase = if result.failed > 0 {
            Phase::Partial
        } else {
            Phase::Published
        };
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
        .with_sentence_settings(record.sentences)
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
            let hydrated = CardDraft::new(
                draft.term.as_str(),
                draft.understanding.as_str(),
                pair.clone(),
            );
            let hydrated = match &draft.meta_request {
                Some(selection) => hydrated.requesting_meta(selection.clone()),
                None => hydrated,
            };
            hydrated
                .with_rewrite(draft.rewrite.clone())
                .with_costs(draft.costs)
        })
        .collect();
    let mut app = app.cards_started(drafts.clone());
    if let Some(result) = record.result.as_ref() {
        app = app.done_published_counted(
            result.deck.clone(),
            result.report.clone(),
            result.output.clone(),
            result.cards,
            result.failed,
        );
    }
    if record.phase == Phase::Published
        && record.result.is_some()
        && !drafts.iter().any(|draft| draft.staged_rewrite().is_some())
    {
        return (app.with_screen(Screen::Done), None);
    }
    let app = app.with_screen(Screen::YourCards);
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
    costs: SessionCostScope,
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
            costs: SessionCostScope::default(),
        })
    }

    /// Resume an existing on-disk session under its original identity, keeping
    /// the record as read so a later save detects outside edits.
    fn resuming(record: &SessionRecord) -> Result<Self> {
        Self::resuming_in(record, SessionStore::system()?)
    }

    /// Resume through an explicit store for deterministic lifecycle tests.
    pub(super) fn resuming_in(record: &SessionRecord, store: SessionStore) -> Result<Self> {
        let costs = SessionCostScope::default();
        if !record.drafts.is_empty() {
            let journal = store.cost_journal(record);
            costs.bind(journal)?;
        }
        Ok(Self {
            store,
            id: Some(record.id.clone()),
            created: Some(record.created.clone()),
            source: record.source.clone(),
            senses: record.senses.clone(),
            written: Some(record.clone()),
            fingerprint: None,
            lock: None,
            costs,
        })
    }

    /// Share this TUI run's late-bound journal with cloned Gemini workflows.
    pub(super) fn cost_scope(&self) -> SessionCostScope {
        self.costs.clone()
    }

    /// Return the output directory already bound to a resumed session.
    pub(super) fn stored_output(&self) -> Option<PathBuf> {
        self.written
            .as_ref()
            .map(|record| PathBuf::from(record.out.as_str()))
    }

    /// Detach from a completed record so the same TUI can mint a fresh session.
    pub(super) fn start_next_batch(&mut self) -> Result<()> {
        if self.lock.is_some() {
            bail!("the completed session still holds its generation lock");
        }
        self.costs.reset()?;
        self.id = None;
        self.created = None;
        self.source = String::from("tui");
        self.senses = String::from("custom");
        self.written = None;
        self.fingerprint = None;
        Ok(())
    }

    /// Close the active run as cancelled, then rotate identity and cost scope.
    pub(super) fn cancel_and_start_next(&mut self, app: &App, output: &Path) -> Result<()> {
        let id = self.ensure_id(app)?;
        let created = self.ensure_created()?;
        let mut projected = app_to_record(
            app,
            id.clone(),
            created,
            self.source.as_str(),
            self.senses.as_str(),
            output.to_string_lossy().as_ref(),
            None,
        );
        projected.phase = Phase::Cancelled;
        let fallback = record_costs(&projected);
        apply_costs(
            &mut projected,
            self.costs.overlay_if_bound(fallback.as_slice())?,
        );
        self.write_projection(id.as_str(), &projected)?;
        self.written = Some(projected);
        self.fingerprint = Some(fingerprint(app, false));
        self.lock = None;
        self.start_next_batch()
    }

    fn hydrate(&self, record: &SessionRecord) -> Result<SessionRecord> {
        let mut hydrated = record.clone();
        apply_costs(
            &mut hydrated,
            self.costs
                .overlay_if_bound(record_costs(record).as_slice())?,
        );
        Ok(hydrated)
    }

    /// Reconcile live cards with provider-boundary journal totals before resuming work.
    pub(super) fn hydrate_app(&self, app: &App) -> Result<App> {
        let costs = self.costs.overlay_if_bound(app_costs(app).as_slice())?;
        let drafts = app
            .cards()
            .iter()
            .cloned()
            .zip(costs)
            .map(|(draft, absolute)| draft.with_costs(absolute))
            .collect();
        Ok(app.clone().cards_replaced(drafts))
    }

    /// Claim generation and persist its committed plan before any provider work starts.
    pub(super) fn claim_and_save(&mut self, app: &App, output: &Path) -> Result<bool> {
        let id = self.ensure_id(app)?;
        let created = self.ensure_created()?;
        let held = self.lock.take();
        let retain_on_failure = held.is_some();
        let lock = match held {
            Some(lock) => lock,
            None => match self.store.hold(id.as_str())? {
                Some(lock) => lock,
                None => return Ok(false),
            },
        };
        let projection = (|| {
            let mut projected = app_to_record(
                app,
                id.clone(),
                created.clone(),
                self.source.as_str(),
                self.senses.as_str(),
                output.to_string_lossy().as_ref(),
                Some(i32::try_from(std::process::id())?),
            );
            let fallback = record_costs(&projected);
            apply_costs(
                &mut projected,
                self.costs.overlay_if_bound(fallback.as_slice())?,
            );
            Ok::<SessionRecord, anyhow::Error>(projected)
        })();
        let projected = match projection {
            Ok(projected) => projected,
            Err(error) => {
                if retain_on_failure {
                    self.lock = Some(lock);
                }
                return Err(error);
            }
        };
        if let Err(error) = self.write_projection(id.as_str(), &projected) {
            if retain_on_failure {
                self.lock = Some(lock);
            }
            return Err(error);
        }
        self.written = Some(projected.clone());
        let journal = self.store.cost_journal_for(id.as_str(), created.as_str());
        if let Err(error) = journal
            .seed(record_costs(&projected).as_slice())
            .and_then(|()| self.costs.bind(journal))
        {
            self.fingerprint = None;
            self.lock = Some(lock);
            return Err(error);
        }
        self.fingerprint = Some(fingerprint(app, true));
        self.lock = Some(lock);
        Ok(true)
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
            bail!("session '{id}' must persist its committed plan before generation starts");
        }
        let worker_pid = if generating {
            Some(i32::try_from(std::process::id())?)
        } else {
            None
        };
        let mut projected = app_to_record(
            app,
            id.clone(),
            created,
            self.source.as_str(),
            self.senses.as_str(),
            output.to_string_lossy().as_ref(),
            worker_pid,
        );
        let fallback = record_costs(&projected);
        apply_costs(
            &mut projected,
            self.costs.overlay_if_bound(fallback.as_slice())?,
        );
        if let Err(error) = self.write_projection(id.as_str(), &projected) {
            if !generating {
                self.lock = None;
            }
            return Err(error);
        }
        self.written = Some(projected);
        if !generating {
            self.lock = None;
        }
        Ok(())
    }

    fn write_projection(&self, id: &str, projected: &SessionRecord) -> Result<()> {
        match &self.written {
            None => self.store.create(projected),
            Some(expected) => {
                let expected = expected.clone();
                self.store.update(id, |on_disk| {
                    if *on_disk != expected {
                        bail!("session '{id}' changed outside this window; reopen it to continue");
                    }
                    *on_disk = projected.clone();
                    Ok(())
                })?;
                Ok(())
            }
        }
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

fn app_costs(app: &App) -> Vec<crate::session::ArtifactCosts> {
    app.cards()
        .iter()
        .map(|draft| crate::session::ArtifactCosts::from_artifacts(draft.artifacts()))
        .collect()
}

fn record_costs(record: &SessionRecord) -> Vec<crate::session::ArtifactCosts> {
    record.drafts.iter().map(|draft| draft.costs).collect()
}

fn apply_costs(record: &mut SessionRecord, costs: Vec<crate::session::ArtifactCosts>) {
    for (draft, absolute) in record.drafts.iter_mut().zip(costs) {
        draft.costs = absolute;
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
    app.sentence_settings().hash(&mut hasher);
    for candidate in app.candidates() {
        candidate.term().hash(&mut hasher);
        candidate.ok().hash(&mut hasher);
        candidate.selected_senses().hash(&mut hasher);
        candidate.senses().len().hash(&mut hasher);
    }
    for draft in app.cards() {
        draft.term().hash(&mut hasher);
        draft.understanding().hash(&mut hasher);
        draft.meta_request().hash(&mut hasher);
        draft.rewrite().hash(&mut hasher);
        for artifact in [
            crate::session::Artifact::Meta,
            crate::session::Artifact::Sound,
            crate::session::Artifact::Scene,
            crate::session::Artifact::Picture,
        ] {
            draft_cost(draft, artifact).hash(&mut hasher);
        }
    }
    app.done_artifacts().deck.hash(&mut hasher);
    app.done_artifacts().cards.hash(&mut hasher);
    app.done_artifacts().failed.hash(&mut hasher);
    hasher.finish()
}

fn draft_cost(draft: &CardDraft, artifact: crate::session::Artifact) -> Option<u64> {
    let costs = crate::session::ArtifactCosts::from_artifacts(draft.artifacts());
    costs.cost(artifact).map(|cost| cost.nanos())
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
    use crate::session::{
        Artifact, ArtifactSlot, CardArtifacts, CardMeta, GenerationCost, Sense,
        SentenceBatchSettings, SentenceLabelSelection, SentenceLevel, SentenceTypeMix,
        SessionEngine,
    };

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
    fn sentence_settings_round_trip_and_participate_in_the_save_fingerprint() {
        let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Mixed);
        let plain = understood_app();
        let configured = plain.clone().with_sentence_settings(settings);
        let record = app_to_record(
            &configured,
            String::from("fr-settings"),
            String::from("t"),
            "tui",
            "primary",
            "/out",
            None,
        );
        let (reopened, _) = record_to_app(&record);
        assert_eq!(
            (
                record.sentences,
                reopened.sentence_settings(),
                fingerprint(&plain, false) != fingerprint(&configured, false),
            ),
            (settings, settings, true),
            "TUI session projection lost sentence settings or debounced their change"
        );
    }

    #[test]
    fn an_initial_meta_request_round_trips_through_the_session_record() {
        let pair = LanguagePair::new("fr", "en");
        let request =
            SentenceLabelSelection::empty().choosing(crate::session::SentenceAxis::Level, 2);
        let app = App::new(pair.clone())
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair).requesting_meta(request.clone()),
            ]);
        let record = app_to_record(
            &app,
            String::from("fr-request"),
            String::from("t"),
            "tui",
            "primary",
            "/out",
            None,
        );
        let (reopened, _) = record_to_app(&record);
        assert_eq!(
            (
                record.drafts[0].meta_request.as_ref(),
                reopened.cards()[0].meta_request(),
            ),
            (Some(&request), Some(&request)),
            "TUI session projection discarded a pending initial metadata request"
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
    fn a_partial_publish_keeps_its_tally_across_reopen_and_save() {
        let pair = LanguagePair::new("fr", "en");
        let app = App::new(pair.clone())
            .confirmed_learning("fr")
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair.clone()),
                CardDraft::new("flaner", "to stroll", pair),
            ])
            .done_published_counted("/o/deck.apkg", "/o/deck.pdf", "/o", 1, 1);
        let record = app_to_record(
            &app,
            String::from("fr-partial"),
            String::from("t"),
            "tui",
            "primary",
            "/o",
            None,
        );
        let (reopened, startup) = record_to_app(&record);
        let saved = app_to_record(
            &reopened,
            String::from("fr-partial"),
            String::from("t"),
            "tui",
            "primary",
            "/o",
            None,
        );
        assert_eq!(
            (
                record.phase,
                record.result,
                reopened.screen(),
                reopened.done_artifacts().cards,
                reopened.done_artifacts().failed,
                startup.is_none(),
                saved.phase,
                saved.result,
            ),
            (
                Phase::Partial,
                Some(ResultRecord {
                    deck: String::from("/o/deck.apkg"),
                    report: String::from("/o/deck.pdf"),
                    output: String::from("/o"),
                    cards: 1,
                    failed: 1,
                }),
                Screen::YourCards,
                1,
                1,
                true,
                Phase::Partial,
                Some(ResultRecord {
                    deck: String::from("/o/deck.apkg"),
                    report: String::from("/o/deck.pdf"),
                    output: String::from("/o"),
                    cards: 1,
                    failed: 1,
                }),
            ),
            "partial publication tally drifted when cache readiness was not hydrated"
        );
    }

    #[test]
    fn cancelling_without_ready_cards_rotates_into_a_clean_understood_session() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let pair = LanguagePair::new("fr", "en");
        let app = understood_app()
            .with_screen(Screen::YourCards)
            .cards_started(vec![CardDraft::new("canard", "a duck", pair)]);
        let record = app_to_record(
            &app,
            String::from("fr-old"),
            String::from("created-old"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store.create(&record).expect("active session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("session must resume");
        session
            .claim_and_save(&app, Path::new("/o"))
            .expect("active session must claim");
        session
            .cancel_and_start_next(&app, Path::new("/o"))
            .expect("active session must cancel and rotate");
        let review = app.generation_cancelled_to_review();
        session
            .save(&review, Path::new("/o"), false)
            .expect("preserved review must save under a new identity");
        let new_id = session
            .id
            .clone()
            .expect("new session must have an identity");
        let old: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-old/session.json"))
                .expect("cancelled record must remain")
                .as_slice(),
        )
        .expect("cancelled record must decode");
        let new: SessionRecord = serde_json::from_slice(
            std::fs::read(
                home.path()
                    .join("sessions")
                    .join(new_id)
                    .join("session.json"),
            )
            .expect("preserved review record must exist")
            .as_slice(),
        )
        .expect("preserved review record must decode");
        let (_, startup) = record_to_app(&new);
        assert_eq!(
            (
                old.id.as_str(),
                old.phase,
                old.drafts.len(),
                old.worker.is_none(),
                new.id != old.id,
                new.phase,
                new.drafts.len(),
                new.words,
                new.candidates[0]
                    .clone()
                    .candidate()
                    .selected_senses()
                    .to_vec(),
                startup.is_none(),
            ),
            (
                "fr-old",
                Phase::Cancelled,
                1,
                true,
                true,
                Phase::Understood,
                0,
                vec![String::from("canard"), String::from("flaner")],
                vec![1],
                true,
            ),
            "cancelled generation reused its identity, costs, or lost the preserved review"
        );
    }

    #[test]
    fn clearing_an_understood_batch_cancels_its_identity_and_forgets_review_state() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let app = understood_app().with_screen(Screen::YourWords);
        let record = app_to_record(
            &app,
            String::from("fr-review"),
            String::from("created-old"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store.create(&record).expect("review session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store).expect("review session must resume");
        session
            .cancel_and_start_next(&app, Path::new("/o"))
            .expect("review session must cancel and rotate");
        let cleared = app.starting_new_batch();
        session
            .save(&cleared, Path::new("/o"), false)
            .expect("an empty next batch must remain unpersisted");
        let old: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-review/session.json"))
                .expect("cancelled review must remain")
                .as_slice(),
        )
        .expect("cancelled review must decode");
        assert_eq!(
            (
                old.phase,
                old.words,
                old.candidates.len(),
                session.id.is_none(),
                cleared.screen(),
                cleared.blob(),
                cleared.candidates().len(),
            ),
            (
                Phase::Cancelled,
                vec![String::from("canard"), String::from("flaner")],
                2,
                true,
                Screen::YourWords,
                "",
                0,
            ),
            "cleared words reused the discarded review identity or hidden candidates"
        );
    }

    #[test]
    fn a_resumed_session_keeps_its_stored_output() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let record = app_to_record(
            &understood_app(),
            String::from("fr-1"),
            String::from("t"),
            "tui",
            "primary",
            "/legacy/kamishibai-out",
            None,
        );
        let session =
            TuiSession::resuming_in(&record, store).expect("session must resume from its record");
        assert_eq!(
            session.stored_output(),
            Some(PathBuf::from("/legacy/kamishibai-out")),
            "resuming a session replaced its stored output with the new default"
        );
    }

    #[test]
    fn a_completed_tui_session_rotates_identity_and_cost_scope_for_the_next_batch() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let record = app_to_record(
            &published_app(),
            String::from("fr-old"),
            String::from("created-old"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store
            .create(&record)
            .expect("published session must persist");
        let old_journal = store.cost_journal(&record);
        old_journal
            .seed(record_costs(&record).as_slice())
            .expect("old cost journal must seed");
        old_journal
            .charge(0, Artifact::Meta, GenerationCost::from_nanos(700))
            .expect("old provider spend must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("session must resume");
        let costs = session.cost_scope();
        let old_cost = costs
            .absolute(0, crate::session::ArtifactCosts::default())
            .expect("old cost must be readable")
            .cost(Artifact::Meta);
        session
            .start_next_batch()
            .expect("completed session must detach");
        let pair = LanguagePair::new("fr", "en");
        let next = App::new(pair.clone())
            .confirmed_learning("fr")
            .with_screen(Screen::YourCards)
            .cards_started(vec![CardDraft::new("chouette", "great", pair)]);
        let claimed = session
            .claim_and_save(&next, Path::new("/o"))
            .expect("next batch must claim");
        let new_id = session.id.clone().expect("next batch must mint an id");
        let fresh_cost = costs
            .absolute(0, crate::session::ArtifactCosts::default())
            .expect("new cost journal must open empty")
            .cost(Artifact::Meta);
        let old: SessionRecord = serde_json::from_slice(
            std::fs::read(
                home.path()
                    .join("sessions")
                    .join("fr-old")
                    .join("session.json"),
            )
            .expect("old session must remain readable")
            .as_slice(),
        )
        .expect("old session must decode");
        assert_eq!(
            (
                claimed,
                old_cost,
                fresh_cost,
                new_id != "fr-old",
                old.phase,
                old.result.map(|result| result.deck),
            ),
            (
                true,
                Some(GenerationCost::from_nanos(700)),
                None,
                true,
                Phase::Published,
                Some(String::from("/o/deck.apkg")),
            ),
            "the next TUI batch reused the published session identity or provider spend"
        );
    }

    #[test]
    fn configured_completed_batch_resets_fresh_settings_and_preserves_its_record() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let settings = SentenceBatchSettings::new(Some(SentenceLevel::C1), SentenceTypeMix::Mixed);
        let completed = published_app().with_sentence_settings(settings);
        let record = app_to_record(
            &completed,
            String::from("fr-configured"),
            String::from("created-old"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store
            .create(&record)
            .expect("configured published session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("session must resume");
        session
            .start_next_batch()
            .expect("completed session must detach");
        let fresh = completed.starting_new_batch();
        let fresh_settings = fresh.sentence_settings();
        let next = fresh
            .seeded_blob("chouette")
            .confirmed_learning("fr")
            .with_screen(Screen::WhatIUnderstood)
            .understood(vec![WordCandidate::new("chouette", "great", true)]);
        session
            .save(&next, Path::new("/o"), false)
            .expect("fresh review must persist under a new identity");
        let new_id = session.id.clone().expect("fresh batch must mint an id");
        let old: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-configured/session.json"))
                .expect("published session must remain readable")
                .as_slice(),
        )
        .expect("published session must decode");
        let new: SessionRecord = serde_json::from_slice(
            std::fs::read(
                home.path()
                    .join("sessions")
                    .join(new_id.as_str())
                    .join("session.json"),
            )
            .expect("fresh session must remain readable")
            .as_slice(),
        )
        .expect("fresh session must decode");
        assert_eq!(
            (
                old.sentences,
                old.phase,
                old.result.map(|result| result.deck),
                new_id != old.id,
                fresh_settings,
                next.sentence_settings(),
                new.sentences,
                new.phase,
            ),
            (
                settings,
                Phase::Published,
                Some(String::from("/o/deck.apkg")),
                true,
                SentenceBatchSettings::default(),
                SentenceBatchSettings::default(),
                SentenceBatchSettings::default(),
                Phase::Understood,
            ),
            "new batch inherited completed settings or rewrote the published session record"
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

    #[test]
    fn a_session_reopen_restores_its_displayed_generation_cost() {
        let pair = LanguagePair::new("fr", "en");
        let costs = crate::session::ArtifactCosts::default()
            .charged(Artifact::Picture, GenerationCost::from_nanos(120_000_000))
            .charged(Artifact::Picture, GenerationCost::from_nanos(220_000_000));
        let app = App::new(pair.clone())
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair).with_costs(costs),
            ]);
        let record = app_to_record(
            &app,
            String::from("fr-1"),
            String::from("t"),
            "tui",
            "primary",
            "/o",
            Some(4321),
        );
        let (reopened, _) = record_to_app(&record);
        assert_eq!(
            reopened.cards()[0].artifacts().picture().cost(),
            Some(GenerationCost::from_nanos(340_000_000)),
            "reopening a session erased the generation cost already shown in the UI"
        );
    }

    #[test]
    fn a_tui_reopen_uses_journal_totals_over_stale_draft_costs() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let pair = LanguagePair::new("fr", "en");
        let stale = crate::session::ArtifactCosts::default()
            .charged(Artifact::Picture, GenerationCost::from_nanos(100_000_000));
        let app = App::new(pair.clone())
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair).with_costs(stale),
            ]);
        let record = app_to_record(
            &app,
            String::from("fr-1"),
            String::from("created-a"),
            "tui",
            "primary",
            "/o",
            Some(4321),
        );
        store.create(&record).expect("stale session must persist");
        let journal = store.cost_journal(&record);
        journal
            .seed(record_costs(&record).as_slice())
            .expect("journal must seed");
        journal
            .charge(
                0,
                Artifact::Picture,
                GenerationCost::from_nanos(250_000_000),
            )
            .expect("provider spend must persist before a crash");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("session must resume");
        let hydrated = session.hydrate(&record).expect("record must hydrate");
        let (reopened, _) = record_to_app(&hydrated);
        session
            .save(&reopened, Path::new("/o"), false)
            .expect("absolute totals must save");
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-1/session.json"))
                .expect("session must reopen")
                .as_slice(),
        )
        .expect("session must decode");
        assert_eq!(
            (
                reopened.cards()[0].artifacts().picture().cost(),
                stored.drafts[0].costs.cost(Artifact::Picture),
            ),
            (
                Some(GenerationCost::from_nanos(350_000_000)),
                Some(GenerationCost::from_nanos(350_000_000)),
            ),
            "TUI reopen trusted stale DraftRecord costs instead of the provider-boundary journal"
        );
    }

    #[test]
    fn session_fingerprint_changes_when_displayed_cost_changes() {
        let pair = LanguagePair::new("fr", "en");
        let plain = App::new(pair.clone()).cards_started(vec![CardDraft::new(
            "canard",
            "a duck",
            pair.clone(),
        )]);
        let artifacts = CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta),
            ArtifactSlot::fresh(Artifact::Scene),
            ArtifactSlot::fresh(Artifact::Picture)
                .attempted_with(GenerationCost::from_nanos(10_000_000)),
            ArtifactSlot::fresh(Artifact::Sound),
        );
        let priced = App::new(pair.clone()).cards_started(vec![
            CardDraft::new("canard", "a duck", pair).with_artifacts(artifacts),
        ]);
        assert_ne!(
            fingerprint(&plain, true),
            fingerprint(&priced, true),
            "a cost-only UI update was debounced instead of being saved for reopen"
        );
    }

    #[test]
    fn session_fingerprint_changes_when_an_initial_meta_request_clears() {
        let pair = LanguagePair::new("fr", "en");
        let request =
            SentenceLabelSelection::empty().choosing(crate::session::SentenceAxis::Level, 2);
        let pending = App::new(pair.clone()).cards_started(vec![
            CardDraft::new("canard", "a duck", pair.clone()).requesting_meta(request),
        ]);
        let cleared =
            App::new(pair.clone()).cards_started(vec![CardDraft::new("canard", "a duck", pair)]);
        assert_ne!(
            fingerprint(&pending, true),
            fingerprint(&cleared, true),
            "clearing a zero-cost initial metadata request was debounced instead of saved"
        );
    }

    #[test]
    fn a_queued_rewrite_is_saved_while_generation_is_already_active() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let pair = LanguagePair::new("fr", "en");
        let app = App::new(pair.clone())
            .with_screen(Screen::YourCards)
            .cards_started(vec![CardDraft::new("canard", "a duck", pair)]);
        let record = app_to_record(
            &app,
            String::from("fr-rewrite"),
            String::from("created-a"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store.create(&record).expect("the session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        session
            .claim_and_save(&app, Path::new("/o"))
            .expect("generation must be claimed");
        let rewritten = app.cards()[0]
            .clone()
            .rewriting(SentenceLabelSelection::empty(), "make it formal");
        session
            .save(&app.cards_replaced(vec![rewritten]), Path::new("/o"), true)
            .expect("queued rewrite must save");
        let stored: SessionRecord = serde_json::from_str(
            std::fs::read_to_string(
                home.path()
                    .join("sessions")
                    .join("fr-rewrite")
                    .join("session.json"),
            )
            .expect("saved session must reopen")
            .as_str(),
        )
        .expect("saved session must deserialize");
        assert_eq!(
            stored.drafts[0]
                .rewrite
                .as_ref()
                .map(|rewrite| rewrite.note()),
            Some("make it formal"),
            "the active-session fingerprint debounced a queued rewrite"
        );
    }

    #[test]
    fn a_staged_rewrite_reopens_with_previous_meta_without_starting_correction() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let pair = LanguagePair::new("fr", "en");
        let meta = CardMeta::new(
            "/ka.naʁ/",
            "/lə ka.naʁ naʒ/",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A common concrete noun",
            "Le canard nage",
        );
        let draft = CardDraft::new("canard", "a duck", pair.clone())
            .with_meta(meta, None)
            .staging_rewrite(SentenceLabelSelection::empty(), "make it formal");
        let record = app_to_record(
            &App::new(pair)
                .with_screen(Screen::YourCards)
                .cards_started(vec![draft])
                .done_published("/o/deck.apkg", "/o/deck.pdf", "/o"),
            String::from("fr-staged"),
            String::from("created-a"),
            "tui",
            "primary",
            "/o",
            None,
        );
        let (reopened, startup) = record_to_app(&record);
        let engine = SessionEngine::start(startup.expect("staged session must hydrate cache rows"));
        store
            .create(&record)
            .expect("published session must persist");
        let mut hydration =
            TuiSession::resuming_in(&record, store.clone()).expect("published session must resume");
        hydration
            .claim_and_save(&reopened, Path::new("/o"))
            .expect("cache hydration must claim the session");
        drop(hydration);
        let path = home.path().join("sessions/fr-staged/session.json");
        let claimed: SessionRecord = serde_json::from_slice(
            std::fs::read(&path)
                .expect("claimed session must read")
                .as_slice(),
        )
        .expect("claimed session must decode");
        let (crashed, crash_startup) = record_to_app(&claimed);
        let active = crashed
            .clone()
            .cards_replaced(
                crashed
                    .cards()
                    .iter()
                    .cloned()
                    .map(CardDraft::starting_rewrite)
                    .collect(),
            )
            .publication_cleared();
        let mut generation =
            TuiSession::resuming_in(&claimed, store).expect("crashed session must resume");
        generation
            .claim_and_save(&active, Path::new("/o"))
            .expect("Ctrl+G generation must claim the session");
        let cleared: SessionRecord = serde_json::from_slice(
            std::fs::read(path)
                .expect("started session must read")
                .as_slice(),
        )
        .expect("started session must decode");
        assert_eq!(
            (
                record.phase,
                reopened.screen(),
                reopened.cards_pending(),
                reopened.done_artifacts().deck.as_str(),
                reopened.cards()[0].meta().map(CardMeta::target_sentence),
                reopened.cards()[0]
                    .rewrite()
                    .map(crate::session::CardRewrite::started),
                engine.next_target(),
                engine.drafts()[0].rewrite().is_some(),
                (
                    claimed.phase,
                    claimed.result.as_ref().map(|result| {
                        (
                            result.deck.as_str(),
                            result.report.as_str(),
                            result.output.as_str(),
                        )
                    }),
                    claimed
                        .drafts
                        .first()
                        .and_then(|draft| draft.rewrite.as_ref())
                        .map(crate::session::CardRewrite::started),
                ),
                (
                    crashed.screen(),
                    crashed.cards_pending(),
                    (
                        crashed.done_artifacts().deck.as_str(),
                        crashed.done_artifacts().report.as_str(),
                        crashed.done_artifacts().output.as_str(),
                    ),
                    crash_startup.is_some(),
                ),
                (
                    cleared.result.is_none(),
                    cleared
                        .drafts
                        .first()
                        .and_then(|draft| draft.rewrite.as_ref())
                        .map(crate::session::CardRewrite::started),
                ),
            ),
            (
                Phase::Published,
                Screen::YourCards,
                1,
                "/o/deck.apkg",
                Some("Le canard nage"),
                Some(false),
                Some((0, Artifact::Sound)),
                true,
                (
                    Phase::Generating,
                    Some(("/o/deck.apkg", "/o/deck.pdf", "/o")),
                    Some(false),
                ),
                (
                    Screen::YourCards,
                    1,
                    ("/o/deck.apkg", "/o/deck.pdf", "/o"),
                    true,
                ),
                (true, Some(true)),
            ),
            "startup hydration lost published output or Ctrl+G failed to clear it transactionally"
        );
    }

    #[test]
    fn a_conflicted_generation_claim_releases_the_lock_without_seeding_slots() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let record = app_to_record(
            &understood_app(),
            String::from("fr-race"),
            String::from("created-a"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store.create(&record).expect("the session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        store
            .update("fr-race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing update must persist");
        let pair = LanguagePair::new("fr", "en");
        let target = understood_app()
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair.clone()),
                CardDraft::new("canard", "a hoax", pair),
            ]);
        let claim = session.claim_and_save(&target, Path::new("/o"));
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let journal = std::fs::read_dir(home.path().join("sessions/fr-race"))
            .expect("the session directory must list")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("costs-"));
        let lock_released = store
            .hold("fr-race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                claim.is_err(),
                session.lock.is_none(),
                stored.senses,
                stored.drafts.is_empty(),
                journal,
                lock_released,
            ),
            (true, true, String::from("all"), true, false, true),
            "a stale TUI claim started or poisoned a plan after losing its optimistic save race"
        );
    }

    #[test]
    fn a_reclaim_conflict_keeps_an_already_held_generation_lock() {
        let home = tempfile::TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let record = app_to_record(
            &understood_app(),
            String::from("fr-held-race"),
            String::from("created-a"),
            "tui",
            "primary",
            "/o",
            None,
        );
        store.create(&record).expect("the session must persist");
        let mut session =
            TuiSession::resuming_in(&record, store.clone()).expect("the session must resume");
        let pair = LanguagePair::new("fr", "en");
        let target = understood_app()
            .with_screen(Screen::YourCards)
            .cards_started(vec![
                CardDraft::new("canard", "a duck", pair.clone()),
                CardDraft::new("canard", "a hoax", pair),
            ]);
        session
            .claim_and_save(&target, Path::new("/o"))
            .expect("the first claim must persist");
        store
            .update("fr-held-race", |fresh| {
                fresh.senses = String::from("all");
                Ok(())
            })
            .expect("the competing update must persist");
        let claim = session.claim_and_save(&target, Path::new("/o"));
        let stored: SessionRecord = serde_json::from_slice(
            std::fs::read(home.path().join("sessions/fr-held-race/session.json"))
                .expect("the competing record must read")
                .as_slice(),
        )
        .expect("the competing record must decode");
        let second_lock = store
            .hold("fr-held-race")
            .expect("the lock probe must succeed")
            .is_some();
        assert_eq!(
            (
                claim.is_err(),
                session.lock.is_some(),
                second_lock,
                stored.phase,
                stored.senses,
                stored.drafts.len(),
            ),
            (true, true, false, Phase::Generating, String::from("all"), 2,),
            "a failed re-claim released live ownership or rewrote the competing record"
        );
    }
}
