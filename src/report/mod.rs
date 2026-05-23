//! Printable PDF rendering with embedded fonts and thumbnails.

mod cards;
mod document;
mod font;
mod layout;
mod thumbnail;

pub use cards::CardSheet;
pub use document::Report;
pub use font::{FontFamily, FontPalette, FontPath, warm_fonts_async};
pub use layout::{LabelSource, ReportLayout, VocabularyLayout};
pub use thumbnail::Thumbnail;
