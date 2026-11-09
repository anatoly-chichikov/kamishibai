//! Counts derived from raw selected rows, shared by preview and validation.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::manifest::{Card, Deck, Manifest, Revlog};

/// A serializable overview of selected rows and the exact-deck checklist.
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    decks: Vec<DeckSummary>,
    counts: Counts,
    missing_media: Vec<String>,
    snapshot_size: usize,
}

impl Preview {
    /// Assemble a preview from its source rows and selected-deck checklist.
    pub(super) fn new(
        manifest: &Manifest,
        decks: Vec<DeckSummary>,
        missing_media: Vec<String>,
        snapshot_size: usize,
    ) -> Self {
        Self {
            decks,
            counts: Counts::from_manifest(manifest),
            missing_media,
            snapshot_size,
        }
    }
}

/// A checklist row for one home deck, including selected and unselected decks.
#[derive(Clone, Debug, Serialize)]
pub struct DeckSummary {
    id: i64,
    name: String,
    selected: bool,
    #[serde(flatten)]
    counts: Rows,
}

impl DeckSummary {
    /// Toggle this exact home deck without implicitly choosing descendants.
    pub fn toggle(&mut self) {
        self.selected = !self.selected;
    }

    /// Append this deck's exact name only when selected for export.
    pub fn include(&self, names: &mut Vec<String>) {
        if self.selected {
            names.push(self.name.clone());
        }
    }

    /// Count a home deck's notes, cards and surviving review rows.
    pub(super) fn from_rows(
        deck: &Deck,
        selected: bool,
        cards: &[Card],
        revlog: &[Revlog],
    ) -> Self {
        let own: Vec<_> = cards.iter().filter(|card| card.deck == deck.id).collect();
        Self {
            id: deck.id,
            name: deck.name.clone(),
            selected,
            counts: Rows::from_cards(&own, revlog),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct Counts {
    #[serde(flatten)]
    rows: Rows,
    media: usize,
    files: usize,
    bytes: u64,
}

impl Counts {
    fn from_manifest(manifest: &Manifest) -> Self {
        let files: BTreeMap<_, _> = manifest
            .media
            .iter()
            .map(|media| (&media.sha256, media.size))
            .collect();
        Self {
            rows: Rows::from_cards(&manifest.cards.iter().collect::<Vec<_>>(), &manifest.revlog),
            media: manifest.media.len(),
            files: files.len(),
            bytes: files.values().sum(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct Rows {
    notes: usize,
    cards: usize,
    reviews: usize,
    mistakes: usize,
}

impl Rows {
    fn from_cards(cards: &[&Card], revlog: &[Revlog]) -> Self {
        let ids: BTreeSet<_> = cards.iter().map(|card| card.id).collect();
        let reviews: Vec<_> = revlog
            .iter()
            .filter(|review| ids.contains(&review.card))
            .collect();
        Self {
            notes: cards
                .iter()
                .map(|card| card.note)
                .collect::<BTreeSet<_>>()
                .len(),
            cards: cards.len(),
            reviews: reviews.len(),
            mistakes: reviews.iter().filter(|review| review.ease == 1).count(),
        }
    }
}
