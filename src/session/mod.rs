//! Word-first session contracts: language pair, target detection, and batch state.

mod detection;
mod pair;

pub use detection::{ScriptDetection, TargetDetection, TargetGuess};
pub use pair::LanguagePair;
