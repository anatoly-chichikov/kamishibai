use anyhow::{Result, bail};

use crate::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};

use super::draft::CardDraft;

/// Bridge one card draft into the strict internal vocabulary entry consumed by
/// the existing `src/generation/*` pipeline. Fails if the body has not been
/// generated yet — every field on `VocabularyEntry` is non-empty by contract.
pub fn to_entry(draft: &CardDraft) -> Result<VocabularyEntry> {
    let Some(body) = draft.body() else {
        bail!("invariant: card body must be generated before bridging to VocabularyEntry");
    };
    Ok(VocabularyEntry {
        term: NonEmptyText::new(draft.term())?,
        meaning: NonEmptyText::new(body.meaning())?,
        pronunciation: NonEmptyText::new(body.pronunciation())?,
        transcription: NonEmptyText::new(body.transcription())?,
        importance: Importance::new(body.importance())?,
        source: VocabularySource {
            sentence: NonEmptyText::new(body.source_sentence())?,
            lang: LanguageCode::new(draft.pair().support())?,
            highlight: NonEmptyText::new(body.source_highlight())?,
            hint: NonEmptyText::new(body.source_hint())?,
            context: NonEmptyText::new(body.source_context())?,
        },
        target: VocabularyTarget {
            sentence: NonEmptyText::new(body.target_sentence())?,
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
