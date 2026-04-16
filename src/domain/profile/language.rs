use crate::domain::entry::NormalizedEntry;

/// Expose language codes from normalized entries.
pub trait LanguageEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str>;
    /// Return the optional target language code.
    fn target(&self) -> Option<&str>;
}

impl LanguageEntry for NormalizedEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str> {
        Some(self.source_lang.as_str())
    }

    /// Return the optional target language code.
    fn target(&self) -> Option<&str> {
        Some(self.target_lang.as_str())
    }
}

/// Audio generation settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioProfile {
    language: String,
    cache: String,
}

impl AudioProfile {
    /// Create one audio profile.
    pub fn new(language: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            cache: cache.into(),
        }
    }

    /// Return the prompt language.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Return the audio cache directory name.
    pub fn cache(&self) -> &str {
        &self.cache
    }
}

/// Image generation settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProfile {
    ocr: String,
    cache: String,
}

impl ImageProfile {
    /// Create one image profile.
    pub fn new(ocr: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            ocr: ocr.into(),
            cache: cache.into(),
        }
    }

    /// Return the OCR language string.
    pub fn ocr(&self) -> &str {
        &self.ocr
    }

    /// Return the illustration cache directory name.
    pub fn cache(&self) -> &str {
        &self.cache
    }
}

/// Deck naming settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckNaming {
    name: String,
    prefix: String,
    default: String,
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

    /// Return the deck name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the filesystem-safe deck prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Return the default input filename.
    pub fn default(&self) -> &str {
        &self.default
    }
}

/// Report font settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontProfile {
    report: String,
}

impl FontProfile {
    /// Create one font profile.
    pub fn new(report: impl Into<String>) -> Self {
        Self {
            report: report.into(),
        }
    }

    /// Return the report font family name.
    pub fn report(&self) -> &str {
        &self.report
    }
}

/// User-facing labels for reports and related UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiLabels {
    sentence: String,
    context: String,
    hint: String,
    importance: String,
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

    /// Return the sentence label.
    pub fn sentence(&self) -> &str {
        &self.sentence
    }

    /// Return the context label.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Return the hint label.
    pub fn hint(&self) -> &str {
        &self.hint
    }

    /// Return the importance label.
    pub fn importance(&self) -> &str {
        &self.importance
    }
}

/// One language profile composed from runtime and UI settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageProfile {
    code: String,
    audio: AudioProfile,
    imagery: ImageProfile,
    naming: DeckNaming,
    font: FontProfile,
    labels: UiLabels,
}

impl LanguageProfile {
    /// Create one language profile.
    pub fn new(
        code: impl Into<String>,
        audio: AudioProfile,
        imagery: ImageProfile,
        naming: DeckNaming,
        font: FontProfile,
        labels: UiLabels,
    ) -> Self {
        Self {
            code: code.into(),
            audio,
            imagery,
            naming,
            font,
            labels,
        }
    }

    /// Return the language code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Return the audio profile.
    pub fn audio(&self) -> &AudioProfile {
        &self.audio
    }

    /// Return the image profile.
    pub fn imagery(&self) -> &ImageProfile {
        &self.imagery
    }

    /// Return the naming profile.
    pub fn naming(&self) -> &DeckNaming {
        &self.naming
    }

    /// Return the font profile.
    pub fn font(&self) -> &FontProfile {
        &self.font
    }

    /// Return the UI labels.
    pub fn labels(&self) -> &UiLabels {
        &self.labels
    }
}
