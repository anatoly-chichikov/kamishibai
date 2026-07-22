//! Session engine: tracks per-card artifact state.
//!
//! The engine is purely state. It exposes `next_target` to advertise the next
//! pending artifact, and `applied_*` methods that the shell calls once a
//! background worker has produced (or failed to produce) an artifact. The
//! shell drives all actual Gemini and disk work in background threads so the
//! TUI never blocks on network I/O.

use anyhow::Result;

use super::GenerationCost;
use super::draft::{
    Artifact, ArtifactAttempt, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta,
};

/// One step emitted by the engine for the outer shell to consume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineEvent {
    /// An artifact succeeded for one card.
    ArtifactReady { card: usize, artifact: Artifact },
    /// A retry attempt was started for one artifact.
    RetryStarted {
        card: usize,
        artifact: Artifact,
        attempt: u8,
    },
    /// A retry budget ran out for one artifact.
    RetryExhausted { card: usize, artifact: Artifact },
    /// Every artifact of every card finished successfully.
    BatchReady,
    /// The queue drained, possibly with terminal failures.
    BatchDone { failed_cards: usize },
}

/// Session engine state: the ordered batch of drafts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEngine {
    drafts: Vec<CardDraft>,
}

impl SessionEngine {
    /// Start one session from a list of drafts.
    pub fn start(drafts: Vec<CardDraft>) -> Self {
        Self { drafts }
    }

    /// Return the current drafts.
    pub fn drafts(&self) -> &[CardDraft] {
        self.drafts.as_slice()
    }

    /// Return the next pending artifact to work on, if any.
    pub fn next_target(&self) -> Option<(usize, Artifact)> {
        for (index, draft) in self.drafts.iter().enumerate() {
            let artifacts = draft.artifacts();
            for kind in [
                Artifact::Meta,
                Artifact::Sound,
                Artifact::Scene,
                Artifact::Picture,
            ] {
                let current = slot(artifacts, kind);
                if !current.complete() {
                    return Some((index, kind));
                }
            }
        }
        None
    }

    /// Apply the outcome of a meta-generation pass for one card.
    pub fn applied_meta(
        &mut self,
        card: usize,
        result: Result<(CardMeta, Option<ArtifactFile>)>,
    ) -> EngineEvent {
        self.applied_meta_attempt(card, ArtifactAttempt::unmetered(result))
    }

    /// Apply a meta-generation operation together with cumulative known spend.
    pub fn applied_meta_attempt(
        &mut self,
        card: usize,
        attempt: ArtifactAttempt<(CardMeta, Option<ArtifactFile>)>,
    ) -> EngineEvent {
        let attempt_before = slot(self.drafts[card].artifacts(), Artifact::Meta)
            .tally()
            .done()
            .saturating_add(1);
        let (result, cost) = attempt.into_parts();
        match result {
            Ok((meta, file)) => {
                let updated = self.drafts[card].clone().with_meta(meta, file);
                let artifacts = mark_cost(updated.artifacts().clone(), Artifact::Meta, cost);
                self.drafts[card] = updated.with_artifacts(artifacts);
                EngineEvent::ArtifactReady {
                    card,
                    artifact: Artifact::Meta,
                }
            }
            Err(_) => {
                let bumped =
                    mark_attempted(self.drafts[card].artifacts().clone(), Artifact::Meta, cost);
                let cascaded = if slot(&bumped, Artifact::Meta).failed_terminally() {
                    discard_dependents_of_meta(bumped)
                } else {
                    bumped
                };
                self.drafts[card] = self.drafts[card].clone().with_artifacts(cascaded);
                let latest = slot(self.drafts[card].artifacts(), Artifact::Meta);
                if latest.failed_terminally() {
                    EngineEvent::RetryExhausted {
                        card,
                        artifact: Artifact::Meta,
                    }
                } else {
                    EngineEvent::RetryStarted {
                        card,
                        artifact: Artifact::Meta,
                        attempt: attempt_before,
                    }
                }
            }
        }
    }

    /// Apply the outcome of a media-artifact pass (scene/picture/sound) for one card.
    pub fn applied_media(
        &mut self,
        card: usize,
        artifact: Artifact,
        result: Result<ArtifactFile>,
    ) -> EngineEvent {
        self.applied_media_attempt(card, artifact, ArtifactAttempt::unmetered(result))
    }

    /// Apply one media operation together with cumulative known spend.
    pub fn applied_media_attempt(
        &mut self,
        card: usize,
        artifact: Artifact,
        attempt: ArtifactAttempt<ArtifactFile>,
    ) -> EngineEvent {
        debug_assert!(
            !matches!(artifact, Artifact::Meta),
            "applied_media must not be called with Artifact::Meta"
        );
        let attempt_before = slot(self.drafts[card].artifacts(), artifact)
            .tally()
            .done()
            .saturating_add(1);
        let (result, cost) = attempt.into_parts();
        match result {
            Ok(file) => {
                self.drafts[card] = self.drafts[card].clone().with_artifacts(mark_ready(
                    self.drafts[card].artifacts().clone(),
                    artifact,
                    file,
                    cost,
                ));
                EngineEvent::ArtifactReady { card, artifact }
            }
            Err(_) => {
                let bumped = mark_attempted(self.drafts[card].artifacts().clone(), artifact, cost);
                let cascaded = if slot(&bumped, artifact).failed_terminally()
                    && matches!(artifact, Artifact::Scene)
                {
                    discard_picture(bumped)
                } else {
                    bumped
                };
                self.drafts[card] = self.drafts[card].clone().with_artifacts(cascaded);
                let latest = slot(self.drafts[card].artifacts(), artifact);
                if latest.failed_terminally() {
                    EngineEvent::RetryExhausted { card, artifact }
                } else {
                    EngineEvent::RetryStarted {
                        card,
                        artifact,
                        attempt: attempt_before,
                    }
                }
            }
        }
    }

    /// Return BatchReady or BatchDone if all per-card work is complete; otherwise None.
    pub fn batch_state(&self) -> Option<EngineEvent> {
        if !self.fully_drained() {
            return None;
        }
        if self.all_ready() {
            Some(EngineEvent::BatchReady)
        } else {
            Some(EngineEvent::BatchDone {
                failed_cards: self.failed_cards(),
            })
        }
    }

    fn all_ready(&self) -> bool {
        self.drafts
            .iter()
            .all(|draft| draft.artifacts().all_ready())
    }

    fn fully_drained(&self) -> bool {
        self.drafts.iter().all(|draft| {
            let artifacts = draft.artifacts();
            [
                Artifact::Meta,
                Artifact::Sound,
                Artifact::Scene,
                Artifact::Picture,
            ]
            .iter()
            .all(|kind| slot(artifacts, *kind).complete())
        })
    }

    fn failed_cards(&self) -> usize {
        self.drafts
            .iter()
            .filter(|draft| draft.artifacts().has_failed())
            .count()
    }
}

fn slot(artifacts: &CardArtifacts, kind: Artifact) -> &ArtifactSlot {
    match kind {
        Artifact::Meta => artifacts.meta(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    }
}

fn mark_ready(
    artifacts: CardArtifacts,
    kind: Artifact,
    file: ArtifactFile,
    cost: Option<GenerationCost>,
) -> CardArtifacts {
    reshape(artifacts, kind, |slot| {
        mark_slot_cost(slot.succeeded_with(file.clone()), cost)
    })
}

fn mark_attempted(
    artifacts: CardArtifacts,
    kind: Artifact,
    cost: Option<GenerationCost>,
) -> CardArtifacts {
    reshape(artifacts, kind, |slot| {
        mark_slot_cost(slot.attempted(), cost)
    })
}

fn mark_cost(
    artifacts: CardArtifacts,
    kind: Artifact,
    cost: Option<GenerationCost>,
) -> CardArtifacts {
    reshape(artifacts, kind, |slot| mark_slot_cost(slot, cost))
}

fn mark_slot_cost(slot: ArtifactSlot, cost: Option<GenerationCost>) -> ArtifactSlot {
    match cost {
        Some(cost) => slot.accounted(cost),
        None => slot,
    }
}

fn discard_dependents_of_meta(artifacts: CardArtifacts) -> CardArtifacts {
    reshape(
        reshape(
            reshape(artifacts, Artifact::Scene, |slot| slot.discard()),
            Artifact::Picture,
            |slot| slot.discard(),
        ),
        Artifact::Sound,
        |slot| slot.discard(),
    )
}

fn discard_picture(artifacts: CardArtifacts) -> CardArtifacts {
    reshape(artifacts, Artifact::Picture, |slot| slot.discard())
}

fn reshape<F>(artifacts: CardArtifacts, kind: Artifact, mutate: F) -> CardArtifacts
where
    F: Fn(ArtifactSlot) -> ArtifactSlot,
{
    let meta = if kind == Artifact::Meta {
        mutate(artifacts.meta().clone())
    } else {
        artifacts.meta().clone()
    };
    let scene = if kind == Artifact::Scene {
        mutate(artifacts.scene().clone())
    } else {
        artifacts.scene().clone()
    };
    let picture = if kind == Artifact::Picture {
        mutate(artifacts.picture().clone())
    } else {
        artifacts.picture().clone()
    };
    let sound = if kind == Artifact::Sound {
        mutate(artifacts.sound().clone())
    } else {
        artifacts.sound().clone()
    };
    CardArtifacts::from_parts(meta, scene, picture, sound)
}
