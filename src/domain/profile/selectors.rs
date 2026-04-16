use std::collections::BTreeSet;

use crate::domain::entry::NormalizedEntry;

use super::{
    DEFAULT_DECK, DEFAULT_FILE, DEFAULT_FONT, DEFAULT_PREFIX, DeckNaming, LanguageEntry, UiLabels,
    profile,
};

/// Return a filesystem-safe deck prefix.
pub fn prefix(name: &str) -> String {
    let mut value = String::new();
    for item in name.chars() {
        if item.is_ascii_alphanumeric() {
            value.push(item.to_ascii_lowercase());
        } else if !value.ends_with('-') && !value.is_empty() {
            value.push('-');
        }
    }
    let trimmed = value.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return String::from("deck");
    }
    trimmed
}

/// Return the effective deck naming after applying CLI overrides.
pub fn naming(custom: Option<&str>, entries: &[NormalizedEntry]) -> DeckNaming {
    if let Some(item) = custom {
        return DeckNaming::new(item, prefix(item), DEFAULT_FILE);
    }
    let codes = entries
        .iter()
        .map(|entry| entry.target_lang.clone())
        .collect::<BTreeSet<_>>();
    if codes.len() == 1 {
        return profile(
            codes
                .iter()
                .next()
                .expect("single target set must contain one code"),
        )
        .expect("supported target language must resolve")
        .naming()
        .clone();
    }
    DeckNaming::new(DEFAULT_DECK, DEFAULT_PREFIX, DEFAULT_FILE)
}

/// Font family name selected for one report entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFamily {
    name: String,
}

impl FontFamily {
    /// Create one font family handle.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Return the font family name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Select report fonts from the language profiles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fonts {
    default: String,
}

impl Default for Fonts {
    /// Return the default font selector.
    fn default() -> Self {
        Self {
            default: String::from(DEFAULT_FONT),
        }
    }
}

impl Fonts {
    /// Return the selected font family for one entry.
    pub fn selected<T>(&self, entry: &T) -> FontFamily
    where
        T: LanguageEntry,
    {
        let names = [entry.source(), entry.target()]
            .into_iter()
            .flatten()
            .filter_map(|code| {
                profile(code)
                    .ok()
                    .map(|item| String::from(item.font().report()))
            })
            .collect::<Vec<_>>();
        if let Some(item) = names
            .iter()
            .find(|name| name.as_str() != self.default.as_str())
        {
            return FontFamily::new(item.clone());
        }
        if let Some(item) = names.first() {
            return FontFamily::new(item.clone());
        }
        FontFamily::new(self.default.clone())
    }
}

/// Select user-facing labels from the source language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Labels {
    default: UiLabels,
}

impl Default for Labels {
    /// Return the default label selector.
    fn default() -> Self {
        Self {
            default: UiLabels::new("Translation", "Context", "Hint", "Importance"),
        }
    }
}

impl Labels {
    /// Return the selected labels for one entry.
    pub fn selected<T>(&self, entry: &T) -> UiLabels
    where
        T: LanguageEntry,
    {
        let Some(code) = entry.source() else {
            return self.default.clone();
        };
        match profile(code) {
            Ok(item) => item.labels().clone(),
            Err(_) => self.default.clone(),
        }
    }
}
