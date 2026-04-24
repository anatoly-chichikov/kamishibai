//! Session engine: drives the queue `scene -> picture -> sound` for every
//! card draft, one artifact at a time, with a three-attempt retry budget.
//!
//! The real Gemini-backed producer lives in `src/generation/*`. Tests pass
//! in an `ArtifactProducer` fake so the engine can be exercised without
//! touching the network.

use anyhow::Result;

use super::draft::{Artifact, ArtifactSlot, CardArtifacts, CardDraft};

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

/// Contract for artifact generation. Implemented by the real Gemini pipeline
/// and by inline fakes in tests.
pub trait ArtifactProducer {
    /// Attempt to produce one artifact for one draft. Success must mark the
    /// slot as ready; failure lets the engine bump the retry tally.
    fn produce(&mut self, draft: &CardDraft, artifact: Artifact) -> Result<()>;
}

/// Session engine state: the ordered batch of drafts plus a cursor.
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

    /// Advance one step. Returns `None` when the queue has fully drained.
    pub fn advance<P: ArtifactProducer>(&mut self, producer: &mut P) -> Option<EngineEvent> {
        if let Some((index, artifact)) = self.next_target() {
            let draft = self.drafts[index].clone();
            let slot_before = slot(draft.artifacts(), artifact).clone();
            let attempt = slot_before.tally().done().saturating_add(1);
            match producer.produce(&draft, artifact) {
                Ok(()) => {
                    self.drafts[index] = self.drafts[index]
                        .clone()
                        .with_artifacts(mark_ready(draft.artifacts().clone(), artifact));
                    Some(EngineEvent::ArtifactReady {
                        card: index,
                        artifact,
                    })
                }
                Err(_) => {
                    self.drafts[index] = self.drafts[index]
                        .clone()
                        .with_artifacts(mark_attempted(draft.artifacts().clone(), artifact));
                    let latest = slot(self.drafts[index].artifacts(), artifact).clone();
                    if latest.failed_terminally() {
                        Some(EngineEvent::RetryExhausted {
                            card: index,
                            artifact,
                        })
                    } else {
                        Some(EngineEvent::RetryStarted {
                            card: index,
                            artifact,
                            attempt,
                        })
                    }
                }
            }
        } else if self.all_ready() {
            Some(EngineEvent::BatchReady)
        } else if self.fully_drained() {
            Some(EngineEvent::BatchDone {
                failed_cards: self.failed_cards(),
            })
        } else {
            None
        }
    }

    fn next_target(&self) -> Option<(usize, Artifact)> {
        for (index, draft) in self.drafts.iter().enumerate() {
            let artifacts = draft.artifacts();
            for kind in [Artifact::Scene, Artifact::Picture, Artifact::Sound] {
                let current = slot(artifacts, kind);
                if !current.complete() {
                    return Some((index, kind));
                }
            }
        }
        None
    }

    fn all_ready(&self) -> bool {
        self.drafts
            .iter()
            .all(|draft| draft.artifacts().all_ready())
    }

    fn fully_drained(&self) -> bool {
        self.drafts.iter().all(|draft| {
            let artifacts = draft.artifacts();
            [Artifact::Scene, Artifact::Picture, Artifact::Sound]
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
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    }
}

fn mark_ready(artifacts: CardArtifacts, kind: Artifact) -> CardArtifacts {
    reshape(artifacts, kind, |slot| slot.succeeded())
}

fn mark_attempted(artifacts: CardArtifacts, kind: Artifact) -> CardArtifacts {
    reshape(artifacts, kind, |slot| slot.attempted())
}

fn reshape<F>(artifacts: CardArtifacts, kind: Artifact, mutate: F) -> CardArtifacts
where
    F: Fn(ArtifactSlot) -> ArtifactSlot,
{
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
    CardArtifacts::from_parts(scene, picture, sound)
}
