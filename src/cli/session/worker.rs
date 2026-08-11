//! The generation worker: drives `produce()`, persisting progress to the session
//! file, plus the detached-spawn plumbing and the hidden `__run` entrypoint.
//!
//! Both the background worker (`__run`, spawned detached) and the foreground
//! `generate --wait` path funnel through [`execute`], so they generate
//! identically; only the streaming reporter differs. Every write is ownership
//! guarded: the worker mutates the record only while it still names this
//! process ([`owned_by`]), so a `cancel` that raced in revokes the run instead
//! of being clobbered, and the worker stops generating at the next step.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use super::cost_journal::{SessionCostJournal, SessionCostScope};
use super::liveness;
use super::store::{Phase, Progress, ResultRecord, SessionRecord, SessionStore, WorkerHandle, now};
use crate::cli::console::{
    Outcome, QuietReporter, Reporter, StepOutcome, produce, workflow_for_session,
};
use crate::session::{Artifact, ArtifactCosts, CardDraft, LanguagePair};

/// Return whether the record still names this process as the generating worker.
fn owned_by(record: &SessionRecord, pid: i32) -> bool {
    matches!(record.phase, Phase::Generating)
        && record
            .worker
            .as_ref()
            .map(|worker| worker.pid == pid)
            .unwrap_or(false)
}

/// A reporter that mirrors `produce()` progress into the session file and also
/// forwards every event to an inner reporter (quiet for the detached worker,
/// human for `--wait`).
struct SessionReporter {
    store: SessionStore,
    id: String,
    pid: i32,
    inner: Box<dyn Reporter>,
    costs: SessionCostScope,
    revoked: Cell<bool>,
    persist_failure: RefCell<Option<String>>,
}

impl SessionReporter {
    fn new(
        store: SessionStore,
        id: String,
        pid: i32,
        inner: Box<dyn Reporter>,
        costs: SessionCostScope,
    ) -> Self {
        Self {
            store,
            id,
            pid,
            inner,
            costs,
            revoked: Cell::new(false),
            persist_failure: RefCell::new(None),
        }
    }

    /// Record a failed run, unless ownership was lost (never write `failed` over
    /// a phase someone else set, e.g. `cancelled`).
    fn fail(&self, message: String) {
        let pid = self.pid;
        let written = self.store.update(self.id.as_str(), |record| {
            if owned_by(record, pid) {
                record.phase = Phase::Failed;
                record.error = Some(message);
                record.progress = None;
                record.worker = None;
            }
            Ok(())
        });
        if let Err(error) = written {
            self.inner
                .warn(format!("worker: failed to persist final session state: {error:#}").as_str());
        }
    }
}

impl Reporter for SessionReporter {
    fn generating(&self, cards: usize) {
        self.inner.generating(cards);
    }

    fn step(&self, card: usize, settled: &CardDraft, artifact: Artifact, outcome: StepOutcome<'_>) {
        let costs = ArtifactCosts::from_artifacts(settled.artifacts());
        let costs = match self.costs.absolute(card, costs) {
            Ok(costs) => costs,
            Err(error) => {
                self.revoked.set(true);
                *self.persist_failure.borrow_mut() = Some(format!("{error:#}"));
                self.inner.warn(
                    format!("worker: failed to read session cost journal: {error:#}").as_str(),
                );
                return;
            }
        };
        let pid = self.pid;
        let progress = Progress {
            term: String::from(settled.term()),
            artifact: String::from(artifact.label()),
        };
        match self.store.update(self.id.as_str(), |record| {
            if owned_by(record, pid) {
                let draft = record
                    .drafts
                    .get_mut(card)
                    .ok_or_else(|| anyhow::anyhow!("worker card index {card} escaped the plan"))?;
                let same =
                    draft.term == settled.term() && draft.understanding == settled.understanding();
                let rewritten = artifact == Artifact::Meta
                    && matches!(outcome, StepOutcome::Ready { .. })
                    && draft.rewrite.is_some()
                    && settled.rewrite().is_none();
                if !same && !rewritten {
                    bail!(
                        "worker card index {card} names '{}' instead of '{}'",
                        draft.term,
                        settled.term()
                    );
                }
                draft.term = String::from(settled.term());
                draft.understanding = String::from(settled.understanding());
                draft.costs = costs;
                draft.rewrite = settled.rewrite().cloned();
                draft.meta_request =
                    if artifact == Artifact::Meta && matches!(outcome, StepOutcome::Ready { .. }) {
                        None
                    } else {
                        settled.meta_request().cloned()
                    };
                record.progress = Some(progress);
            }
            Ok(())
        }) {
            Ok(fresh) => self.revoked.set(!owned_by(&fresh, pid)),
            Err(error) => {
                self.revoked.set(true);
                *self.persist_failure.borrow_mut() = Some(format!("{error:#}"));
                self.inner
                    .warn(format!("worker: failed to persist progress: {error:#}").as_str());
            }
        }
        self.inner.step(card, settled, artifact, outcome);
    }

    fn publishing(&self) {
        self.inner.publishing();
    }

    fn finished(&self, outcome: &Outcome) {
        let published = outcome.cards() > 0;
        let pid = self.pid;
        let mut owned = false;
        let written = self.store.update(self.id.as_str(), |record| {
            owned = owned_by(record, pid);
            if !owned {
                return Ok(());
            }
            record.phase = terminal_phase(outcome);
            record.result = published.then(|| ResultRecord {
                deck: String::from(outcome.deck()),
                report: String::from(outcome.report()),
                output: String::from(outcome.output()),
                cards: outcome.cards(),
                failed: outcome.failed(),
            });
            record.error = (!published)
                .then(|| format!("all {} card(s) failed to generate", outcome.failed()));
            record.progress = None;
            record.worker = None;
            Ok(())
        });
        if let Err(error) = written {
            *self.persist_failure.borrow_mut() = Some(format!("{error:#}"));
            return;
        }
        if !owned {
            self.inner
                .warn("worker: outcome not recorded: the session no longer names this worker");
            return;
        }
        if published {
            self.inner.finished(outcome);
        }
    }

    fn warn(&self, message: &str) {
        self.inner.warn(message);
    }

    fn revoked(&self) -> bool {
        self.revoked.get()
    }
}

/// Map a finished run to its terminal phase: a clean run is `Published`, a run
/// with surviving cards plus failures is `Partial`, and a run that produced no
/// card at all is `Failed`.
fn terminal_phase(outcome: &Outcome) -> Phase {
    if outcome.failed() == 0 {
        Phase::Published
    } else if outcome.cards() == 0 {
        Phase::Failed
    } else {
        Phase::Partial
    }
}

/// Generate and publish one session, persisting state as it goes. Returns the
/// terminal record, read while the caller still holds the liveness lock, so a
/// JSON render can print it without racing a concurrent `rm` or `generate`.
///
/// On success the published result is recorded by `finished`; on failure the
/// session is marked failed — unless the run was revoked by a `cancel`, whose
/// phase is never overwritten — and the error is returned for the exit code.
fn execute(store: &SessionStore, id: &str, inner: Box<dyn Reporter>) -> Result<SessionRecord> {
    let record = store.open(id)?;
    let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
    let journal = store.cost_journal(&record);
    let costs = SessionCostScope::bound(journal.clone());
    let drafts = drafts_with_costs(&record, &pair, &journal)?;
    ensure_rewrites_started(drafts.as_slice())?;
    let workflow = workflow_for_session(PathBuf::from(record.out), costs.clone())?;
    let pid = i32::try_from(std::process::id())?;
    let reporter = SessionReporter::new(store.clone(), String::from(id), pid, inner, costs);
    match produce(&workflow, drafts, &reporter) {
        Ok(()) => match reporter.persist_failure.borrow_mut().take() {
            Some(message) => {
                bail!("generated the cards but failed to persist the published state: {message}")
            }
            None => {
                let saved = store.open(id)?;
                if matches!(saved.phase, Phase::Failed) {
                    bail!(
                        "{}",
                        saved
                            .error
                            .as_deref()
                            .unwrap_or("generation failed: no cards were produced")
                    );
                }
                Ok(saved)
            }
        },
        Err(error) => {
            if !reporter.revoked.get() {
                reporter.fail(format!("{error:#}"));
            }
            Err(error)
        }
    }
}

fn drafts_with_costs(
    record: &SessionRecord,
    pair: &LanguagePair,
    journal: &SessionCostJournal,
) -> Result<Vec<CardDraft>> {
    let fallback = record
        .drafts
        .iter()
        .map(|draft| draft.costs)
        .collect::<Vec<_>>();
    let absolute = journal.overlay(fallback.as_slice())?;
    record
        .drafts
        .iter()
        .zip(absolute)
        .map(|(draft, costs)| {
            let hydrated = CardDraft::new(
                draft.term.as_str(),
                draft.understanding.as_str(),
                pair.clone(),
            );
            let hydrated = match &draft.meta_request {
                Some(selection) => hydrated.requesting_meta(selection.clone()),
                None => hydrated,
            };
            Ok(hydrated
                .with_rewrite(draft.rewrite.clone())
                .with_costs(costs))
        })
        .collect()
}

fn ensure_rewrites_started(drafts: &[CardDraft]) -> Result<()> {
    if drafts.iter().any(|draft| draft.staged_rewrite().is_some()) {
        bail!("staged card rewrites require Ctrl+G before worker generation");
    }
    Ok(())
}

fn ensure_record_rewrites_started(record: &SessionRecord) -> Result<()> {
    if record.drafts.iter().any(|draft| {
        draft
            .rewrite
            .as_ref()
            .is_some_and(|rewrite| !rewrite.started())
    }) {
        bail!("staged card rewrites require Ctrl+G before worker generation");
    }
    Ok(())
}

/// Claim the session for this process: record our own pid as the worker and
/// mark it generating. Both worker entrypoints claim from inside their own
/// process so the pid on disk always belongs to a process that is really running
/// and no other process writes the record while the worker owns it. A cancel
/// that landed before the claim wins: the claim is refused and the worker exits
/// without generating.
fn claim_self(store: &SessionStore, id: &str) -> Result<()> {
    let pid = i32::try_from(std::process::id())?;
    let started = now()?;
    store.update(id, |record| {
        ensure_record_rewrites_started(record)?;
        if matches!(record.phase, Phase::Cancelled) {
            bail!("session '{id}' was cancelled before generation started");
        }
        record.worker = Some(WorkerHandle { pid, started });
        record.phase = Phase::Generating;
        record.progress = None;
        record.result = None;
        record.error = None;
        Ok(())
    })?;
    Ok(())
}

/// The hidden `__run <id>` entrypoint: claim the session under this detached
/// process's own pid, then generate in its foreground, recording final state.
/// Always exits cleanly — the outcome lives in the session file, the error (if
/// any) in the worker log.
pub(super) fn run_detached_entry(id: &str) -> Result<()> {
    let store = SessionStore::system()?;
    let _guard = match liveness::hold(&store.lock_path(id)) {
        Ok(Some(file)) => file,
        Ok(None) => {
            eprintln!("worker: session '{id}' is already locked by another worker");
            return Ok(());
        }
        Err(error) => {
            eprintln!("worker: cannot lock session '{id}': {error:#}");
            return Ok(());
        }
    };
    let claimed =
        claim_self(&store, id).and_then(|()| execute(&store, id, Box::new(QuietReporter)));
    if let Err(error) = claimed {
        eprintln!("worker failed: {error:#}");
    }
    Ok(())
}

/// Run the worker in this process (the `generate --wait` path), recording self
/// as the worker and streaming through `inner`. Returns the terminal record
/// read under the still-held liveness lock; propagates failures for the exit code.
pub(super) fn run_foreground(
    store: &SessionStore,
    id: &str,
    inner: Box<dyn Reporter>,
) -> Result<SessionRecord> {
    let Some(_guard) = liveness::hold(&store.lock_path(id))? else {
        bail!("session '{id}' is already being generated by another worker");
    };
    claim_self(store, id)?;
    execute(store, id, inner)
}

/// Start a detached background worker for one session. Returns the record as
/// this function wrote it (phase `Generating`, no worker yet), so a JSON render
/// prints a deterministic document instead of racing the fresh worker's writes.
///
/// The Generating phase is written before the worker is spawned, but the worker
/// pid is claimed by the detached process itself (see `claim_self`): the parent
/// never writes the record after spawning, so a fast cache-only worker that
/// finishes and saves `published` first can never be clobbered by a late parent
/// save reverting it to `generating`.
pub(super) fn start_background(store: &SessionStore, id: &str) -> Result<SessionRecord> {
    ensure_record_rewrites_started(&store.open(id)?)?;
    let log = File::create(store.log_path(id))?;
    let exe = std::env::current_exe()?;
    let record = prepare_background(store, id)?;
    if let Err(error) = spawn_detached(exe, id, log) {
        let _ = store.update(id, |record| {
            if !matches!(record.phase, Phase::Cancelled) {
                record.phase = Phase::Failed;
                record.error = Some(format!("failed to start worker: {error:#}"));
            }
            Ok(())
        });
        return Err(error);
    }
    Ok(record)
}

fn prepare_background(store: &SessionStore, id: &str) -> Result<SessionRecord> {
    let record = store.update(id, |record| {
        ensure_record_rewrites_started(record)?;
        if matches!(record.phase, Phase::Cancelled) {
            bail!("session '{id}' was cancelled before generation started");
        }
        record.worker = None;
        record.phase = Phase::Generating;
        record.progress = None;
        record.result = None;
        record.error = None;
        Ok(())
    })?;
    Ok(record)
}

#[cfg(unix)]
fn spawn_detached(exe: PathBuf, id: &str, log: File) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    let err = log.try_clone()?;
    let child = Command::new(exe)
        .arg("__run")
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .process_group(0)
        .spawn()?;
    Ok(i32::try_from(child.id())?)
}

#[cfg(not(unix))]
fn spawn_detached(exe: PathBuf, id: &str, log: File) -> Result<i32> {
    let err = log.try_clone()?;
    let child = Command::new(exe)
        .arg("__run")
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()?;
    Ok(i32::try_from(child.id())?)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::super::store::DraftRecord;
    use super::*;

    fn generating_session(store: &SessionStore, pid: i32) -> SessionRecord {
        let mut record = SessionRecord::understood(
            String::from("fr-1"),
            String::from("2026-06-06T00:00:00Z"),
            String::from("en"),
            String::from("fr"),
            String::from("/out"),
            String::from("primary"),
            String::from("words"),
            vec![String::from("canard")],
            Vec::new(),
        );
        record.drafts = vec![DraftRecord {
            term: String::from("canard"),
            understanding: String::from("a duck"),
            costs: crate::session::ArtifactCosts::default(),
            rewrite: None,
            meta_request: None,
        }];
        record.phase = Phase::Generating;
        record.worker = Some(WorkerHandle {
            pid,
            started: String::from("t"),
        });
        store.create(&record).expect("seed session");
        record
    }

    fn reporter(store: &SessionStore, pid: i32) -> SessionReporter {
        let record = store.open("fr-1").expect("session must open");
        let journal = store.cost_journal(&record);
        journal
            .seed(
                record
                    .drafts
                    .iter()
                    .map(|draft| draft.costs)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .expect("session costs must seed");
        SessionReporter::new(
            store.clone(),
            String::from("fr-1"),
            pid,
            Box::new(QuietReporter),
            SessionCostScope::bound(journal),
        )
    }

    #[test]
    fn finishing_records_published_and_clears_the_pid() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        reporter(&store, 1).finished(&Outcome::for_test("/o/d.apkg", "/o/d.pdf", "/o", 1, 0));
        let reopened = store.open("fr-1").expect("reopen");
        assert_eq!(
            (
                reopened.phase,
                reopened.worker.is_none(),
                reopened.result.is_some()
            ),
            (Phase::Published, true, true),
            "finishing must record published, clear the pid, and store the result"
        );
    }

    #[test]
    fn a_run_with_some_failures_records_partial_and_keeps_the_deck() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        reporter(&store, 1).finished(&Outcome::for_test("/o/d.apkg", "/o/d.pdf", "/o", 1, 1));
        let reopened = store.open("fr-1").expect("reopen");
        assert_eq!(
            (reopened.phase, reopened.result.map(|result| result.failed)),
            (Phase::Partial, Some(1)),
            "a run that published some cards and failed others must record partial with the failed count"
        );
    }

    #[test]
    fn a_run_with_no_surviving_cards_records_failed_without_a_result() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        reporter(&store, 1).finished(&Outcome::for_test("/o/d.apkg", "/o/d.pdf", "/o", 0, 2));
        let reopened = store.open("fr-1").expect("reopen");
        assert_eq!(
            (reopened.phase, reopened.result.is_none()),
            (Phase::Failed, true),
            "a run where every card failed must record failed and store no published result"
        );
    }

    #[test]
    fn each_step_persists_progress_to_the_session_file() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        reporter(&store, 1).step(
            0,
            &draft,
            Artifact::Scene,
            StepOutcome::Ready { cached: false },
        );
        assert_eq!(
            store
                .open("fr-1")
                .expect("reopen")
                .progress
                .map(|progress| (progress.term, progress.artifact)),
            Some((String::from("canard"), String::from("scene"))),
            "each artifact step must persist the current term and artifact to disk"
        );
    }

    #[test]
    fn successful_rewrite_meta_persists_the_new_identity_and_clears_the_request() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let previous = crate::session::CardMeta::new(
            "/canard/",
            "/canard/",
            "a duck",
            5,
            "The duck swims.",
            "duck",
            "water bird",
            "animals",
            "Le canard nage.",
        );
        store
            .update("fr-1", |record| {
                record.drafts[0].rewrite = Some(crate::session::CardRewrite::new(
                    Some(previous),
                    crate::session::SentenceLabelSelection::default(),
                    "use the newspaper sense",
                ));
                Ok(())
            })
            .expect("rewrite must queue");
        let settled = CardDraft::new(
            "canard",
            "a false newspaper story",
            LanguagePair::new("fr", "en"),
        )
        .with_meta(
            crate::session::CardMeta::new(
                "/canard/",
                "/canard/",
                "a hoax",
                7,
                "The newspaper ran a hoax.",
                "hoax",
                "a false story",
                "journalism",
                "Ce canard a trompé tout le monde.",
            ),
            None,
        );
        reporter(&store, 1).step(
            0,
            &settled,
            Artifact::Meta,
            StepOutcome::Ready { cached: false },
        );
        let saved = store.open("fr-1").expect("session must reopen");
        assert_eq!(
            (
                saved.drafts[0].term.as_str(),
                saved.drafts[0].understanding.as_str(),
                saved.drafts[0].rewrite.is_none(),
            ),
            ("canard", "a false newspaper story", true),
            "successful rewrite metadata did not replace the durable draft identity"
        );
    }

    #[test]
    fn each_step_persists_the_sessions_current_artifact_costs() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let costs = ArtifactCosts::default().charged(
            Artifact::Picture,
            crate::session::GenerationCost::from_nanos(420_000_000),
        );
        let draft =
            CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en")).with_costs(costs);
        let record = store.open("fr-1").expect("session must open");
        store
            .cost_journal(&record)
            .charge(
                0,
                Artifact::Picture,
                crate::session::GenerationCost::from_nanos(420_000_000),
            )
            .expect("provider observer must journal spend before progress");
        reporter(&store, 1).step(
            0,
            &draft,
            Artifact::Picture,
            StepOutcome::Retry {
                retry: 1,
                retries: 3,
                fault: None,
            },
        );
        assert_eq!(
            store.open("fr-1").expect("reopen").drafts[0]
                .costs
                .cost(Artifact::Picture),
            Some(crate::session::GenerationCost::from_nanos(420_000_000)),
            "the worker persisted progress but lost the session-scoped provider spend"
        );
    }

    #[test]
    fn a_restarted_worker_uses_journal_totals_over_stale_draft_costs() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let record = generating_session(&store, 1);
        let journal = store.cost_journal(&record);
        journal
            .seed(
                record
                    .drafts
                    .iter()
                    .map(|draft| draft.costs)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .expect("journal must seed");
        journal
            .charge(
                0,
                Artifact::Picture,
                crate::session::GenerationCost::from_nanos(730_000_000),
            )
            .expect("hard-crash spend must persist");
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        assert_eq!(
            drafts_with_costs(&record, &pair, &journal).expect("worker drafts must hydrate")[0]
                .artifacts()
                .picture()
                .cost(),
            Some(crate::session::GenerationCost::from_nanos(730_000_000)),
            "worker restart trusted stale DraftRecord costs instead of the provider-boundary journal"
        );
    }

    #[test]
    fn a_restarted_worker_restores_the_pending_initial_meta_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut record = generating_session(&store, 1);
        let request = crate::session::SentenceLabelSelection::empty()
            .choosing(crate::session::SentenceAxis::Level, 2);
        record.drafts[0].meta_request = Some(request.clone());
        let journal = store.cost_journal(&record);
        journal
            .seed(
                record
                    .drafts
                    .iter()
                    .map(|draft| draft.costs)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .expect("journal must seed");
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        assert_eq!(
            drafts_with_costs(&record, &pair, &journal).expect("worker drafts must hydrate")[0]
                .meta_request(),
            Some(&request),
            "worker restart discarded an initial metadata request that had not completed"
        );
    }

    #[test]
    fn a_restarted_worker_prefers_a_rewrite_over_a_corrupt_initial_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut record = generating_session(&store, 1);
        record.drafts[0].meta_request = Some(
            crate::session::SentenceLabelSelection::empty()
                .choosing(crate::session::SentenceAxis::Level, 2),
        );
        record.drafts[0].rewrite = Some(crate::session::CardRewrite::new(
            None,
            crate::session::SentenceLabelSelection::empty()
                .choosing(crate::session::SentenceAxis::Register, 2),
            "",
        ));
        let journal = store.cost_journal(&record);
        journal
            .seed(
                record
                    .drafts
                    .iter()
                    .map(|draft| draft.costs)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .expect("journal must seed");
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        let hydrated = drafts_with_costs(&record, &pair, &journal)
            .expect("worker drafts must hydrate")
            .remove(0);
        assert_eq!(
            (hydrated.meta_request(), hydrated.rewrite().is_some()),
            (None, true),
            "worker hydration let a corrupt initial request override a durable rewrite"
        );
    }

    #[test]
    fn a_successful_meta_step_clears_the_durable_initial_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let request = crate::session::SentenceLabelSelection::empty()
            .choosing(crate::session::SentenceAxis::Level, 2);
        store
            .update("fr-1", |record| {
                record.drafts[0].meta_request = Some(request.clone());
                Ok(())
            })
            .expect("initial request must persist");
        let settled = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"))
            .requesting_meta(request);
        reporter(&store, 1).step(
            0,
            &settled,
            Artifact::Meta,
            StepOutcome::Ready { cached: false },
        );
        assert_eq!(
            store.open("fr-1").expect("reopen").drafts[0].meta_request,
            None,
            "a successful metadata step left the initial request pending on disk"
        );
    }

    #[test]
    fn a_worker_refuses_a_persisted_staged_rewrite_before_production() {
        let home = TempDir::new().expect("tempdir must be created");
        let store = SessionStore::new(home.path());
        let mut record = generating_session(&store, 1);
        let staged = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"))
            .staging_rewrite(
                crate::session::SentenceLabelSelection::empty(),
                "make it formal",
            );
        record.drafts[0].rewrite = staged.rewrite().cloned();
        let journal = store.cost_journal(&record);
        journal
            .seed(
                record
                    .drafts
                    .iter()
                    .map(|draft| draft.costs)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .expect("journal must seed");
        let pair = LanguagePair::new(record.learning.as_str(), record.known.as_str());
        let draft = drafts_with_costs(&record, &pair, &journal)
            .expect("worker drafts must hydrate")
            .remove(0);
        assert_eq!(
            (
                draft.rewrite().map(crate::session::CardRewrite::started),
                ensure_rewrites_started(&[draft]).is_err(),
            ),
            (Some(false), true),
            "worker production activated or accepted a rewrite that Ctrl+G never started"
        );
    }

    #[test]
    fn a_staged_edit_between_preflight_and_claim_cannot_start_worker_or_provider_work() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        store
            .update("fr-1", |record| {
                record.phase = Phase::Published;
                record.worker = None;
                record.result = Some(ResultRecord {
                    deck: String::from("/out/old.apkg"),
                    report: String::from("/out/old.pdf"),
                    output: String::from("/out"),
                    cards: 1,
                    failed: 0,
                });
                Ok(())
            })
            .expect("published session must persist");
        ensure_record_rewrites_started(&store.open("fr-1").expect("preflight record must open"))
            .expect("initial preflight must pass");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let concurrent = store.clone();
        let released = barrier.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            released.wait();
            let staged = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"))
                .staging_rewrite(
                    crate::session::SentenceLabelSelection::empty(),
                    "make it formal",
                );
            let result = concurrent.update("fr-1", |record| {
                record.drafts[0].rewrite = staged.rewrite().cloned();
                Ok(())
            });
            let _ = sender.send(result);
        });
        barrier.wait();
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("concurrent staged edit must finish before its deadline")
            .expect("concurrent staged edit must persist");
        writer.join().expect("concurrent editor must exit");
        let inserted = store.open("fr-1").expect("staged record must open");
        let provider_calls = std::sync::atomic::AtomicUsize::new(0);
        let background = prepare_background(&store, "fr-1");
        if background.is_ok() {
            provider_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let claimed = claim_self(&store, "fr-1");
        if claimed.is_ok() {
            provider_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let after = store.open("fr-1").expect("refused record must open");
        assert_eq!(
            (
                background.is_err(),
                claimed.is_err(),
                provider_calls.load(std::sync::atomic::Ordering::SeqCst),
                after.phase,
                after.worker.is_none(),
                after.result.as_ref().map(|result| {
                    (
                        result.deck.as_str(),
                        result.report.as_str(),
                        result.output.as_str(),
                    )
                }),
                after.drafts[0]
                    .rewrite
                    .as_ref()
                    .map(crate::session::CardRewrite::started),
                after == inserted,
            ),
            (
                true,
                true,
                0,
                Phase::Published,
                true,
                Some(("/out/old.apkg", "/out/old.pdf", "/out")),
                Some(false),
                true,
            ),
            "a staged race crossed worker claim or mutated the published session"
        );
    }

    #[test]
    fn a_step_after_the_session_stops_naming_this_worker_flags_revocation() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let revoked = reporter(&store, 2);
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        revoked.step(
            0,
            &draft,
            Artifact::Scene,
            StepOutcome::Ready { cached: false },
        );
        assert!(
            revoked.revoked(),
            "a step against a session naming another worker must flag this run revoked"
        );
    }

    #[test]
    fn preparing_or_claiming_a_cancelled_session_is_refused() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        store
            .update("fr-1", |record| {
                record.phase = Phase::Cancelled;
                record.worker = None;
                Ok(())
            })
            .expect("cancel the session");
        let prepared = prepare_background(&store, "fr-1");
        let claimed = claim_self(&store, "fr-1");
        assert!(
            prepared.is_err() && claimed.is_err(),
            "a worker prepared or claimed a session cancelled before its start"
        );
    }

    #[test]
    fn a_finish_after_revocation_never_overwrites_the_cancelled_phase() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        store
            .update("fr-1", |record| {
                record.phase = Phase::Cancelled;
                record.worker = None;
                Ok(())
            })
            .expect("cancel the session");
        reporter(&store, 1).finished(&Outcome::for_test("/o/d.apkg", "/o/d.pdf", "/o", 1, 0));
        assert_eq!(
            store.open("fr-1").expect("reopen").phase,
            Phase::Cancelled,
            "a worker finishing after a cancel must not overwrite the cancelled phase"
        );
    }
}
