use crate::vocabulary::VocabularyEntry;

/// Expose language codes from normalized entries.
pub trait LanguageEntry {
    /// Return the optional source language code.
    fn source(&self) -> Option<&str>;
    /// Return the optional target language code.
    fn target(&self) -> Option<&str>;
}

impl LanguageEntry for VocabularyEntry {
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
    pub language: String,
    pub cache: String,
}

impl AudioProfile {
    /// Create one audio profile.
    pub fn new(language: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            cache: cache.into(),
        }
    }
}

/// Image generation settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProfile {
    pub ocr: String,
    pub cache: String,
}

impl ImageProfile {
    /// Create one image profile.
    pub fn new(ocr: impl Into<String>, cache: impl Into<String>) -> Self {
        Self {
            ocr: ocr.into(),
            cache: cache.into(),
        }
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

/// Report font settings for one language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontProfile {
    pub report: String,
}

impl FontProfile {
    /// Create one font profile.
    pub fn new(report: impl Into<String>) -> Self {
        Self {
            report: report.into(),
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
    pub code: String,
    pub audio: AudioProfile,
    pub imagery: ImageProfile,
    pub naming: DeckNaming,
    pub font: FontProfile,
    pub labels: UiLabels,
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
}
