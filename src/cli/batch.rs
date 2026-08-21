//! Strict vocabulary JSON loading for cards available at startup.

use std::path::Path;

use anyhow::{Result, bail};

use crate::session::{CardDraft, MAX_PLAN_CARDS, drafts_from_document};
use crate::tui::{App, Screen};
use crate::vocabulary::VocabularyDocument;

/// Cards loaded before the TUI starts so generation can begin on `Your Cards`.
pub(super) struct StartupCards {
    app: App,
    drafts: Vec<CardDraft>,
}

impl StartupCards {
    /// Load one strict vocabulary JSON document from disk.
    pub(super) fn load(path: &Path) -> Result<Self> {
        let document = VocabularyDocument::load(path)?;
        Self::from_document(&document)
    }

    /// Build startup cards from an already validated vocabulary document.
    pub(super) fn from_document(document: &VocabularyDocument) -> Result<Self> {
        let (pair, drafts) = drafts_from_document(document)?;
        if drafts.len() > MAX_PLAN_CARDS {
            bail!(
                "Expected at most {MAX_PLAN_CARDS} entries but found {}",
                drafts.len()
            );
        }
        let learning = pair.learning().to_string();
        let app = App::new(pair)
            .confirmed_learning(learning)
            .with_screen(Screen::YourCards)
            .cards_started(drafts.clone());
        Ok(Self { app, drafts })
    }

    /// Consume the batch into the TUI seed state and generation drafts.
    pub(super) fn into_parts(self) -> (App, Vec<CardDraft>) {
        (self.app, self.drafts)
    }
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
        let (app, _drafts) = StartupCards::from_document(&document)
            .expect("batch must derive app and drafts")
            .into_parts();
        assert_eq!(
            (
                app.pair().known().to_string(),
                app.pair().learning().to_string(),
                app.learning_pending(),
            ),
            (String::from("RU"), String::from("EN"), false),
            "loaded batch must seed the chip with file's languages (uppercased) and not leave it pending"
        );
    }
}
