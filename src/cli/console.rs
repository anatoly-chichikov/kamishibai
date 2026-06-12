//! Non-interactive console flow: the same understand → generate → publish path
//! the TUI walks, scripted for agents and pipes.
//!
//! The driver reuses the real flow components — the `Understanding` pass, the
//! `SessionEngine` artifact queue, and the `DeckPublishing` step — so a console
//! run produces exactly what the interactive run would. Only the rendering
//! differs: progress is reported through a [`Reporter`] port with human,
//! NDJSON, and quiet implementations.

use std::sync::mpsc::channel;

use anyhow::{Result, bail};

use super::card_workflow::{CardGeneration, DeckPublishProgress, DeckPublishing};
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
    /// The artifact exhausted its retries and was abandoned.
    Failed,
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
    /// Return whether the run lost ownership of its session and must stop
    /// (a cancel raced in); the default never revokes.
    fn revoked(&self) -> bool {
        false
    }
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
        advance(generator, &mut engine, card, artifact, &draft);
        reporter.step(term.as_str(), artifact, outcome_of(&engine, card, artifact));
    }
    let drafts = engine.drafts().to_vec();
    let cards = drafts.iter().filter(|d| d.artifacts().all_ready()).count();
    let failed = drafts.iter().filter(|d| d.artifacts().has_failed()).count();
    if reporter.revoked() {
        bail!("the session no longer names this worker");
    }
    reporter.publishing();
    let (sender, _receiver) = channel();
    let progress = DeckPublishProgress::new(sender);
    let (deck, report, output) = generator.publish_deck(&drafts, &progress)?;
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
) where
    G: CardGeneration + DeckPublishing,
{
    match artifact {
        Artifact::Meta => {
            let result = generator
                .generate_card_meta(draft.term(), draft.understanding(), draft.pair())
                .map(|meta| {
                    let file = generator
                        .store_card_meta(draft.term(), draft.understanding(), draft.pair(), &meta)
                        .ok();
                    (meta, file)
                });
            engine.applied_meta(card, result);
        }
        Artifact::Scene => {
            engine.applied_media(card, artifact, generator.generate_scene(draft));
        }
        Artifact::Picture => {
            engine.applied_media(card, artifact, generator.generate_picture(draft));
        }
        Artifact::Sound => {
            engine.applied_media(card, artifact, generator.generate_sound(draft));
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
        return StepOutcome::Failed;
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
    fn generating(&self, cards: usize) {
        eprintln!("generating {cards} card(s)…");
    }

    fn step(&self, term: &str, artifact: Artifact, outcome: StepOutcome) {
        let label = artifact.label();
        match outcome {
            StepOutcome::Ready { cached: true } => eprintln!("  cache  {term} · {label}"),
            StepOutcome::Ready { cached: false } => eprintln!("  ok     {term} · {label}"),
            StepOutcome::Retry { attempt, ceiling } => {
                eprintln!("  retry  {term} · {label} ({attempt}/{ceiling})")
            }
            StepOutcome::Failed => eprintln!("  fail   {term} · {label}"),
        }
    }

    fn publishing(&self) {
        eprintln!("building deck and report…");
    }

    fn finished(&self, outcome: &Outcome) {
        if outcome.failed > 0 {
            eprintln!(
                "done: {} card(s) published, {} failed",
                outcome.cards, outcome.failed
            );
        } else {
            eprintln!("done: {} card(s) published", outcome.cards);
        }
        print_paths(outcome);
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::Result;

    use super::*;
    use crate::session::{
        ArtifactFile, CardCorrection, CardMeta, CardMetaGeneration, CardRevision, Sense,
        WordCandidate,
    };

    #[derive(Clone, Default)]
    struct LocalGenerator;

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
        fn generate_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            Ok(local_file(draft.term(), "scene"))
        }

        fn generate_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            Ok(local_file(draft.term(), "picture"))
        }

        fn generate_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
            Ok(local_file(draft.term(), "sound"))
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
            progress: &DeckPublishProgress,
        ) -> Result<(String, String, String)> {
            progress.report_phase(crate::tui::BusyKind::PublishingReport);
            Ok((
                format!("/out/deck-{}.apkg", drafts.len()),
                format!("/out/deck-{}.pdf", drafts.len()),
                String::from("/out"),
            ))
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
}
