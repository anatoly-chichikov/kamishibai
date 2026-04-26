/// Raw user input captured on the `Your words` screen.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RawInputBatch {
    text: String,
}

impl RawInputBatch {
    /// Create one raw input batch from a blob of pasted text.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// Return the raw text as-is.
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Return whether the blob has any non-whitespace content.
    pub fn has_content(&self) -> bool {
        self.text
            .chars()
            .any(|character| !character.is_whitespace())
    }
}

/// Classification label attached to one candidate row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateKind {
    Word,
    Phrase,
    Idiom,
    Collocation,
    Sentence,
    Skipped,
}

impl CandidateKind {
    /// Return the label displayed next to the candidate.
    pub fn label(&self) -> &str {
        match self {
            CandidateKind::Word => "word",
            CandidateKind::Phrase => "phrase",
            CandidateKind::Idiom => "idiom",
            CandidateKind::Collocation => "collocation",
            CandidateKind::Sentence => "sentence",
            CandidateKind::Skipped => "skip",
        }
    }
}

/// Visual emphasis for one sense-check metadata label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetaTone {
    Dim,
    Bright,
}

/// One metadata label shown after the translation on the sense-check screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetaSegment {
    text: String,
    tone: MetaTone,
}

impl MetaSegment {
    /// Create a quiet metadata label for facts that are not model decisions.
    pub fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: MetaTone::Dim,
        }
    }

    /// Create a bright metadata label for a model decision or typo correction.
    pub fn bright(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: MetaTone::Bright,
        }
    }

    /// Return the user-facing label text.
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Return how strongly this label should be rendered.
    pub fn tone(&self) -> MetaTone {
        self.tone
    }
}

/// Metadata displayed for one candidate on the sense-check screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateMeta {
    segments: Vec<MetaSegment>,
}

impl CandidateMeta {
    /// Create regular metadata from part/form, sense shade, and optional style.
    pub fn new(part: MetaSegment, sense: MetaSegment, style: Option<MetaSegment>) -> Self {
        let mut segments = vec![part, sense];
        if let Some(segment) = style {
            segments.push(segment);
        }
        Self { segments }
    }

    /// Create typo-correction metadata from part of speech and correction label.
    pub fn typo(part: MetaSegment, correction: impl Into<String>) -> Self {
        Self {
            segments: vec![part, MetaSegment::bright(correction)],
        }
    }

    /// Create metadata from already localized display labels.
    pub fn from_segments(segments: Vec<MetaSegment>) -> Self {
        assert!(
            !segments.is_empty(),
            "invariant: candidate metadata must contain at least one segment"
        );
        Self { segments }
    }

    /// Return every display segment in rendering order.
    pub fn segments(&self) -> &[MetaSegment] {
        self.segments.as_slice()
    }

    /// Return fallback metadata for legacy callers that have only kind and note.
    pub fn legacy(kind: &CandidateKind, note: &str) -> Self {
        let mut segments = vec![MetaSegment::dim(legacy_label(kind))];
        if !note.trim().is_empty() {
            segments.push(MetaSegment::dim(note));
        }
        Self { segments }
    }
}

/// One candidate row produced by the first understanding pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordCandidate {
    term: String,
    kind: CandidateKind,
    preview: String,
    note: String,
    meta: CandidateMeta,
}

impl WordCandidate {
    /// Create one candidate row.
    pub fn new(
        term: impl Into<String>,
        kind: CandidateKind,
        preview: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        let note = note.into();
        let meta = CandidateMeta::legacy(&kind, note.as_str());
        Self {
            term: term.into(),
            kind,
            preview: preview.into(),
            note,
            meta,
        }
    }

    /// Create one candidate row with explicit sense-check metadata.
    pub fn with_meta(
        term: impl Into<String>,
        kind: CandidateKind,
        preview: impl Into<String>,
        note: impl Into<String>,
        meta: CandidateMeta,
    ) -> Self {
        Self {
            term: term.into(),
            kind,
            preview: preview.into(),
            note: note.into(),
            meta,
        }
    }

    /// Return the term the user entered (possibly normalised by the LLM).
    pub fn term(&self) -> &str {
        self.term.as_str()
    }

    /// Return the kind of the candidate.
    pub fn kind(&self) -> &CandidateKind {
        &self.kind
    }

    /// Return the short translation preview shown in What I understood.
    pub fn preview(&self) -> &str {
        self.preview.as_str()
    }

    /// Return any free-form note attached to the candidate.
    pub fn note(&self) -> &str {
        self.note.as_str()
    }

    /// Return the metadata shown on the sense-check screen.
    pub fn meta(&self) -> &CandidateMeta {
        &self.meta
    }

    /// Return whether this row should be forwarded to card generation.
    pub fn included(&self) -> bool {
        !matches!(self.kind, CandidateKind::Skipped)
    }
}

fn legacy_label(kind: &CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Word => "single lexical item",
        CandidateKind::Phrase => "multi-word expression",
        CandidateKind::Idiom => "fixed expression",
        CandidateKind::Collocation => "natural word combination",
        CandidateKind::Sentence => "sentence learned as a unit",
        CandidateKind::Skipped => "not generated",
    }
}
