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

    fn step(
        &self,
        card: usize,
        term: &str,
        artifact: Artifact,
        outcome: StepOutcome<'_>,
        costs: ArtifactCosts,
    ) {
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
            term: String::from(term),
            artifact: String::from(artifact.label()),
        };
        match self.store.update(self.id.as_str(), |record| {
            if owned_by(record, pid) {
                let draft = record
                    .drafts
                    .get_mut(card)
                    .ok_or_else(|| anyhow::anyhow!("worker card index {card} escaped the plan"))?;
                if draft.term != term {
                    bail!(
                        "worker card index {card} names '{}' instead of '{term}'",
                        draft.term
                    );
                }
                draft.costs = costs;
                record.progress = Some(progress);
            }
            Ok(())
        }) {
            Ok(fresh) => self.revoked.set(!owned_by(&fresh, pid)),
            Err(error) => self
                .inner
                .warn(format!("worker: failed to persist progress: {error:#}").as_str()),
        }
        self.inner.step(card, term, artifact, outcome, costs);
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
            Ok(CardDraft::new(
                draft.term.as_str(),
                draft.understanding.as_str(),
                pair.clone(),
            )
            .with_costs(costs))
        })
        .collect()
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
    let log = File::create(store.log_path(id))?;
    let exe = std::env::current_exe()?;
    let record = store.update(id, |record| {
        record.worker = None;
        record.phase = Phase::Generating;
        record.progress = None;
        record.result = None;
        record.error = None;
        Ok(())
    })?;
    if let Err(error) = spawn_detached(exe, id, log) {
        let _ = store.update(id, |record| {
            record.phase = Phase::Failed;
            record.error = Some(format!("failed to start worker: {error:#}"));
            Ok(())
        });
        return Err(error);
    }
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
        reporter(&store, 1).step(
            0,
            "canard",
            Artifact::Scene,
            StepOutcome::Ready { cached: false },
            ArtifactCosts::default(),
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
    fn each_step_persists_the_sessions_current_artifact_costs() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let costs = ArtifactCosts::default().charged(
            Artifact::Picture,
            crate::session::GenerationCost::from_nanos(420_000_000),
        );
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
            "canard",
            Artifact::Picture,
            StepOutcome::Retry {
                retry: 1,
                retries: 3,
                fault: None,
            },
            costs,
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
    fn a_step_after_the_session_stops_naming_this_worker_flags_revocation() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        generating_session(&store, 1);
        let revoked = reporter(&store, 2);
        revoked.step(
            0,
            "canard",
            Artifact::Scene,
            StepOutcome::Ready { cached: false },
            ArtifactCosts::default(),
        );
        assert!(
            revoked.revoked(),
            "a step against a session naming another worker must flag this run revoked"
        );
    }

    #[test]
    fn claiming_a_cancelled_session_is_refused() {
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
        assert!(
            claim_self(&store, "fr-1").is_err(),
            "a worker claiming a session cancelled before its start must be refused"
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
