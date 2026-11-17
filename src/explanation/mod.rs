//! Offline explanation parts, language quotation, layout, and actionable review.

mod layout;
mod review;
mod writing;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use layout::{compose, quote, tidy};
pub use review::{Verdict, judge};
pub use writing::{Input, Task, Writer, write};

/// Persisted provenance of an attempted explanation, retaining the metadata draft.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Record {
    parts: Option<Parts>,
    task: String,
    draft: String,
}

impl Record {
    /// Retain accepted parts or a fallback together with its assignment and source.
    #[must_use]
    pub fn new(parts: Option<Parts>, task: String, draft: String) -> Self {
        Self { parts, task, draft }
    }

    /// Restore the facts-only metadata source for a later correction request.
    #[must_use]
    pub fn metadata(&self) -> &str {
        &self.draft
    }

    /// Select the new renderer only when this transaction accepted replacement parts.
    #[must_use]
    pub fn format(&self) -> crate::vocabulary::ContextFormat {
        match self.parts {
            Some(_) => crate::vocabulary::ContextFormat::ExplanationV1,
            None => crate::vocabulary::ContextFormat::Legacy,
        }
    }
}

/// The strict model response, shared by writing, editing, and repair requests.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parts {
    meanings: Vec<Meaning>,
    literal: String,
    r#where: Section,
    limits: Section,
    nuance: Nuance,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Meaning {
    head: String,
    note: String,
    from: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Example {
    text: String,
    translation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Section {
    text: String,
    example: Example,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Nuance {
    text: String,
    why: String,
    example: Example,
}

impl Parts {
    fn sections(&self) -> [(&str, &str, &Example); 3] {
        [
            ("where", &self.r#where.text, &self.r#where.example),
            ("limits", &self.limits.text, &self.limits.example),
            ("nuance", &self.nuance.text, &self.nuance.example),
        ]
    }

    fn strings(&self) -> Vec<&str> {
        self.meanings
            .iter()
            .flat_map(|line| [line.head.as_str(), &line.note, &line.from])
            .chain([self.literal.as_str(), &self.nuance.why])
            .chain(
                self.sections()
                    .into_iter()
                    .flat_map(|(_, text, example)| [text, &example.text, &example.translation]),
            )
            .collect()
    }
}

/// Facts used to check an explanation, with optional human-reviewed meanings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Card {
    #[serde(default)]
    term: String,
    translation: String,
    sentence: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    senses: Vec<Sense>,
}

/// One approved sense, numbered in the exact order required on the card.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sense {
    number: String,
    understanding: String,
    label: String,
}

impl Card {
    /// Bind the card's facts without adding or guessing reviewed senses.
    #[must_use]
    pub fn new(term: String, translation: String, sentence: String, senses: Vec<Sense>) -> Self {
        Self {
            term,
            translation,
            sentence,
            senses,
        }
    }
}

impl Sense {
    /// Bind a reviewed sense to its one-based position and optional style label.
    #[must_use]
    pub fn new(number: String, understanding: String, label: String) -> Self {
        Self {
            number,
            understanding,
            label,
        }
    }
}

/// Reader conventions and script-aware quotation policy from the language catalog.
#[derive(Clone, Debug)]
pub struct Language {
    context: crate::languages::ExplanationContext,
    known: Vec<unicode_script::Script>,
    learning: Vec<unicode_script::Script>,
    names: Names,
}

#[derive(Clone, Debug)]
struct Names {
    reader: String,
    learning: String,
    examples: crate::languages::LanguagePromptExamples,
}

impl Language {
    /// Resolve both supported language codes, accepting either letter case.
    pub fn from_codes(known: &str, learning: &str) -> Result<Self> {
        let catalog = crate::languages::catalog();
        let known_profile = catalog.item(known)?;
        let learning_profile = catalog.item(learning)?;
        Ok(Self {
            context: crate::languages::explanation_context(known),
            known: crate::languages::explanation_scripts(known),
            learning: crate::languages::explanation_scripts(learning),
            names: Names {
                reader: known_profile.prompt,
                learning: learning_profile.prompt,
                examples: crate::languages::explanation_examples(known),
            },
        })
    }

    fn foreign(&self, ch: char) -> bool {
        use unicode_script::UnicodeScript;
        let script = ch.script();
        self.learning.contains(&script) && !self.known.contains(&script)
    }
}
