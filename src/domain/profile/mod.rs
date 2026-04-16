//! Language profiles, naming, labels, and font selection.

const DEFAULT_FONT: &str = "DejaVu Sans";
const DEFAULT_DECK: &str = "Kamishibai Deck";
const DEFAULT_PREFIX: &str = "kamishibai-deck";
const DEFAULT_FILE: &str = "kamishibai.json";
const FALLBACK_OCR: &str = "eng";

mod language;
mod registry;
mod selectors;

pub use language::{
    AudioProfile, DeckNaming, FontProfile, ImageProfile, LanguageEntry, LanguageProfile, UiLabels,
};
pub use registry::{ProfileRegistry, profile, profiles};
pub use selectors::{FontFamily, Fonts, Labels, naming, prefix};
