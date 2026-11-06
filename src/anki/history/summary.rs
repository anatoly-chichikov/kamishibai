//! Counts derived from raw selected rows, shared by preview and validation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use serde::Serialize;

use super::manifest::{Card, Deck, Manifest, Revlog};

/// A serializable overview; use `write` for a human-readable deck checklist.
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

    /// Write selected decks, totals and missing-media guidance to a delivery stream.
    pub fn write(&self, output: &mut impl Write) -> io::Result<()> {
        for deck in &self.decks {
            writeln!(
                output,
                "  {} {} · {} notes · {} cards · {} reviews · {} mistakes",
                if deck.selected { "✓" } else { "·" },
                deck.name,
                deck.counts.notes,
                deck.counts.cards,
                deck.counts.reviews,
                deck.counts.mistakes
            )?;
        }
        writeln!(
            output,
            "in the file: {} notes · {} cards · {} reviews · {} mistakes",
            self.counts.rows.notes,
            self.counts.rows.cards,
            self.counts.rows.reviews,
            self.counts.rows.mistakes
        )?;
        writeln!(
            output,
            "media: {} references · {} files · {} bytes",
            self.counts.media, self.counts.files, self.counts.bytes
        )?;
        writeln!(
            output,
            "snapshot: {} bytes; about {} bytes plus ZIP headers in total",
            self.snapshot_size,
            self.counts.bytes
                + u64::try_from(self.snapshot_size).expect("invariant: snapshot size fits in u64")
        )?;
        if !self.missing_media.is_empty() {
            writeln!(
                output,
                "{} pictures or sounds are missing in Anki — Kamishibai web cannot import until they are back (Anki: Tools → Check Media)",
                self.missing_media.len()
            )?;
            for name in &self.missing_media {
                writeln!(output, "  missing: {name}")?;
            }
        }
        Ok(())
    }
}

/// A checklist row for one home deck, including selected and unselected decks.
#[derive(Clone, Debug, Serialize)]
pub(super) struct DeckSummary {
    id: i64,
    name: String,
    selected: bool,
    #[serde(flatten)]
    counts: Rows,
}

impl DeckSummary {
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
