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

/// One reviewed word produced by the human-in-the-loop understanding pass.
///
/// `understanding` is a single short sentence in the user's support language
/// describing how the model parsed the term: chosen sense for polysemous words,
/// the morphological form for non-default surface forms, typo corrections,
/// register notes, or a reason for excluding the row.
///
/// `ok` is the inclusion gate: `false` rows are not turned into cards but stay
/// visible in `what i understood` (with a strikethrough) so the user can see
/// what was rejected and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordCandidate {
    term: String,
    understanding: String,
    ok: bool,
}

impl WordCandidate {
    /// Create one reviewed candidate.
    pub fn new(term: impl Into<String>, understanding: impl Into<String>, ok: bool) -> Self {
        Self {
            term: term.into(),
            understanding: understanding.into(),
            ok,
        }
    }

    /// Return the term the user entered (with typos already corrected).
    pub fn term(&self) -> &str {
        self.term.as_str()
    }

    /// Return the human-language explanation of how the model understood the term.
    pub fn understanding(&self) -> &str {
        self.understanding.as_str()
    }

    /// Return whether this row should be forwarded to card generation.
    pub fn ok(&self) -> bool {
        self.ok
    }
}
