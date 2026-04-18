use crate::languages::LanguageCatalog;
use anyhow::Result;

/// One guessed target language with a confidence flag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetGuess {
    code: String,
    confident: bool,
}

impl TargetGuess {
    /// Create one target guess with confidence.
    pub fn new(code: impl Into<String>, confident: bool) -> Self {
        Self {
            code: code.into(),
            confident,
        }
    }

    /// Return the guessed language code.
    pub fn code(&self) -> &str {
        self.code.as_str()
    }

    /// Return whether the detector is confident in the guess.
    pub fn confident(&self) -> bool {
        self.confident
    }
}

/// Contract for detecting the target language from raw user input.
pub trait TargetDetection {
    /// Detect the most likely target code for one raw blob.
    fn detect(&self, raw: &str, catalog: &LanguageCatalog) -> Result<TargetGuess>;
}

/// Deterministic script-based detector used as a fallback before the LLM pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScriptDetection;

impl TargetDetection for ScriptDetection {
    /// Detect the target language code from the dominant Unicode script in the input.
    fn detect(&self, raw: &str, _catalog: &LanguageCatalog) -> Result<TargetGuess> {
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
        if (0x3400..=0x4DBF).contains(&point)
            || (0x4E00..=0x9FFF).contains(&point)
            || (0x3040..=0x30FF).contains(&point)
        {
            self.han += 1;
            return;
        }
        if character.is_ascii_alphabetic() || (0x00C0..=0x024F).contains(&point) {
            self.latin += 1;
        }
    }

    fn finalize(self) -> TargetGuess {
        let max = self.cyrillic.max(self.greek).max(self.han).max(self.latin);
        if max == 0 {
            return TargetGuess::new("en", false);
        }
        if self.cyrillic == max {
            return TargetGuess::new("ru", true);
        }
        if self.greek == max {
            return TargetGuess::new("el", true);
        }
        if self.han == max {
            return TargetGuess::new("zh", true);
        }
        TargetGuess::new("en", false)
    }
}
