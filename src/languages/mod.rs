//! Language profiles, naming, and report labels.

const DEFAULT_FONT: &str = "DejaVu Sans";
const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

mod labels;
mod naming;
mod profile;
mod registry;

pub use labels::ReportLabels;
pub use naming::{naming, prefix};
pub use profile::{DeckNaming, LanguageEntry, LanguageProfile, UiLabels};
pub use registry::{LanguageCatalog, catalog, language};
