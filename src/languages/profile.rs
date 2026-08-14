use crate::vocabulary::VocabularyEntry;

/// One PP-OCRv5 recognition bundle declared by a language profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrModel {
    /// Shared multilingual bundle used by Chinese and Japanese.
    Default,
    /// English recognition bundle.
    En,
    /// Latin-script recognition bundle.
    Latin,
    /// Cyrillic-script recognition bundle.
    Cyrillic,
    /// Greek recognition bundle.
    El,
    /// Korean recognition bundle.
    Korean,
    /// Arabic recognition bundle.
    Arabic,
    /// Devanagari recognition bundle.
    Devanagari,
    /// Thai recognition bundle.
    Th,
}

/// Text-validation route for one language's generated manga.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGate {
    /// Validate visible text with the selected PP-OCRv5 bundle.
    Ocr(OcrModel),
    /// Validate visible text directly with the Gemini vision judge.
    LlmJudge,
}

/// Reading direction for language-dependent presentation surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextDirection {
    /// Render text from left to right.
    Ltr,
    /// Render text from right to left.
    Rtl,
}

/// Expose language codes from canonical entries.
pub trait LanguageEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str>;
    /// Return the optional target language code.
    fn target(&self) -> Option<&str>;
}

impl LanguageEntry for VocabularyEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str> {
        Some(self.source.lang.as_str())
    }

    /// Return the optional target language code.
    fn target(&self) -> Option<&str> {
        Some(self.target.lang.as_str())
    }
}

/// Deck naming settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckNaming {
    pub name: String,
    pub prefix: String,
    pub default: String,
}

impl DeckNaming {
    /// Create one deck naming profile.
    pub fn new(
        name: impl Into<String>,
        prefix: impl Into<String>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prefix: prefix.into(),
            default: default.into(),
        }
    }
}

/// User-facing labels for reports and related UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiLabels {
    pub sentence: String,
    pub context: String,
    pub hint: String,
    pub importance: String,
}

impl UiLabels {
    /// Create one label set.
    pub fn new(
        sentence: impl Into<String>,
        context: impl Into<String>,
        hint: impl Into<String>,
        importance: impl Into<String>,
    ) -> Self {
        Self {
            sentence: sentence.into(),
            context: context.into(),
            hint: hint.into(),
            importance: importance.into(),
        }
    }
}

/// One language profile composed from runtime and UI settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageProfile {
    /// Canonical lowercase ISO 639-1 code.
    pub code: &'static str,
    /// English display name used in Gemini prompts.
    pub prompt: String,
    /// The language's name in that language, shown when a human picks it.
    ///
    /// A script the terminal cannot lay out carries its English name instead.
    /// Two of them cannot: a right-to-left script, because the terminal — unlike
    /// the PDF report — does no bidi reordering; and a script that writes one
    /// letter as several code points, because the terminal composes them into a
    /// single glyph while the cell buffer still counts them one by one, so the
    /// cells between them are left unpainted and a highlighted row comes out
    /// with holes in it. Everything else is written the way its own speakers
    /// write it.
    pub endonym: String,
    /// Authoritative route for generated-image text validation.
    pub text_gate: TextGate,
    /// Reading direction used by presentation surfaces.
    pub direction: TextDirection,
    /// Default Anki deck naming.
    pub naming: DeckNaming,
    /// Native labels used when this is the known language.
    pub labels: UiLabels,
}
