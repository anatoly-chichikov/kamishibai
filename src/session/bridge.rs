use anyhow::Result;

use crate::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};

use super::draft::CardDraft;

/// Bridge one card draft into the strict internal vocabulary entry used by the
/// existing `src/generation/*` pipeline.
///
/// This keeps the expensive pieces (`catalog.rs`, `manga/render.rs`, `speech.rs`)
/// working without modification while the user-facing flow migrates off the
/// JSON-first entry point.
pub fn to_entry(draft: &CardDraft) -> Result<VocabularyEntry> {
    Ok(VocabularyEntry {
        term: NonEmptyText::new(draft.term())?,
        meaning: NonEmptyText::new(draft.payload().back())?,
        pronunciation: NonEmptyText::new(draft.payload().front())?,
        transcription: NonEmptyText::new(draft.payload().highlight())?,
        importance: Importance::new(5)?,
        source: VocabularySource {
            sentence: NonEmptyText::new(draft.payload().back())?,
            lang: LanguageCode::new(draft.pair().support())?,
            highlight: NonEmptyText::new(draft.payload().highlight())?,
            hint: NonEmptyText::new(draft.payload().hint())?,
            context: NonEmptyText::new(draft.payload().hint())?,
        },
        target: VocabularyTarget {
            sentence: NonEmptyText::new(draft.payload().front())?,
            lang: LanguageCode::new(draft.pair().target())?,
        },
    })
}

/// Bridge a batch of drafts into one internal vocabulary document.
pub fn to_document(drafts: &[CardDraft]) -> Result<VocabularyDocument> {
    let mut entries = Vec::with_capacity(drafts.len());
    for draft in drafts {
        entries.push(to_entry(draft)?);
    }
    Ok(VocabularyDocument { entries })
}
