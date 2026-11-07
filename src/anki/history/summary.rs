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
                "  {} {} · {} · {} · {} · {}",
                if deck.selected { "✓" } else { "·" },
                deck.name,
                quantity(deck.counts.notes, "note"),
                quantity(deck.counts.cards, "card"),
                quantity(deck.counts.reviews, "review"),
                quantity(deck.counts.mistakes, "mistake")
            )?;
        }
        self.summarize(output)
    }

    /// Write selected totals and omissions without repeating the whole deck checklist.
    pub fn summarize(&self, output: &mut impl Write) -> io::Result<()> {
        writeln!(
            output,
            "in the file: {} · {} · {} · {}",
            quantity(self.counts.rows.notes, "note"),
            quantity(self.counts.rows.cards, "card"),
            quantity(self.counts.rows.reviews, "review"),
            quantity(self.counts.rows.mistakes, "mistake")
        )?;
        writeln!(
            output,
            "media: {} · {} · {} bytes",
            quantity(self.counts.media, "reference"),
            quantity(self.counts.files, "file"),
            self.counts.bytes
        )?;
        writeln!(
            output,
            "estimated file size: {}",
            size(
                self.counts.bytes
                    + u64::try_from(self.snapshot_size)
                        .expect("invariant: snapshot size fits in u64")
            )
        )?;
        let skipped = self
            .decks
            .iter()
            .filter(|deck| !deck.selected)
            .collect::<Vec<_>>();
        writeln!(
            output,
            "skipped: {} · {} · {}",
            quantity(skipped.len(), "deck"),
            quantity(skipped.iter().map(|deck| deck.counts.cards).sum(), "card"),
            quantity(
                skipped.iter().map(|deck| deck.counts.reviews).sum(),
                "review"
            )
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

    /// Write a selectable checklist row with the actual card and review counts.
    pub fn write(&self, output: &mut impl Write) -> io::Result<()> {
        write!(
            output,
            "{} {} · {} · {}",
            if self.selected { "[✓]" } else { "[ ]" },
            self.name,
            quantity(self.counts.cards, "card"),
            quantity(self.counts.reviews, "review")
        )
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

fn quantity(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{} KiB", bytes.div_ceil(1024))
    } else {
        format!("{} MiB", bytes.div_ceil(1024 * 1024))
    }
}
