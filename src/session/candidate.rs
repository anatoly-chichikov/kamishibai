use thiserror::Error;

/// Largest number of vocabulary lines one understanding pass accepts.
///
/// The intake call is the only Gemini request that carries the whole batch, and
/// it is neither streamed nor retried. A polysemous list costs roughly 570
/// billed tokens per word, so beyond this many lines one request stops fitting
/// inside the 300-second transport timeout even after chunking hides the
/// per-request cost.
pub const MAX_INTAKE_WORDS: usize = 60;

/// Raised when a batch carries more vocabulary lines than one intake accepts.
#[derive(Debug, Error)]
#[error("too many words: {count} lines, at most {ceiling} per batch")]
pub struct IntakeTooLarge {
    count: usize,
    ceiling: usize,
}

impl IntakeTooLarge {
    /// Create the refusal for one oversized batch.
    #[must_use]
    pub fn new(count: usize) -> Self {
        Self {
            count,
            ceiling: MAX_INTAKE_WORDS,
        }
    }
}

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

    /// Return how many vocabulary lines the blob carries.
    ///
    /// One line is one word to understand: surrounding whitespace is trimmed
    /// and blank lines are dropped. Every surface counts the batch this way, so
    /// the number the footer shows is the number the limit is checked against.
    #[must_use]
    pub fn word_count(&self) -> usize {
        self.lines().count()
    }

    /// Return the trimmed, non-empty vocabulary lines in input order.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    }
}

const MAX_SENSES: usize = 6;

/// Limit one card glossary to its chosen meaning and four prioritized alternatives.
pub(crate) const MAX_CARD_MEANINGS: usize = 5;

/// One possible meaning for a reviewed word.
///
/// `understanding` is a single short sentence in the user's support language.
/// `tag` is a short domain, register, region, idiom, or part-of-use marker
/// shown only when the sense needs it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sense {
    understanding: String,
    tag: Option<String>,
}

impl Sense {
    /// Create one sense with an optional short tag.
    pub fn new(understanding: impl Into<String>, tag: Option<String>) -> Self {
        let tag = tag.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        Self {
            understanding: understanding.into(),
            tag,
        }
    }

    /// Create one untagged sense.
    pub fn plain(understanding: impl Into<String>) -> Self {
        Self::new(understanding, None)
    }

    /// Create one tagged sense.
    pub fn tagged(understanding: impl Into<String>, tag: impl Into<String>) -> Self {
        Self::new(understanding, Some(tag.into()))
    }

    /// Return the human-language explanation for this sense.
    pub fn understanding(&self) -> &str {
        self.understanding.as_str()
    }

    /// Return the optional short sense tag.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// Return whether another understanding differs only in case or whitespace.
    pub(crate) fn matches(&self, understanding: &str) -> bool {
        normalized(self.understanding()) == normalized(understanding)
    }
}

/// One reviewed word produced by the human-in-the-loop understanding pass.
///
/// `senses` is a non-empty ordered list from the most suitable/common sense to
/// rarer alternatives. `selected` points at every sense that should become a
/// card. `ok` is the row-level inclusion gate: `false` rows are not turned into
/// cards but stay visible in `what i understood` with a strikethrough.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordCandidate {
    term: String,
    senses: Vec<Sense>,
    selected: Vec<usize>,
    ok: bool,
}

impl WordCandidate {
    /// Create one reviewed candidate with a single sense.
    pub fn new(term: impl Into<String>, understanding: impl Into<String>, ok: bool) -> Self {
        Self::with_senses(term, vec![Sense::plain(understanding)], 0, ok)
    }

    /// Create one reviewed candidate with an ordered sense list.
    pub fn with_senses(
        term: impl Into<String>,
        senses: Vec<Sense>,
        selected: usize,
        ok: bool,
    ) -> Self {
        Self::with_selected_senses(term, senses, vec![selected], ok)
    }

    /// Create one reviewed candidate with multiple selected senses.
    pub fn with_selected_senses(
        term: impl Into<String>,
        senses: Vec<Sense>,
        selected: Vec<usize>,
        ok: bool,
    ) -> Self {
        let mut senses = deduplicated(senses);
        if senses.is_empty() {
            senses.push(Sense::plain("модель не поняла слово"));
        }
        senses.truncate(MAX_SENSES);
        let selected = normalized_selection(selected, senses.len());
        Self {
            term: term.into(),
            senses,
            selected,
            ok,
        }
    }

    /// Return the term the user entered (with typos already corrected).
    pub fn term(&self) -> &str {
        self.term.as_str()
    }

    /// Return the active human-language explanation.
    pub fn understanding(&self) -> &str {
        self.sense().understanding()
    }

    /// Return the active sense.
    pub fn sense(&self) -> &Sense {
        &self.senses[self.selected()]
    }

    /// Return all available senses in display order.
    pub fn senses(&self) -> &[Sense] {
        self.senses.as_slice()
    }

    /// Return the active sense index.
    pub fn selected(&self) -> usize {
        self.selected[0]
    }

    /// Return every selected sense index in display order.
    pub fn selected_senses(&self) -> &[usize] {
        self.selected.as_slice()
    }

    /// Return how many cards this row will generate.
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Return whether this row should be forwarded to card generation.
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// Return whether the row has more than one selectable sense.
    pub fn has_multiple_senses(&self) -> bool {
        self.senses.len() > 1
    }

    /// Return the candidate with a different active sense selected.
    pub fn selecting(mut self, selected: usize) -> Self {
        self = self.selecting_senses(vec![selected]);
        self
    }

    /// Return the candidate with a different set of selected senses.
    pub fn selecting_senses(mut self, selected: Vec<usize>) -> Self {
        assert!(
            selected.iter().all(|index| *index < self.senses.len()),
            "invariant: every selected sense index must exist"
        );
        self.selected = normalized_selection(selected, self.senses.len());
        self
    }

    /// Return the candidate with its row-level inclusion gate set.
    #[must_use]
    pub fn with_ok(mut self, ok: bool) -> Self {
        self.ok = ok;
        self
    }

    /// Append non-duplicate senses and select the first appended one.
    pub fn with_added_senses(mut self, senses: Vec<Sense>) -> (Self, Option<usize>) {
        let mut first = None;
        for sense in deduplicated(senses) {
            if self.senses.len() >= MAX_SENSES {
                break;
            }
            if self
                .senses
                .iter()
                .any(|existing| same_understanding(existing, &sense))
            {
                continue;
            }
            if first.is_none() {
                first = Some(self.senses.len());
            }
            self.senses.push(sense);
        }
        if let Some(index) = first {
            self.selected = vec![index];
        }
        (self, first)
    }
}

fn normalized_selection(selected: Vec<usize>, len: usize) -> Vec<usize> {
    let last = len.saturating_sub(1);
    let mut output = Vec::new();
    for index in selected {
        let index = index.min(last);
        if !output.contains(&index) {
            output.push(index);
        }
    }
    output.sort_unstable();
    if output.is_empty() {
        output.push(0);
    }
    output
}

fn deduplicated(senses: Vec<Sense>) -> Vec<Sense> {
    let mut output = Vec::new();
    for sense in senses {
        if sense.understanding().trim().is_empty() {
            continue;
        }
        if output
            .iter()
            .any(|existing| same_understanding(existing, &sense))
        {
            continue;
        }
        output.push(sense);
    }
    output
}

fn same_understanding(left: &Sense, right: &Sense) -> bool {
    left.matches(right.understanding())
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
