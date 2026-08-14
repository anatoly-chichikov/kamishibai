use crate::languages::LanguageCatalog;
use anyhow::Result;

/// One guessed learning language with a confidence flag and the languages the
/// same input would equally have read as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningGuess {
    code: String,
    confident: bool,
    alternates: Vec<String>,
}

impl LearningGuess {
    /// Create one learning guess with confidence and no reported alternates.
    pub fn new(code: impl Into<String>, confident: bool) -> Self {
        Self {
            code: code.into(),
            confident,
            alternates: Vec::new(),
        }
    }

    /// Return the guess carrying the languages that were equally plausible.
    #[must_use]
    pub fn with_alternates(mut self, alternates: Vec<String>) -> Self {
        self.alternates = alternates;
        self
    }

    /// Return the guessed language code.
    pub fn code(&self) -> &str {
        self.code.as_str()
    }

    /// Return whether the detector is confident in the guess.
    pub fn confident(&self) -> bool {
        self.confident
    }

    /// Return the languages the same input would equally have read as. Empty
    /// when the input was unambiguous, when the target was pinned, and for
    /// detectors that do not report alternates at all.
    pub fn alternates(&self) -> &[String] {
        self.alternates.as_slice()
    }
}

/// Contract for detecting the learning language from raw user input.
pub trait LearningDetection {
    /// Detect the most likely learning code for one raw blob.
    fn detect(&self, raw: &str, catalog: &LanguageCatalog) -> Result<LearningGuess>;
}

/// Deterministic script-based detector used as a fallback before the LLM pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScriptDetection;

impl LearningDetection for ScriptDetection {
    /// Detect the learning language code from the dominant Unicode script in the input.
    fn detect(&self, raw: &str, _catalog: &LanguageCatalog) -> Result<LearningGuess> {
        let mut tally = Tally::default();
        for character in raw.chars() {
            tally.observe(character);
        }
        Ok(tally.finalize())
    }
}

#[derive(Default)]
struct Tally {
    cyrillic: usize,
    greek: usize,
    han: usize,
    kana: usize,
    latin: usize,
}

impl Tally {
    fn observe(&mut self, character: char) {
        let point = character as u32;
        if (0x0400..=0x04FF).contains(&point) {
            self.cyrillic += 1;
            return;
        }
        if (0x0370..=0x03FF).contains(&point) || (0x1F00..=0x1FFF).contains(&point) {
            self.greek += 1;
            return;
        }
        if (0x3040..=0x30FF).contains(&point) {
            self.kana += 1;
            return;
        }
        if (0x3400..=0x4DBF).contains(&point) || (0x4E00..=0x9FFF).contains(&point) {
            self.han += 1;
            return;
        }
        if character.is_ascii_alphabetic() || (0x00C0..=0x024F).contains(&point) {
            self.latin += 1;
        }
    }

    fn finalize(self) -> LearningGuess {
        if self.kana > 0 {
            return LearningGuess::new("ja", true);
        }
        let max = self.cyrillic.max(self.greek).max(self.han).max(self.latin);
        if max == 0 {
            return LearningGuess::new("en", false);
        }
        if self.cyrillic == max {
            return LearningGuess::new("ru", true);
        }
        if self.greek == max {
            return LearningGuess::new("el", true);
        }
        if self.han == max {
            return LearningGuess::new("zh", true);
        }
        LearningGuess::new("en", false)
    }
}
