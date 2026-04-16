//! PDF report rendering with system fonts and thumbnails.

mod document;
mod font;
mod layout;
mod thumbnail;

pub use document::Report;
pub use font::{FontFamily, FontPath};
pub use layout::{FontSelector, LabelSource, ReportLayout, VocabularyLayout};
pub use thumbnail::Thumbnail;
