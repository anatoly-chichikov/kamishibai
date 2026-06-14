use anyhow::{Result, anyhow, bail};

use crate::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};

use super::draft::{CardDraft, CardMeta};
use super::pair::LanguagePair;

/// Bridge one card draft into the strict internal vocabulary entry consumed by
/// the existing `src/generation/*` pipeline. Fails if the meta has not been
/// generated yet — every field on `VocabularyEntry` is non-empty by contract.
pub fn to_entry(draft: &CardDraft) -> Result<VocabularyEntry> {
    let Some(meta) = draft.meta() else {
        bail!("invariant: card meta must be generated before bridging to VocabularyEntry");
    };
    Ok(VocabularyEntry {
        term: NonEmptyText::new(draft.term())?,
        meaning: NonEmptyText::new(meta.meaning())?,
        pronunciation: NonEmptyText::new(meta.pronunciation())?,
        transcription: NonEmptyText::new(meta.transcription())?,
        importance: Importance::new(meta.importance())?,
        source: VocabularySource {
            sentence: NonEmptyText::new(meta.source_sentence())?,
            lang: LanguageCode::new(draft.pair().known())?,
            highlight: NonEmptyText::new(meta.source_highlight())?,
            hint: NonEmptyText::new(meta.source_hint())?,
            context: NonEmptyText::new(meta.source_context())?,
        },
        target: VocabularyTarget {
            sentence: NonEmptyText::new(meta.target_sentence())?,
            lang: LanguageCode::new(draft.pair().learning())?,
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

/// Bridge a validated vocabulary document into its single language pair plus
/// generation drafts, one per entry with meta pre-attached, after checking
/// every entry shares the pair the first entry establishes. The drafts are
/// never empty: an empty document is refused here (and by the strict
/// vocabulary schema before that).
pub fn drafts_from_document(
    document: &VocabularyDocument,
) -> Result<(LanguagePair, Vec<CardDraft>)> {
    let pair = pair_from_document(document)?;
    let drafts = document
        .entries
        .iter()
        .map(|entry| from_entry(entry, pair.clone()))
        .collect();
    Ok((pair, drafts))
}

fn pair_from_document(document: &VocabularyDocument) -> Result<LanguagePair> {
    let first = document
        .entries
        .first()
        .ok_or_else(|| anyhow!("vocabulary document contains no entries"))?;
    let target = first.target.lang.as_str().to_uppercase();
    let support = first.source.lang.as_str().to_uppercase();
    for (index, entry) in document.entries.iter().enumerate().skip(1) {
        if !entry.target.lang.as_str().eq_ignore_ascii_case(&target) {
            bail!(
                "entry {} has target language '{}' but the batch started with '{}'",
                index,
                entry.target.lang.as_str(),
                target
            );
        }
        if !entry.source.lang.as_str().eq_ignore_ascii_case(&support) {
            bail!(
                "entry {} has source language '{}' but the batch started with '{}'",
                index,
                entry.source.lang.as_str(),
                support
            );
        }
    }
    Ok(LanguagePair::new(target, support))
}

/// Build one card draft from a strict vocabulary entry, with the rich meta
/// already attached. The engine treats the Meta slot as ready and starts at the
/// first media artifact, so callers loading a pre-rendered batch from JSON skip
/// the Gemini meta-generation pass entirely.
pub fn from_entry(entry: &VocabularyEntry, pair: LanguagePair) -> CardDraft {
    let meta = CardMeta::new(
        entry.pronunciation.as_str(),
        entry.transcription.as_str(),
        entry.meaning.as_str(),
        entry.importance.value(),
        entry.source.sentence.as_str(),
        entry.source.highlight.as_str(),
        entry.source.hint.as_str(),
        entry.source.context.as_str(),
        entry.target.sentence.as_str(),
    );
    CardDraft::new(entry.term.as_str(), entry.meaning.as_str(), pair).with_meta(meta, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::session::{Artifact, SessionEngine};

    fn sample_entry() -> VocabularyEntry {
        VocabularyEntry {
            term: NonEmptyText::new("sincerely").expect("term must accept non-empty"),
            meaning: NonEmptyText::new("искренне").expect("meaning must accept non-empty"),
            pronunciation: NonEmptyText::new("sɪnˈsɪəli")
                .expect("pronunciation must accept non-empty"),
            transcription: NonEmptyText::new("aɪ sɪnˈsɪəli əˈpɒlədʒaɪz")
                .expect("transcription must accept non-empty"),
            importance: Importance::new(7).expect("importance 7 must be in 1..=10"),
            source: VocabularySource {
                sentence: NonEmptyText::new("Я искренне извиняюсь.")
                    .expect("source.sentence must accept non-empty"),
                lang: LanguageCode::new("ru").expect("lang must accept non-empty"),
                highlight: NonEmptyText::new("искренне")
                    .expect("source.highlight must accept non-empty"),
                hint: NonEmptyText::new("От всего сердца, без капли притворства.")
                    .expect("source.hint must accept non-empty"),
                context: NonEmptyText::new("Наречие, образованное от sincere.")
                    .expect("source.context must accept non-empty"),
            },
            target: VocabularyTarget {
                sentence: NonEmptyText::new("I sincerely apologize.")
                    .expect("target.sentence must accept non-empty"),
                lang: LanguageCode::new("en").expect("lang must accept non-empty"),
            },
        }
    }

    #[test]
    fn from_entry_attaches_meta_so_engine_skips_meta_pass() {
        let entry = sample_entry();
        let pair = LanguagePair::new("en", "ru");
        let draft = from_entry(&entry, pair);
        let engine = SessionEngine::start(vec![draft]);
        assert_eq!(
            engine.next_target().map(|(_, kind)| kind),
            Some(Artifact::Sound),
            "engine cannot start at Meta when the JSON batch already supplies one"
        );
    }
}
