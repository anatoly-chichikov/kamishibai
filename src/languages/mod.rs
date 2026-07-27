//! Language profiles, naming, and report labels.

const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

mod labels;
mod naming;
mod profile;
mod prompt_examples;
mod registry;

pub use labels::ReportLabels;
pub use naming::{naming, prefix};
pub use profile::{DeckNaming, LanguageEntry, LanguageProfile, UiLabels};
pub(crate) use prompt_examples::recall_document as prompt_recall_examples_document;
pub use registry::{LanguageCatalog, catalog, language};
