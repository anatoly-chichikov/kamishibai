//! Strict vocabulary JSON loading for the primed CLI flow.

use std::path::Path;

use anyhow::{Result, anyhow, bail};

use crate::session::{CardDraft, LanguagePair, from_entry};
use crate::tui::{App, Screen};
use crate::vocabulary::VocabularyDocument;

/// Preloaded batch that can skip intake and enter card generation directly.
pub(super) struct PrimedBatch {
    app: App,
    drafts: Vec<CardDraft>,
}

impl PrimedBatch {
    /// Load one strict vocabulary JSON document from disk.
    pub(super) fn load(path: &Path) -> Result<Self> {
        let document = VocabularyDocument::load(path)?;
        Self::from_document(&document)
    }

    /// Build a primed batch from an already validated vocabulary document.
    pub(super) fn from_document(document: &VocabularyDocument) -> Result<Self> {
        let pair = pair_from_document(document)?;
        let drafts: Vec<CardDraft> = document
            .entries
            .iter()
            .map(|entry| from_entry(entry, pair.clone()))
            .collect();
        let target = pair.target().to_string();
        let app = App::new(pair)
            .confirmed_target(target)
            .with_screen(Screen::YourCards)
            .cards_started(drafts.clone());
        Ok(Self { app, drafts })
    }

    /// Consume the batch into the TUI seed state and generation drafts.
    pub(super) fn into_parts(self) -> (App, Vec<CardDraft>) {
        (self.app, self.drafts)
    }
}

fn pair_from_document(document: &VocabularyDocument) -> Result<LanguagePair> {
    let first = document
        .entries
        .first()
        .ok_or_else(|| anyhow!("vocabulary document contains no entries"))?;
    let target = first.target.lang.as_str();
    let support = first.source.lang.as_str();
    for (index, entry) in document.entries.iter().enumerate().skip(1) {
        if entry.target.lang.as_str() != target {
            bail!(
                "entry {} has target language '{}' but the batch started with '{}'",
                index,
                entry.target.lang.as_str(),
                target
            );
        }
        if entry.source.lang.as_str() != support {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_batch_load_locks_in_the_target_language_so_the_chip_drops_the_pending_dots() {
        let payload = serde_json::json!({
            "entries": [{
                "term": "sincerely",
                "meaning": "искренне",
                "pronunciation": "sɪnˈsɪəli",
                "transcription": "aɪ sɪnˈsɪəli əˈpɒlədʒaɪz",
                "importance": 7,
                "source": {
                    "sentence": "Я искренне извиняюсь.",
                    "lang": "ru",
                    "highlight": "искренне",
                    "hint": "От всего сердца.",
                    "context": "Наречие."
                },
                "target": {
                    "sentence": "I sincerely apologize.",
                    "lang": "en"
                }
            }]
        });
        let document: VocabularyDocument =
            serde_json::from_value(payload).expect("batch must parse");
        let (app, _drafts) = PrimedBatch::from_document(&document)
            .expect("batch must derive app and drafts")
            .into_parts();
        assert_eq!(
            (
                app.pair().support().to_string(),
                app.pair().target().to_string(),
                app.target_pending(),
            ),
            (String::from("ru"), String::from("en"), false),
            "loaded batch must seed the chip with file's languages and not leave it pending"
        );
    }
}
