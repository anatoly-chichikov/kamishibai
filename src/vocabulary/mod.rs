//! Vocabulary document parsing and canonical entry types.

mod document;
mod entry;

pub use entry::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};
