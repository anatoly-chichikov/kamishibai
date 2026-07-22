//! Non-interactive console flow: the same understand → generate → publish path
//! the TUI walks, scripted for agents and pipes.
//!
//! The driver reuses the real flow components — the `Understanding` pass, the
//! `SessionEngine` artifact queue, and the `DeckPublishing` step — so a console
//! run produces exactly what the interactive run would. Only the rendering
//! differs: progress is reported through a [`Reporter`] port with human,
//! quiet, and NDJSON-on-stderr implementations.

use anyhow::{Result, bail};
use serde::Serialize;

use super::card_workflow::{CardGeneration, DeckPublishing, PublishPhase, PublishProgress};
use super::live_generator::LiveCardGenerator;
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, LanguagePair, SessionEngine, WordCandidate,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepOutcome {
    /// The artifact is ready, optionally served straight from cache.
    Ready { cached: bool },
    /// The artifact failed and a retry will follow.
    Retry { attempt: u8, ceiling: u8 },
    /// The artifact exhausted its retries and was abandoned after `ceiling`.
    Failed { ceiling: u8 },
}

/// Progress sink for one console run. The flow is identical to the TUI; only the
/// rendering differs between implementations.
pub(super) trait Reporter {
    /// Announce that generation started for `cards` cards.
    fn generating(&self, cards: usize);
    /// Report one artifact step for one card.
    fn step(&self, term: &str, artifact: Artifact, outcome: StepOutcome);
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

/// Build a console-flow live generator rooted at the shared cache and output dir.
pub(super) fn generator(output: PathBuf) -> Result<LiveCardGenerator> {
    let cache = Locations::new(LocationArgs::default(), SystemContext).cache()?;
    Ok(LiveCardGenerator::for_console(cache, output))
}

/// Drive the missing artifacts for `drafts` to completion, then publish the deck.
///
/// Reuses the engine order (meta → sound → scene → picture) with per-artifact
/// retries; artifacts already present in the cache are cheap hits, so this is
/// idempotent and resumable. Progress and the final paths flow through `reporter`.
pub(super) fn produce<G>(
    generator: &G,
    drafts: Vec<CardDraft>,
    reporter: &dyn Reporter,
) -> Result<()>
where
    G: CardGeneration + DeckPublishing,
{
    reporter.generating(drafts.len());
    let mut engine = SessionEngine::start(drafts);
    while let Some((card, artifact)) = engine.next_target() {
        if reporter.revoked() {
            bail!("the session no longer names this worker");
        }
        let draft = engine.drafts()[card].clone();
        let term = draft.term().to_string();
        if let Some(error) = advance(generator, &mut engine, card, artifact, &draft) {
            reporter.warn(format!("{term} · {}: {error}", artifact.label()).as_str());
        }
        reporter.step(term.as_str(), artifact, outcome_of(&engine, card, artifact));
    }
    let drafts = engine.drafts().to_vec();
    let cards = drafts.iter().filter(|d| d.artifacts().all_ready()).count();
    let failed = drafts.iter().filter(|d| d.artifacts().has_failed()).count();
    if reporter.revoked() {
        bail!("the session no longer names this worker");
    }
    reporter.publishing();
    let (deck, report, output) = generator.publish_deck(&drafts, &Unwatched)?;
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
    generator: &G,
    engine: &mut SessionEngine,
    card: usize,
    artifact: Artifact,
    draft: &CardDraft,
) -> Option<String>
where
    G: CardGeneration + DeckPublishing,
{
    match artifact {
        Artifact::Meta => {
            let attempt =
                generator.generate_meta(draft.term(), draft.understanding(), draft.pair());
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_meta_attempt(card, attempt);
            error
        }
        Artifact::Scene => {
            let attempt = generator.generate_scene(draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
        Artifact::Picture => {
            let attempt = generator.generate_picture(draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
        Artifact::Sound => {
            let attempt = generator.generate_sound(draft);
            let error = attempt.error().map(|error| format!("{error:#}"));
            engine.applied_media_attempt(card, artifact, attempt);
            error
        }
    }
}

fn outcome_of(engine: &SessionEngine, card: usize, artifact: Artifact) -> StepOutcome {
    let slot = slot_for(engine.drafts()[card].artifacts(), artifact);
    if slot.ready() {
        return StepOutcome::Ready {
            cached: slot.file().map(|file| file.cached()).unwrap_or(false),
        };
    }
    if slot.failed_terminally() {
        return StepOutcome::Failed {
            ceiling: slot.tally().ceiling(),
        };
    }
    StepOutcome::Retry {
        attempt: slot.tally().done(),
        ceiling: slot.tally().ceiling(),
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
                candidate.senses().get(index).map(|sense| {
                    CardDraft::new(candidate.term(), sense.understanding(), pair.clone())
                })
            })
        })
        .collect()
}

/// Reporter that narrates progress to stderr and prints final paths to stdout.
pub(super) struct HumanReporter;

impl Reporter for HumanReporter {
    fn generating(&self, _cards: usize) {}

    fn step(&self, term: &str, artifact: Artifact, outcome: StepOutcome) {
        let label = human_label(artifact);
        match outcome {
            StepOutcome::Ready { cached: true } => eprintln!("  {term} · {label} (cached)"),
            StepOutcome::Ready { cached: false } => eprintln!("  {term} · {label} ✓"),
            StepOutcome::Retry { attempt, ceiling } => {
                eprintln!("  {term} · {label} · retry {attempt}/{ceiling}")
            }
            StepOutcome::Failed { ceiling } => {
                eprintln!("  {term} · {label} · gave up ({ceiling}/{ceiling})")
            }
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

    fn step(&self, _term: &str, _artifact: Artifact, _outcome: StepOutcome) {}

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
        attempt: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ceiling: Option<u8>,
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

    fn step(&self, term: &str, artifact: Artifact, outcome: StepOutcome) {
        let (status, attempt, ceiling) = match outcome {
            StepOutcome::Ready { cached: true } => ("cache", None, None),
            StepOutcome::Ready { cached: false } => ("ok", None, None),
            StepOutcome::Retry { attempt, ceiling } => ("retry", Some(attempt), Some(ceiling)),
            StepOutcome::Failed { ceiling } => ("fail", None, Some(ceiling)),
        };
        stream(&Event::Step {
            term,
            artifact: artifact.label(),
            status,
            attempt,
            ceiling,
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
    use crate::session::{
        ArtifactAttempt, ArtifactFile, CardCorrection, CardMeta, CardMetaGeneration, CardRevision,
        Sense, WordCandidate,
    };

    #[derive(Clone, Default)]
    struct LocalGenerator;

    #[derive(Clone, Default)]
    struct FailingPictureGenerator {
        pictures: Cell<usize>,
    }

    impl CardMetaGeneration for LocalGenerator {
        fn generate_card_meta(
            &self,
            term: &str,
            understanding: &str,
            _pair: &LanguagePair,
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

    impl CardCorrection for LocalGenerator {
        fn correct_card(
            &self,
            draft: &CardDraft,
            _comment: &str,
            _pair: &LanguagePair,
        ) -> Result<CardRevision> {
            let meta =
                self.generate_card_meta(draft.term(), draft.understanding(), draft.pair())?;
            Ok(CardRevision::new(draft.term(), draft.understanding(), meta))
        }
    }

    impl CardGeneration for LocalGenerator {
        fn generate_scene(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(Ok(local_file(draft.term(), "scene")))
        }

        fn generate_picture(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
            ArtifactAttempt::unmetered(Ok(local_file(draft.term(), "picture")))
        }

        fn generate_sound(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
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

    impl DeckPublishing for LocalGenerator {
        fn publish_deck(
            &self,
            drafts: &[CardDraft],
            progress: &dyn PublishProgress,
        ) -> Result<(String, String, String)> {
            progress.advance(PublishPhase::Report);
            Ok((
                format!("/out/deck-{}.apkg", drafts.len()),
                format!("/out/deck-{}.pdf", drafts.len()),
                String::from("/out"),
            ))
        }
    }

    impl CardMetaGeneration for FailingPictureGenerator {
        fn generate_card_meta(
            &self,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
        ) -> Result<CardMeta> {
            LocalGenerator.generate_card_meta(term, understanding, pair)
        }
    }

    impl CardCorrection for FailingPictureGenerator {
        fn correct_card(
            &self,
            draft: &CardDraft,
            comment: &str,
            pair: &LanguagePair,
        ) -> Result<CardRevision> {
            LocalGenerator.correct_card(draft, comment, pair)
        }
    }

    impl CardGeneration for FailingPictureGenerator {
        fn generate_scene(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
            LocalGenerator.generate_scene(draft)
        }

        fn generate_picture(&self, _draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
            self.pictures.set(self.pictures.get().saturating_add(1));
            ArtifactAttempt::unmetered(Err(anyhow::anyhow!("picture rejected")))
        }

        fn generate_sound(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
            LocalGenerator.generate_sound(draft)
        }

        fn store_card_meta(
            &self,
            term: &str,
            understanding: &str,
            pair: &LanguagePair,
            meta: &CardMeta,
        ) -> Result<ArtifactFile> {
            LocalGenerator.store_card_meta(term, understanding, pair, meta)
        }
    }

    impl DeckPublishing for FailingPictureGenerator {
        fn publish_deck(
            &self,
            drafts: &[CardDraft],
            progress: &dyn PublishProgress,
        ) -> Result<(String, String, String)> {
            LocalGenerator.publish_deck(drafts, progress)
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
    }

    impl Reporter for RecordingReporter {
        fn generating(&self, _cards: usize) {}
        fn step(&self, term: &str, artifact: Artifact, _outcome: StepOutcome) {
            self.steps
                .borrow_mut()
                .push(format!("{term}:{}", artifact.label()));
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
        produce(&LocalGenerator, drafts, &reporter).expect("produce must publish");
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
        produce(&LocalGenerator, drafts, &reporter).expect("produce must publish");
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
    fn produce_stops_picture_generation_at_three_failed_calls() {
        let generator = FailingPictureGenerator::default();
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&generator, drafts, &reporter).expect("produce must publish surviving cards");
        assert_eq!(
            (generator.pictures.get(), *reporter.published.borrow()),
            (3, Some((0, 1))),
            "produce exceeded the three-call picture ceiling for one failed card"
        );
    }

    #[test]
    fn produce_reports_each_picture_failure_reason() {
        let generator = FailingPictureGenerator::default();
        let drafts = vec![CardDraft::new("canard", "a duck", pair())];
        let reporter = RecordingReporter::default();
        produce(&generator, drafts, &reporter).expect("produce must publish surviving cards");
        assert_eq!(
            *reporter.warnings.borrow(),
            vec![String::from("canard · picture: picture rejected"); 3],
            "produce hid one or more picture failure reasons"
        );
    }
}
