//! Language profiles, naming, and report labels.

const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

mod explanation_examples;
mod explanation_pairs;
mod labels;
mod naming;
mod profile;
mod prompt_examples;
mod registry;

pub(crate) use explanation_examples::{
    example_punctuation, example_semicolon, missing_question, mixed_sentence_word, native_scripted,
    native_terminator,
};
pub(crate) use explanation_pairs::explanation_pair;

#[cfg(test)]
pub(crate) use explanation_examples::prompt_scripted;

pub use labels::ReportLabels;
pub use naming::{naming, prefix};
pub use profile::{
    DeckNaming, LanguageEntry, LanguageProfile, OcrModel, TextDirection, TextGate, UiLabels,
};
pub(crate) use prompt_examples::ContextExamples as ExplanationContext;
pub(crate) use prompt_examples::LanguagePromptExamples;
pub(crate) use prompt_examples::recall_document as prompt_recall_examples_document;
pub(crate) use prompt_examples::{explanation_scripts, is_meaning_header};
pub use registry::{LanguageCatalog, LanguageCode, catalog, language};

/// Resolve reader conventions from the same complete prompt-example catalog.
pub(crate) fn explanation_context(code: &str) -> ExplanationContext {
    prompt_examples::examples(code).explanation()
}

/// Resolve authored explanation examples and their existing word-spacing policy.
pub(crate) fn explanation_examples(code: &str) -> LanguagePromptExamples {
    prompt_examples::examples(code)
}
