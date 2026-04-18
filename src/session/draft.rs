use super::pair::LanguagePair;

/// Artifact type produced for each card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Artifact {
    Scene,
    Picture,
    Sound,
}

impl Artifact {
    /// Return the stable label used in progress output.
    pub fn label(&self) -> &'static str {
        match self {
            Artifact::Scene => "scene",
            Artifact::Picture => "picture",
            Artifact::Sound => "sound",
        }
    }
}

/// Number of attempts already spent versus the absolute cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptTally {
    done: u8,
    ceiling: u8,
}

impl AttemptTally {
    /// Start one tally with an explicit ceiling.
    pub fn new(ceiling: u8) -> Self {
        Self { done: 0, ceiling }
    }

    /// Return the number of attempts already spent.
    pub fn done(&self) -> u8 {
        self.done
    }

    /// Return the ceiling (typically 3).
    pub fn ceiling(&self) -> u8 {
        self.ceiling
    }

    /// Return whether the artifact has run out of retry budget.
    pub fn exhausted(&self) -> bool {
        self.done >= self.ceiling
    }

    /// Record one more attempt.
    pub fn spent(self) -> Self {
        let next = self.done.saturating_add(1);
        Self {
            done: next.min(self.ceiling),
            ceiling: self.ceiling,
        }
    }
}

/// Per-artifact state for one card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSlot {
    kind: Artifact,
    ready: bool,
    tally: AttemptTally,
}

impl ArtifactSlot {
    /// Create one fresh slot for the given artifact kind.
    pub fn fresh(kind: Artifact) -> Self {
        Self {
            kind,
            ready: false,
            tally: AttemptTally::new(3),
        }
    }

    /// Return the artifact kind.
    pub fn kind(&self) -> Artifact {
        self.kind
    }

    /// Return whether the artifact is ready.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Return the attempt tally.
    pub fn tally(&self) -> AttemptTally {
        self.tally
    }

    /// Return the slot as ready.
    pub fn succeeded(mut self) -> Self {
        self.ready = true;
        self
    }

    /// Return the slot after a failed attempt.
    pub fn attempted(mut self) -> Self {
        self.tally = self.tally.spent();
        self
    }

    /// Return whether retry budget has been exhausted without success.
    pub fn failed_terminally(&self) -> bool {
        !self.ready && self.tally.exhausted()
    }
}

/// The three per-card artifact slots bundled together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardArtifacts {
    scene: ArtifactSlot,
    picture: ArtifactSlot,
    sound: ArtifactSlot,
}

impl Default for CardArtifacts {
    /// Return three fresh slots for a new card.
    fn default() -> Self {
        Self {
            scene: ArtifactSlot::fresh(Artifact::Scene),
            picture: ArtifactSlot::fresh(Artifact::Picture),
            sound: ArtifactSlot::fresh(Artifact::Sound),
        }
    }
}

impl CardArtifacts {
    /// Compose one artifacts bundle from three explicit slots.
    pub fn from_parts(scene: ArtifactSlot, picture: ArtifactSlot, sound: ArtifactSlot) -> Self {
        Self {
            scene,
            picture,
            sound,
        }
    }

    /// Return the slot for `Artifact::Scene`.
    pub fn scene(&self) -> &ArtifactSlot {
        &self.scene
    }

    /// Return the slot for `Artifact::Picture`.
    pub fn picture(&self) -> &ArtifactSlot {
        &self.picture
    }

    /// Return the slot for `Artifact::Sound`.
    pub fn sound(&self) -> &ArtifactSlot {
        &self.sound
    }

    /// Return whether every artifact is ready.
    pub fn all_ready(&self) -> bool {
        self.scene.ready() && self.picture.ready() && self.sound.ready()
    }

    /// Return whether any artifact failed terminally.
    pub fn has_failed(&self) -> bool {
        self.scene.failed_terminally()
            || self.picture.failed_terminally()
            || self.sound.failed_terminally()
    }
}

/// Payload visible on both sides of a card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardPayload {
    front: String,
    back: String,
    hint: String,
    highlight: String,
}

impl CardPayload {
    /// Create one card payload.
    pub fn new(
        front: impl Into<String>,
        back: impl Into<String>,
        hint: impl Into<String>,
        highlight: impl Into<String>,
    ) -> Self {
        Self {
            front: front.into(),
            back: back.into(),
            hint: hint.into(),
            highlight: highlight.into(),
        }
    }

    /// Return the front sentence.
    pub fn front(&self) -> &str {
        self.front.as_str()
    }

    /// Return the back sentence with translation.
    pub fn back(&self) -> &str {
        self.back.as_str()
    }

    /// Return the hint text.
    pub fn hint(&self) -> &str {
        self.hint.as_str()
    }

    /// Return the highlighted fragment.
    pub fn highlight(&self) -> &str {
        self.highlight.as_str()
    }
}

/// One card draft inside the current batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardDraft {
    term: String,
    pair: LanguagePair,
    payload: CardPayload,
    artifacts: CardArtifacts,
}

impl CardDraft {
    /// Create one draft.
    pub fn new(term: impl Into<String>, pair: LanguagePair, payload: CardPayload) -> Self {
        Self {
            term: term.into(),
            pair,
            payload,
            artifacts: CardArtifacts::default(),
        }
    }

    /// Return the primary term.
    pub fn term(&self) -> &str {
        self.term.as_str()
    }

    /// Return the language pair locked into the card.
    pub fn pair(&self) -> &LanguagePair {
        &self.pair
    }

    /// Return the payload shown on both sides.
    pub fn payload(&self) -> &CardPayload {
        &self.payload
    }

    /// Return the per-artifact state.
    pub fn artifacts(&self) -> &CardArtifacts {
        &self.artifacts
    }

    /// Return the draft with a new payload (after per-card correction).
    pub fn recomposed(self, payload: CardPayload) -> Self {
        Self {
            term: self.term,
            pair: self.pair,
            payload,
            artifacts: CardArtifacts::default(),
        }
    }

    /// Return the draft with a different artifacts bundle (for retry/success events).
    pub fn with_artifacts(self, artifacts: CardArtifacts) -> Self {
        Self {
            term: self.term,
            pair: self.pair,
            payload: self.payload,
            artifacts,
        }
    }
}
