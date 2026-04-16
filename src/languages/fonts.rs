use super::{DEFAULT_FONT, LanguageEntry, language};

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
pub struct ReportFonts {
    default: String,
}

impl Default for ReportFonts {
    /// Return the default font selector.
    fn default() -> Self {
        Self {
            default: String::from(DEFAULT_FONT),
        }
    }
}

impl ReportFonts {
    /// Return the selected font family for one entry.
    pub fn selected<T>(&self, entry: &T) -> FontFamily
    where
        T: LanguageEntry,
    {
        let names = [entry.source(), entry.target()]
            .into_iter()
            .flatten()
            .filter_map(|code| language(code).ok().map(|item| item.font.report))
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
