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
    Other(String),
}

impl CandidateKind {
    /// Return the label displayed next to the candidate.
    pub fn label(&self) -> &str {
        match self {
            CandidateKind::Word => "word",
            CandidateKind::Phrase => "phrase",
            CandidateKind::Idiom => "idiom",
            CandidateKind::Collocation => "collocation",
            CandidateKind::Other(value) => value.as_str(),
        }
    }
}

/// One candidate row produced by the cheap understanding pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordCandidate {
    term: String,
    kind: CandidateKind,
    preview: String,
    note: String,
}

impl WordCandidate {
    /// Create one candidate row.
    pub fn new(
        term: impl Into<String>,
        kind: CandidateKind,
        preview: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            term: term.into(),
            kind,
            preview: preview.into(),
            note: note.into(),
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
}
