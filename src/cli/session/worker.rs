//! The generation worker: drives `produce()`, persisting progress to the session
//! file, plus the detached-spawn plumbing and the hidden `__run` entrypoint.
//!
//! Both the background worker (`__run`, spawned detached) and the foreground
//! `generate --wait` path funnel through [`execute`], so they generate
//! identically; only the streaming reporter differs.

use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use super::liveness;
use super::store::{Phase, Progress, ResultRecord, SessionRecord, SessionStore, WorkerHandle, now};
use crate::cli::console::{Outcome, QuietReporter, Reporter, StepOutcome, generator, produce};
use crate::session::{Artifact, CardDraft, LanguagePair};

/// A reporter that mirrors `produce()` progress into the session file and also
/// forwards every event to an inner reporter (quiet for the detached worker,
/// human for `--wait`).
struct SessionReporter {
    store: SessionStore,
    record: RefCell<SessionRecord>,
    inner: Box<dyn Reporter>,
    final_error: RefCell<Option<String>>,
}

impl SessionReporter {
    fn new(store: SessionStore, record: SessionRecord, inner: Box<dyn Reporter>) -> Self {
        Self {
            store,
            record: RefCell::new(record),
            inner,
            final_error: RefCell::new(None),
        }
    }

    fn persist(&self) {
        let _ = self.store.save(&mut self.record.borrow_mut());
    }

    /// Persist a terminal state, surfacing a write failure loudly (to the worker
    /// log or `--wait` stderr) rather than silently leaving the session stale.
    fn persist_final(&self) {
        if let Err(error) = self.store.save(&mut self.record.borrow_mut()) {
            eprintln!("worker: failed to persist final session state: {error:#}");
        }
    }

    fn fail(&self, message: String) {
        {
            let mut record = self.record.borrow_mut();
            record.phase = Phase::Failed;
            record.error = Some(message);
            record.progress = None;
            record.worker = None;
        }
        self.persist_final();
    }
}

impl Reporter for SessionReporter {
    fn generating(&self, cards: usize) {
        self.inner.generating(cards);
    }

    fn step(&self, term: &str, artifact: Artifact, outcome: StepOutcome) {
        {
            let mut record = self.record.borrow_mut();
            record.progress = Some(Progress {
                term: String::from(term),
                artifact: String::from(artifact.label()),
            });
        }
        self.persist();
        self.inner.step(term, artifact, outcome);
    }

    fn publishing(&self) {
        self.inner.publishing();
    }

    fn finished(&self, outcome: &Outcome) {
        {
            let mut record = self.record.borrow_mut();
            record.phase = Phase::Published;
            record.result = Some(ResultRecord {
                deck: String::from(outcome.deck()),
                report: String::from(outcome.report()),
                output: String::from(outcome.output()),
            });
            record.progress = None;
            record.worker = None;
        }
        if let Err(error) = self.store.save(&mut self.record.borrow_mut()) {
            *self.final_error.borrow_mut() = Some(format!("{error:#}"));
            return;
        }
        self.inner.finished(outcome);
    }
}

/// Generate and publish one session, persisting state as it goes.
///
/// On success the published result is recorded by `finished`; on failure the
/// session is marked failed and the error is returned for the caller's exit code.
fn execute(store: &SessionStore, id: &str, inner: Box<dyn Reporter>) -> Result<()> {
    let record = store.open(id)?;
    let pair = LanguagePair::new(record.to.as_str(), record.from.as_str());
    let drafts = record
        .drafts
        .iter()
        .map(|draft| {
            CardDraft::new(
                draft.term.as_str(),
                draft.understanding.as_str(),
                pair.clone(),
            )
        })
        .collect::<Vec<_>>();
    let live = generator(PathBuf::from(record.out.clone()))?;
    let reporter = SessionReporter::new(store.clone(), record, inner);
    match produce(&live, drafts, &reporter) {
        Ok(()) => match reporter.final_error.borrow_mut().take() {
            Some(message) => {
                bail!("generated the cards but failed to persist the published state: {message}")
            }
            None => Ok(()),
        },
        Err(error) => {
            reporter.fail(format!("{error:#}"));
            Err(error)
        }
    }
}

/// Claim the session for this process: record our own pid as the worker and
/// mark it generating. Both worker entrypoints claim from inside their own
/// process so the pid on disk always belongs to a process that is really running
/// and no other process writes the record after the worker starts.
fn claim_self(store: &SessionStore, id: &str) -> Result<()> {
    let mut record = store.open(id)?;
    record.worker = Some(WorkerHandle {
        pid: i32::try_from(std::process::id())?,
        started: now()?,
    });
    record.phase = Phase::Generating;
    record.progress = None;
    record.result = None;
    record.error = None;
    store.save(&mut record)
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

/// Run the worker in this process (the `generate --wait` path), recording self as
/// the worker and streaming through `inner`. Propagates failures for the exit code.
pub(super) fn run_foreground(
    store: &SessionStore,
    id: &str,
    inner: Box<dyn Reporter>,
) -> Result<()> {
    let Some(_guard) = liveness::hold(&store.lock_path(id))? else {
        bail!("session '{id}' is already being generated by another worker");
    };
    claim_self(store, id)?;
    execute(store, id, inner)
}

/// Start a detached background worker for one session.
///
/// The Generating phase is written before the worker is spawned, but the worker
/// pid is claimed by the detached process itself (see `claim_self`): the parent
/// never writes the record after spawning, so a fast cache-only worker that
/// finishes and saves `published` first can never be clobbered by a late parent
/// save reverting it to `generating`.
pub(super) fn start_background(store: &SessionStore, mut record: SessionRecord) -> Result<()> {
    let log = File::create(store.log_path(&record.id))?;
    let exe = std::env::current_exe()?;
    record.worker = None;
    record.phase = Phase::Generating;
    record.progress = None;
    record.result = None;
    record.error = None;
    store.save(&mut record)?;
    if let Err(error) = spawn_detached(exe, record.id.as_str(), log) {
        record.phase = Phase::Failed;
        record.error = Some(format!("failed to start worker: {error:#}"));
        let _ = store.save(&mut record);
        return Err(error);
    }
    Ok(())
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

    fn session(store: &SessionStore) -> SessionRecord {
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
        }];
        store.save(&mut record).expect("seed session");
        record
    }

    #[test]
    fn finishing_records_published_and_clears_the_pid() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        let mut record = session(&store);
        record.worker = Some(WorkerHandle {
            pid: 1,
            started: String::from("t"),
        });
        let reporter = SessionReporter::new(store.clone(), record, Box::new(QuietReporter));
        reporter.finished(&Outcome::for_test("/o/d.apkg", "/o/d.pdf", "/o", 1, 0));
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
    fn each_step_persists_progress_to_the_session_file() {
        let home = TempDir::new().expect("tempdir");
        let store = SessionStore::new(home.path());
        let record = session(&store);
        let reporter = SessionReporter::new(store.clone(), record, Box::new(QuietReporter));
        reporter.step(
            "canard",
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
}
