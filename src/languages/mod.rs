//! Language profiles, naming, labels, and report font selection.

const DEFAULT_FONT: &str = "DejaVu Sans";
const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

mod fonts;
mod labels;
mod naming;
mod profile;
mod registry;

pub use fonts::{FontFamily, ReportFonts};
pub use labels::ReportLabels;
pub use naming::{naming, prefix};
pub use profile::{
    AudioProfile, DeckNaming, FontProfile, ImageProfile, LanguageEntry, LanguageProfile, UiLabels,
};
pub use registry::{LanguageCatalog, catalog, language};
