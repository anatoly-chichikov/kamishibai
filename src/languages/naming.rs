use std::collections::BTreeSet;

use crate::vocabulary::VocabularyEntry;

use super::{DEFAULT_DECK, DEFAULT_FILE, DEFAULT_PREFIX, DeckNaming, language};

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
pub fn naming(custom: Option<&str>, entries: &[VocabularyEntry]) -> DeckNaming {
    if let Some(item) = custom {
        return DeckNaming::new(item, prefix(item), DEFAULT_FILE);
    }
    let codes = entries
        .iter()
        .map(|entry| String::from(entry.target.lang.as_str()))
        .collect::<BTreeSet<_>>();
    if codes.len() == 1 {
        return language(
            codes
                .iter()
                .next()
                .expect("single target set must contain one code"),
        )
        .expect("supported target language must resolve")
        .naming
        .clone();
    }
    DeckNaming::new(DEFAULT_DECK, DEFAULT_PREFIX, DEFAULT_FILE)
}
