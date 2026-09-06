//! Non-interactive console flow: the same understand → generate → publish path
//! the TUI walks, scripted for agents and pipes.
//!
//! The driver reuses the real flow components — the `Understanding` pass, the
//! `SessionEngine` artifact queue, and the `StudyPublishing` step — so a console
//! run produces exactly what the interactive run would. Only the rendering
//! differs: progress is reported through a [`Reporter`] port with human,
//! quiet, and NDJSON-on-stderr implementations.

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::application::{CardProduction, PublishPhase, PublishProgress, StudyPublishing};

use super::session::SessionCostScope;
use super::wiring::{GeminiCardWorkflow, console_workflow, session_workflow};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactSlot, AttemptFault, CardArtifacts, CardDraft, LanguagePair, SessionEngine,
    WordCandidate,
};

use std::path::PathBuf;

/// How many senses of each word become cards in a non-interactive run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(super) enum SensePolicy {
    /// One card per word, using the model's primary sense (the TUI default).
    Primary,
    /// One card for every sense the model proposed.
    All,
}

/// The final artifacts a console run produced.
pub(super) struct Outcome {
    deck: String,
    report: String,
    output: String,
    cards: usize,
    failed: usize,
}

impl Outcome {
    /// Return the path of the published Anki deck.
    pub(super) fn deck(&self) -> &str {
        self.deck.as_str()
    }

    /// Return the path of the published PDF report.
    pub(super) fn report(&self) -> &str {
        self.report.as_str()
    }

    /// Return the output directory holding the deck and report.
    pub(super) fn output(&self) -> &str {
        self.output.as_str()
    }

    /// Return how many fully generated cards made it into the deck.
    pub(super) fn cards(&self) -> usize {
        self.cards
    }

    /// Return how many cards failed and were left out of the deck.
    pub(super) fn failed(&self) -> usize {
        self.failed
    }
}

#[cfg(test)]
impl Outcome {
    /// Build an outcome for tests in sibling modules (no published files).
    pub(super) fn for_test(
        deck: &str,
        report: &str,
        output: &str,
        cards: usize,
        failed: usize,
    ) -> Self {
        Self {
            deck: String::from(deck),
            report: String::from(report),
            output: String::from(output),
            cards,
            failed,
        }
    }
}

/// Per-artifact result reported as one step advances.
///
/// A failed step always carries the fault of the attempt it just spent, so
/// both renderings can name why the picture (or any other artifact) was
/// rejected instead of printing a bare counter. `retry` numbers the retry that
/// the reported failure has just triggered, matching what the TUI step row
/// shows at that same moment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepOutcome<'a> {
    /// The artifact is ready, optionally served straight from cache.
    Ready { cached: bool },
    /// An attempt failed and retry number `retry` of `retries` will follow.
    Retry {
        retry: u8,
        retries: u8,
        fault: Option<&'a AttemptFault>,
    },
    /// The artifact exhausted its retries and was abandoned after `retries`.
    Failed {
        retries: u8,
        fault: Option<&'a AttemptFault>,
    },
}

/// Progress sink for one console run. The flow is identical to the TUI; only the
/// rendering differs between implementations.
pub(super) trait Reporter {
    /// Announce that generation started for `cards` cards.
    fn generating(&self, cards: usize);
    /// Report one artifact step for one card.
    fn step(&self, card: usize, draft: &CardDraft, artifact: Artifact, outcome: StepOutcome<'_>);
    /// Announce that the deck and report are being written.
    fn publishing(&self);
    /// Report the final artifacts once the run completes.
    fn finished(&self, outcome: &Outcome);
    /// Surface one out-of-band warning (persistence hiccups, lost ownership);
    /// the default writes it straight to stderr.
    fn warn(&self, message: &str) {
        eprintln!("{message}");
    }
    /// Return whether the run lost ownership of its session and must stop
    /// (a cancel raced in); the default never revokes.
    fn revoked(&self) -> bool {
        false
    }
}

/// Publish-progress sink for runs nobody watches phase-by-phase: the console
/// reporter already announces publishing once, so phase changes are dropped.
struct Unwatched;

impl PublishProgress for Unwatched {
    fn advance(&self, _phase: PublishPhase) {}
}

/// Build a console Gemini workflow rooted at the shared cache and output dir.
pub(super) fn workflow(output: PathBuf) -> Result<GeminiCardWorkflow> {
    let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
    Ok(console_workflow(cache, output))
}

/// Build a console workflow whose observed spend belongs to one session run.
pub(super) fn workflow_for_session(
    output: PathBuf,
    costs: SessionCostScope,
) -> Result<GeminiCardWorkflow> {
    let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
    Ok(session_workflow(cache, output, costs))
}

/// Drive the missing artifacts for `drafts` to completion, then publish the deck.
///
/// Reuses the engine order (meta → sound → scene → picture) with per-artifact
/// retries; artifacts already present in the cache are cheap hits, so this is
/// idempotent and resumable. Progress and the final paths flow through `reporter`.
pub(super) fn produce<G>(
    workflow: &G,
    drafts: Vec<CardDraft>,
    reporter: &dyn Reporter,
) -> Result<()>
where
    G: CardProduction + StudyPublishing,
{
    reporter.generating(drafts.len());
    let mut engine = SessionEngine::start(drafts);
    while let Some((card, artifact)) = engine.next_target() {
        if reporter.revoked() {
            bail!("the session no longer names this worker");
        }
        let draft = engine.drafts()[card].clone();
        let term = draft.term().to_string();
        if let Some(error) = advance(workflow, &mut engine, card, artifact, &draft) {
            reporter.warn(format!("{term} · {}: {error}", artifact.label()).as_str());
        }
        reporter.step(
            card,
            &engine.drafts()[card],
            artifact,
            outcome_of(&engine, card, artifact),
        );
    }
    let drafts = engine.drafts().to_vec();
    let cards = drafts.iter().filter(|d| d.artifacts().all_ready()).count();
    let failed = drafts.iter().filter(|d| d.artifacts().has_failed()).count();
    if reporter.revoked() {
        bail!("the session no longer names this worker");
    }
    reporter.publishing();
    let publication = workflow.publish(&drafts, &Unwatched);
    let (deck, report, output) = if cards > 0 {
        publication.context("could not save your cards")?
    } else {
        publication?
    }
    .into_paths();
    let outcome = Outcome {
        deck,
        report,
        output,
        cards,
        failed,
    };
    reporter.finished(&outcome);
    Ok(())
}

fn advance<G>(
    workflow: &G,
    engine: &mut SessionEngine,
    card: usize,
    artifact: Artifact,
    draft: &CardDraft,
) -> Option<String>
where
    G: CardProduction + StudyPublishing,
{
    match artifact {
        Artifact::Meta => {
            let attempt = workflow.generate_draft_meta_in(card, draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_revision_attempt(card, attempt);
            error
        }
        Artifact::Scene => {
            let attempt = workflow.generate_scene_in(card, draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
        Artifact::Picture => {
            let attempt = workflow.generate_picture_in(card, draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
        Artifact::Sound => {
            let attempt = workflow.generate_sound_in(card, draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
    }
}

fn outcome_of(engine: &SessionEngine, card: usize, artifact: Artifact) -> StepOutcome<'_> {
    let slot = slot_for(engine.drafts()[card].artifacts(), artifact);
    if slot.ready() {
        return StepOutcome::Ready {
            cached: slot.file().map(|file| file.cached()).unwrap_or(false),
        };
    }
    if slot.failed_terminally() {
        return StepOutcome::Failed {
            retries: slot.tally().retries(),
            fault: slot.latest_fault(),
        };
    }
    StepOutcome::Retry {
        retry: slot.tally().done(),
        retries: slot.tally().retries(),
        fault: slot.latest_fault(),
    }
}

fn slot_for(artifacts: &CardArtifacts, artifact: Artifact) -> &ArtifactSlot {
    match artifact {
        Artifact::Meta => artifacts.meta(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    }
}

/// Turn reviewed candidates into card drafts from their stored sense selection,
/// dropping the rows the model marked not-ok. The `--senses` policy is applied
/// once at `new`-time as the initial selection; this only reads the selection.
pub(super) fn drafts_for(candidates: &[WordCandidate], pair: &LanguagePair) -> Vec<CardDraft> {
    candidates
        .iter()
        .filter(|candidate| candidate.ok())
        .flat_map(|candidate| {
            let indices: Vec<usize> = candidate.selected_senses().to_vec();
            indices.into_iter().filter_map(move |index| {
                candidate
                    .senses()
                    .get(index)
                    .map(|_| CardDraft::from_candidate(candidate, index, pair.clone()))
            })
        })
        .collect()
}

/// Reporter that narrates progress to stderr and prints final paths to stdout.
pub(super) struct HumanReporter;

impl Reporter for HumanReporter {
    fn generating(&self, _cards: usize) {}

    fn step(&self, _card: usize, draft: &CardDraft, artifact: Artifact, outcome: StepOutcome<'_>) {
        let term = draft.term();
        let label = human_label(artifact);
        match outcome {
            StepOutcome::Ready { cached: true } => eprintln!("  {term} · {label} (cached)"),
            StepOutcome::Ready { cached: false } => eprintln!("  {term} · {label} ✓"),
            StepOutcome::Retry {
                retry,
                retries,
                fault,
            } => eprintln!(
                "  {term} · {label} · retry {retry}/{retries}{}",
                because(fault)
            ),
            StepOutcome::Failed { retries, fault } => eprintln!(
                "  {term} · {label} · gave up after {retries} retries{}",
                because(fault)
            ),
        }
    }

    fn publishing(&self) {}

    fn finished(&self, outcome: &Outcome) {
        eprintln!("done:");
        eprintln!("{}", outcome.deck());
        eprintln!("{}", outcome.report());
        eprintln!("{}", outcome.output());
    }
}

/// Render the cause of one spent attempt as a plain-output suffix.
fn because(fault: Option<&AttemptFault>) -> String {
    fault.map_or_else(String::new, |fault| format!(" · {}", fault.reason()))
}

/// The user-facing artifact label for the `--wait` stream: meta reads as
/// "meaning", sound as "audio" (display only — the JSON keys stay meta/sound).
fn human_label(artifact: Artifact) -> &'static str {
    match artifact {
        Artifact::Meta => "meaning",
        Artifact::Sound => "audio",
        Artifact::Scene => "scene",
        Artifact::Picture => "picture",
    }
}

/// Reporter that stays silent and prints only the final paths to stdout.
pub(super) struct QuietReporter;

impl Reporter for QuietReporter {
    fn generating(&self, _cards: usize) {}

    fn step(&self, _card: usize, _draft: &CardDraft, _artifact: Artifact, _outcome: StepOutcome) {}

    fn publishing(&self) {}

    fn finished(&self, outcome: &Outcome) {
        print_paths(outcome);
    }
}

fn print_paths(outcome: &Outcome) {
    println!("{}", outcome.deck);
    println!("{}", outcome.report);
    println!("{}", outcome.output);
}

/// One structured progress event, streamed as a single NDJSON line on stderr.
#[derive(Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Event<'a> {
    Generating {
        cards: usize,
    },
    Step {
        term: &'a str,
        artifact: &'a str,
        status: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        retries: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        category: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        score: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocker: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        penalties: Option<std::collections::BTreeMap<&'static str, u32>>,
    },
    Publishing,
    Warning {
        message: &'a str,
    },
}

fn stream(event: &Event<'_>) {
    eprintln!(
        "{}",
        serde_json::to_string(event).expect("invariant: progress events always serialize")
    );
}

/// Reporter for `--wait --json`: progress streams as NDJSON events on stderr,
/// while stdout stays reserved for the one final session document the caller
/// prints after the run; `finished` is deliberately silent here.
pub(super) struct JsonReporter;

impl Reporter for JsonReporter {
    fn generating(&self, cards: usize) {
        stream(&Event::Generating { cards });
    }

    fn step(&self, _card: usize, draft: &CardDraft, artifact: Artifact, outcome: StepOutcome<'_>) {
        let term = draft.term();
        let (status, retry, retries, fault) = match outcome {
            StepOutcome::Ready { cached: true } => ("cache", None, None, None),
            StepOutcome::Ready { cached: false } => ("ok", None, None, None),
            StepOutcome::Retry {
                retry,
                retries,
                fault,
            } => ("retry", Some(retry), Some(retries), fault),
            StepOutcome::Failed { retries, fault } => ("fail", None, Some(retries), fault),
        };
        let scorecard = fault.and_then(AttemptFault::scorecard);
        stream(&Event::Step {
            term,
            artifact: artifact.label(),
            status,
            retry,
            retries,
            category: fault.map(AttemptFault::category),
            reason: fault.map(AttemptFault::reason),
            score: scorecard.map(|card| card.score()),
            blocker: scorecard.map(|card| card.blocker()),
            penalties: scorecard.map(|card| card.penalties().each().into_iter().collect()),
        });
    }

    fn publishing(&self) {
        stream(&Event::Publishing);
    }

    fn finished(&self, _outcome: &Outcome) {}

    fn warn(&self, message: &str) {
        stream(&Event::Warning { message });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use anyhow::Result;

    use super::*;
    use crate::application::{CardCorrection, CardMetaGeneration, PublishedStudyPackage};
    use crate::session::{
        ArtifactAttempt, ArtifactFile, CardMeta, CardRevision, Sense, SentenceLabelSelection,
        WordCandidate,
    };

    #[derive(Clone, Default)]
    struct LocalWorkflow;

    #[derive(Clone, Default)]
    struct FailingPictureWorkflow {
        pictures: Cell<usize>,
    }

    impl CardMetaGeneration for LocalWorkflow {
        fn generate_card_meta(
            &self,
            term: &str,
            understanding: &str,
            _pair: &LanguagePair,
            _request: Option<&SentenceLabelSelection>,
        ) -> Result<CardMeta> {
            Ok(CardMeta::new(
                format!("/{term}/"),
                format!("/{term}/"),
                format!("meaning of {term}"),
                5,
                format!("source for {term} ({understanding})"),
                term,
                format!("cue for {term}"),
                format!("usage of {term}"),
                format!("Example with {term}."),
            ))
        }
    }

    impl CardCorrection for LocalWorkflow {
        fn correct_card(
            &self,
            draft: &CardDraft,
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            let meta =
                self.generate_card_meta(draft.term(), draft.understanding(), draft.pair(), None)?;
            Ok(CardRevision::new(draft.term(), draft.understanding(), meta))
        }
    }

    impl CardProduction for LocalWorkflow {
        fn generate_meta_in(
            &self,
            _slot: usize,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            request: Option<&SentenceLabelSelection>,
        ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
            let result = self
                .generate_card_meta(term, understanding, pair, request)
                .and_then(|meta| {
                    self.store_card_meta(term, understanding, pair, &meta)
                        .map(|file| (meta, Some(file)))
                });
            ArtifactAttempt::unmetered(result)
        }

        fn generate_scene_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(Ok(local_file(draft.term(), "scene")))
        }

        fn generate_picture_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(Ok(local_file(draft.term(), "picture")))
        }

        fn generate_sound_in(
            &self,
            _slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(Ok(local_file(draft.term(), "sound")))
        }

        fn store_card_meta(
            &self,
            _term: &str,
            _understanding: &str,
            _pair: &LanguagePair,
            _meta: &CardMeta,
        ) -> Result<ArtifactFile> {
            Ok(ArtifactFile::new(
                "meta.json",
                std::env::temp_dir().join("meta.json"),
                "1 B",
                false,
            ))
        }
    }

    impl StudyPublishing for LocalWorkflow {
        fn publish(
            &self,
            drafts: &[CardDraft],
            progress: &dyn PublishProgress,
        ) -> Result<PublishedStudyPackage> {
            progress.advance(PublishPhase::Report);
            Ok(PublishedStudyPackage::new(
                format!("/out/deck-{}.apkg", drafts.len()),
                format!("/out/deck-{}.pdf", drafts.len()),
                String::from("/out"),
            ))
        }
    }

    impl CardMetaGeneration for FailingPictureWorkflow {
        fn generate_card_meta(
            &self,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            request: Option<&SentenceLabelSelection>,
        ) -> Result<CardMeta> {
            LocalWorkflow.generate_card_meta(term, understanding, pair, request)
        }
    }

    impl CardCorrection for FailingPictureWorkflow {
        fn correct_card(
            &self,
            draft: &CardDraft,
            comment: &str,
            pair: &LanguagePair,
        ) -> Result<CardRevision> {
            LocalWorkflow.correct_card(draft, comment, pair)
        }
    }

    impl CardProduction for FailingPictureWorkflow {
        fn generate_meta_in(
            &self,
            slot: usize,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            request: Option<&SentenceLabelSelection>,
        ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
            LocalWorkflow.generate_meta_in(slot, term, understanding, pair, request)
        }

        fn generate_scene_in(
            &self,
            slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            LocalWorkflow.generate_scene_in(slot, draft)
        }

        fn generate_picture_in(
            &self,
            _slot: usize,
            _draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            self.pictures.set(self.pictures.get().saturating_add(1));
            ArtifactAttempt::unmetered(Err(anyhow::anyhow!("picture rejected")))
        }

        fn generate_sound_in(
            &self,
            slot: usize,
            draft: &CardDraft,
        ) -> ArtifactAttempt<ArtifactFile> {
            LocalWorkflow.generate_sound_in(slot, draft)
        }

        fn store_card_meta(
            &self,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            meta: &CardMeta,
        ) -> Result<ArtifactFile> {
            LocalWorkflow.store_card_meta(term, understanding, pair, meta)
        }
    }

    impl StudyPublishing for FailingPictureWorkflow {
        fn publish(
            &self,
            drafts: &[CardDraft],
            progress: &dyn PublishProgress,
        ) -> Result<PublishedStudyPackage> {
            LocalWorkflow.publish(drafts, progress)
        }
    }

    fn local_file(term: &str, kind: &str) -> ArtifactFile {
        let name = format!("{term}-{kind}");
        ArtifactFile::new(name.clone(), std::env::temp_dir().join(&name), "1 B", false)
    }

    #[derive(Default)]
    struct RecordingReporter {
        steps: RefCell<Vec<String>>,
        published: RefCell<Option<(usize, usize)>>,
        warnings: RefCell<Vec<String>>,
        faults: RefCell<Vec<String>>,
    }

    impl Reporter for RecordingReporter {
        fn generating(&self, _cards: usize) {}
        fn step(
            &self,
            _card: usize,
            draft: &CardDraft,
            artifact: Artifact,
            outcome: StepOutcome<'_>,
        ) {
            self.steps
                .borrow_mut()
                .push(format!("{}:{}", draft.term(), artifact.label()));
            let fault = match outcome {
                StepOutcome::Retry { fault, .. } | StepOutcome::Failed { fault, .. } => fault,
                StepOutcome::Ready { .. } => None,
            };
            if let Some(fault) = fault {
                self.faults
                    .borrow_mut()
                    .push(format!("{}:{}", fault.category(), fault.reason()));
            }
        }
        fn publishing(&self) {}
        fn finished(&self, outcome: &Outcome) {
            *self.published.borrow_mut() = Some((outcome.cards, outcome.failed));
        }
        fn warn(&self, message: &str) {
            self.warnings.borrow_mut().push(String::from(message));
        }
    }

    fn pair() -> LanguagePair {
        LanguagePair::new("fr", "en")
    }

    #[test]
    fn only_the_selected_sense_becomes_a_card() {
        let candidate = WordCandidate::with_selected_senses(
            "canard",
            vec![Sense::plain("a duck"), Sense::plain("a false report")],
            vec![0],
            true,
        );
        let drafts = drafts_for(&[candidate], &pair());
        assert_eq!(
            drafts.len(),
            1,
            "only the selected sense of a word must become a card"
        );
    }

    #[test]
    fn each_selected_sense_becomes_one_card() {
        let candidate = WordCandidate::with_selected_senses(
            "canard",
            vec![Sense::plain("a duck"), Sense::plain("a false report")],
            vec![0, 1],
            true,
        );
        let drafts = drafts_for(&[candidate], &pair());
        assert_eq!(
            drafts.len(),
            2,
            "every selected sense must become its own card"
        );
    }

    #[test]
    fn every_card_keeps_the_reviewed_senses_with_its_selected_sense_first() {
        let candidate = WordCandidate::with_selected_senses(
            "canard",
            vec![
                Sense::plain("a duck"),
                Sense::tagged("a false report", "journalism"),
                Sense::plain("a newspaper hoax"),
            ],
            vec![1],
            true,
        );
        let drafts = drafts_for(&[candidate], &pair());
        assert_eq!(
            drafts[0]
                .reviewed_senses()
                .iter()
                .map(|sense| (sense.understanding(), sense.tag()))
                .collect::<Vec<_>>(),
            vec![
                ("a false report", Some("journalism")),
                ("a duck", None),
                ("a newspaper hoax", None),
            ],
            "a generated card lost the reviewed alternatives or left its selected sense buried among them"
        );
    }

    #[test]
    fn skipped_words_never_become_cards() {
        let kept = WordCandidate::new("canard", "a duck", true);
        let dropped = WordCandidate::new("xyzzy", "not a word", false);
        let drafts = drafts_for(&[kept, dropped], &pair());
        assert_eq!(
            drafts.iter().map(CardDraft::term).collect::<Vec<_>>(),
            vec!["canard"],
            "rows marked not-ok must never reach card generation"
        );
    }

    #[test]
    fn produce_drives_every_card_to_publish() {
        let drafts = vec![
            CardDraft::new("canard", "a duck", pair()),
            CardDraft::new("flaner", "to stroll", pair()),
        ];
        let reporter = RecordingReporter::default();
        produce(&LocalWorkflow, drafts, &reporter).expect("produce must publish");
        assert_eq!(
            *reporter.published.borrow(),
            Some((2, 0)),
            "produce must publish every fully generated card with no failures"
        );
    }

    #[test]
    fn produce_runs_artifacts_in_engine_order() {
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&LocalWorkflow, drafts, &reporter).expect("produce must publish");
        assert_eq!(
            *reporter.steps.borrow(),
            vec![
                String::from("canard:meta"),
                String::from("canard:sound"),
                String::from("canard:scene"),
                String::from("canard:picture"),
            ],
            "produce must visit artifacts in the engine's meta→sound→scene→picture order"
        );
    }

    #[test]
    fn produce_stops_picture_generation_after_the_first_try_and_three_retries() {
        let workflow = FailingPictureWorkflow::default();
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&workflow, drafts, &reporter).expect("produce must publish surviving cards");
        assert_eq!(
            (workflow.pictures.get(), *reporter.published.borrow()),
            (4, Some((0, 1))),
            "produce exceeded the first try plus three retries for one failed card"
        );
    }

    #[test]
    fn every_spent_picture_attempt_reports_the_fault_that_spent_it() {
        let workflow = FailingPictureWorkflow::default();
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&workflow, drafts, &reporter).expect("produce must publish surviving cards");
        assert_eq!(
            *reporter.faults.borrow(),
            vec![String::from("error:picture rejected"); 4],
            "a spent picture attempt reached the reporter without naming its cause"
        );
    }

    #[test]
    fn produce_reports_each_picture_failure_reason() {
        let workflow = FailingPictureWorkflow::default();
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&workflow, drafts, &reporter).expect("produce must publish surviving cards");
        assert_eq!(
            *reporter.warnings.borrow(),
            vec![String::from("canard · picture: picture rejected"); 4],
            "produce hid one or more picture failure reasons"
        );
    }
}
