use serde::{Deserialize, Serialize};

/// One normalized vocabulary entry from the input document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VocabularyEntry {
    pub word: String,
    pub pronunciation: String,
    pub translation: String,
    pub example: String,
    pub source_lang: String,
    pub target_lang: String,
    pub sentence: String,
    pub highlight: String,
    pub hint: String,
    pub context: String,
    pub importance: String,
    pub transcription: String,
}
