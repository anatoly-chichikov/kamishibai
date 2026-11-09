//! Human facts projected from the unchanged history document.

use std::io::{self, Write};

use ratatui::text::{Line, Span};
use serde::Deserialize;
use unicode_width::UnicodeWidthStr;

use crate::anki::history::Preview;
use crate::tui::palette::Ink;
use crate::tui::wrap_words;

/// Present selected totals and missing-media guidance to either delivery surface.
#[derive(Clone, Deserialize)]
pub(super) struct Overview {
    decks: Vec<DeckRow>,
    counts: Totals,
    missing_media: Vec<String>,
}

/// Project one exact deck without changing the machine document.
#[derive(Clone, Deserialize)]
pub(super) struct DeckRow {
    name: String,
    selected: bool,
    cards: usize,
}

#[derive(Clone, Deserialize)]
struct Totals {
    cards: usize,
    reviews: usize,
}

impl Overview {
    /// Project the archive's typed machine document without changing its contract.
    #[must_use]
    pub(super) fn from_preview(preview: &Preview) -> Self {
        serde_json::from_value(
            serde_json::to_value(preview).expect("invariant: history preview serializes"),
        )
        .expect("invariant: history preview contains documented totals and decks")
    }

    /// Render the facts needed for the explicit save decision.
    pub(super) fn summary(&self, folder: &str, width: u16) -> Vec<Line<'static>> {
        let mut lines = wrapped(
            "saves these cards with their review history into one file for kamishibai web",
            width,
            Ink::Detail,
        );
        lines.push(Line::default());
        lines.push(fact("cards", number(self.counts.cards)));
        lines.push(fact("reviews", number(self.counts.reviews)));
        let paths = path_rows(folder, usize::from(width.saturating_sub(10)));
        for (index, path) in paths.into_iter().enumerate() {
            lines.push(fact(if index == 0 { "folder" } else { "" }, path));
        }
        if !self.missing_media.is_empty() {
            lines.push(Line::default());
            for (index, line) in self.missing("anki").into_iter().enumerate() {
                lines.extend(wrapped(
                    &line,
                    width,
                    if index == 0 {
                        Ink::Subject
                    } else {
                        Ink::Detail
                    },
                ));
            }
        }
        lines
    }

    /// Render the verified save receipt with the reviewed card and review totals.
    pub(super) fn receipt(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled("✓ saved ", Ink::Detail.on(false)),
            Span::styled(number(self.counts.cards), Ink::Subject.on(false)),
            Span::styled(
                format!(
                    " {} and ",
                    if self.counts.cards == 1 {
                        "card"
                    } else {
                        "cards"
                    }
                ),
                Ink::Detail.on(false),
            ),
            Span::styled(number(self.counts.reviews), Ink::Subject.on(false)),
            Span::styled(
                format!(
                    " {}",
                    if self.counts.reviews == 1 {
                        "review"
                    } else {
                        "reviews"
                    }
                ),
                Ink::Detail.on(false),
            ),
        ])
    }

    /// Write the exact deck checklist, selected totals and optional missing files.
    pub(super) fn write(
        &self,
        output: &mut impl Write,
        location: &str,
        path: &str,
    ) -> io::Result<()> {
        let mut decks = self.decks.clone();
        decks.sort_by_key(DeckRow::order);
        let names = decks
            .iter()
            .map(|deck| deck.name.width())
            .max()
            .unwrap_or(0);
        let counts = decks
            .iter()
            .map(|deck| quantity(deck.cards, "card").width())
            .max()
            .unwrap_or(0);
        for deck in &decks {
            writeln!(
                output,
                "  {} {}{}   {:>counts$}",
                if deck.selected { "✓" } else { " " },
                deck.name,
                " ".repeat(names.saturating_sub(deck.name.width())),
                quantity(deck.cards, "card")
            )?;
        }
        writeln!(output, "cards     {}", number(self.counts.cards))?;
        writeln!(output, "reviews   {}", number(self.counts.reviews))?;
        writeln!(output, "{location:<10}{path}")?;
        if !self.missing_media.is_empty() {
            let lines = self.missing("Anki");
            writeln!(output, "{}", lines[0])?;
            for name in &self.missing_media {
                writeln!(output, "    {name}")?;
            }
            for line in &lines[1..] {
                writeln!(output, "{line}")?;
            }
        }
        Ok(())
    }

    fn missing(&self, anki: &str) -> Vec<String> {
        let count = self.missing_media.len();
        let (noun, verb, pronoun) = if count == 1 {
            ("sound or picture", "is", "it")
        } else {
            ("sounds or pictures", "are", "they")
        };
        vec![
            format!("✗ {} {noun} {verb} missing in {anki}", number(count)),
            format!(
                "  the web import stops until {pronoun} {} back",
                if count == 1 { "is" } else { "are" }
            ),
            format!(
                "  check {} in {anki}: Tools → Check Media",
                if count == 1 { "it" } else { "them" }
            ),
        ]
    }
}

impl DeckRow {
    /// Project a deck checklist into presentation-only values.
    pub(super) fn from_deck(deck: &crate::anki::history::DeckSummary) -> Self {
        serde_json::from_value(serde_json::to_value(deck).expect("invariant: a deck serializes"))
            .expect("invariant: a deck contains name, selection and card count")
    }

    /// Compare the case-folded path segments, with parents before descendants.
    pub(super) fn order(&self) -> Vec<String> {
        self.name.split("::").map(str::to_lowercase).collect()
    }

    /// Add this selected deck's own cards to the footer census.
    pub(super) fn count(&self) -> usize {
        if self.selected { self.cards } else { 0 }
    }
}

/// Render a sorted exact-deck tree with aligned own-card counts.
pub(super) fn deck_lines(decks: &[DeckRow], width: u16) -> Vec<Vec<Line<'static>>> {
    let mut paths: Vec<(usize, String)> = Vec::new();
    for (index, deck) in decks.iter().enumerate() {
        let parent = decks[..index]
            .iter()
            .enumerate()
            .filter(|(_, other)| deck.name.starts_with(&format!("{}::", other.name)))
            .max_by_key(|(_, other)| other.name.len());
        let (depth, label) = match parent {
            Some((parent, other)) => (
                paths[parent].0 + 1,
                deck.name[other.name.len() + 2..].replace("::", " / "),
            ),
            None => (0, deck.name.replace("::", " / ")),
        };
        paths.push((depth, label));
    }
    let counts = decks
        .iter()
        .map(|deck| quantity(deck.cards, "card").width())
        .max()
        .unwrap_or(0);
    let names = paths
        .iter()
        .map(|(depth, label)| depth * 2 + label.width())
        .max()
        .unwrap_or(0)
        .min(usize::from(width).saturating_sub(2 + 3 + counts));
    decks
        .iter()
        .zip(paths)
        .map(|(deck, (depth, label))| {
            let indent = depth * 2;
            let rows = deck_wrap(&label, names.saturating_sub(indent).max(1));
            rows.into_iter()
                .enumerate()
                .map(|(index, row)| {
                    let mut spans = vec![
                        Span::styled(
                            if index == 0 && deck.selected {
                                "✓ "
                            } else {
                                "  "
                            },
                            Ink::Detail.on(false),
                        ),
                        Span::styled(
                            format!("{}{row}", " ".repeat(indent)),
                            if deck.selected {
                                Ink::Subject
                            } else {
                                Ink::Aside
                            }
                            .on(false),
                        ),
                    ];
                    if index == 0 {
                        spans.push(Span::styled(
                            format!(
                                "{}   {:>counts$}",
                                " ".repeat(names.saturating_sub(indent + row.width())),
                                quantity(deck.cards, "card")
                            ),
                            Ink::Aside.on(false),
                        ));
                    }
                    Line::from(spans)
                })
                .collect()
        })
        .collect()
}

fn deck_wrap(label: &str, width: usize) -> Vec<String> {
    let segments = label.split(" / ").collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut row = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let text = format!(
            "{segment}{}",
            if index + 1 < segments.len() { " /" } else { "" }
        );
        if !row.is_empty() && row.width() + 1 + text.width() > width {
            rows.push(std::mem::take(&mut row));
        }
        for part in wrap_words(&text, width, width) {
            if !row.is_empty() && row.width() + 1 + part.width() <= width {
                row.push(' ');
                row.push_str(&part);
            } else {
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
                row = part;
            }
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn fact(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Ink::Aside.on(false)),
        Span::styled(value, Ink::Subject.on(false)),
    ])
}

fn path_rows(path: &str, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    for part in path.split_inclusive('/') {
        let row = rows.last_mut().expect("invariant: path has a first row");
        if !row.is_empty() && row.width() + part.width() > width {
            rows.push(String::new());
        }
        for part in wrap_words(part, width.max(1), width.max(1)) {
            let row = rows.last_mut().expect("invariant: path has a first row");
            if !row.is_empty() && row.width() + part.width() > width {
                rows.push(part);
            } else {
                row.push_str(&part);
            }
        }
    }
    rows
}

/// Format a human count with its singular or plural noun.
pub(super) fn quantity(count: usize, noun: &str) -> String {
    format!(
        "{} {noun}{}",
        number(count),
        if count == 1 { "" } else { "s" }
    )
}

/// Format a nonnegative count with comma-separated thousands.
pub(super) fn number(count: usize) -> String {
    let digits = count.to_string();
    digits
        .chars()
        .enumerate()
        .fold(String::new(), |mut text, (index, digit)| {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                text.push(',');
            }
            text.push(digit);
            text
        })
}

/// Wrap one uniformly ranked paragraph into terminal cells.
pub(super) fn wrapped(text: &str, width: u16, ink: Ink) -> Vec<Line<'static>> {
    text.split('\n')
        .flat_map(|line| {
            let indent = line.len().saturating_sub(line.trim_start().len());
            let room = usize::from(width).saturating_sub(indent).max(1);
            wrap_words(line, room, room)
                .into_iter()
                .map(move |line| format!("{}{line}", " ".repeat(indent)))
        })
        .map(|line| Line::styled(line, ink.on(false)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overview(cards: usize, reviews: usize, missing: Vec<String>) -> Overview {
        serde_json::from_value(serde_json::json!({
            "decks": [
                {"name":"French::Daily", "selected":true, "cards":1216, "reviews":11854},
                {"name":"Old lessons", "selected":false, "cards":cards, "reviews":reviews}
            ],
            "counts":{"notes":1216,"cards":1216,"reviews":11854,"bytes":881109802},
            "snapshot_size":3891437,
            "missing_media":missing
        }))
        .unwrap()
    }

    #[test]
    fn an_empty_excluded_deck_cannot_distract_from_the_save_decision() {
        let text = overview(0, 0, Vec::new())
            .summary("~/Downloads", 92)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            text,
            "saves these cards with their review history into one file for kamishibai web\n\ncards     1,216\nreviews   11,854\nfolder    ~/Downloads",
            "the decision summary lost its facts or added empty bookkeeping"
        );
    }

    #[test]
    fn missing_media_cannot_look_ready_for_web_import() {
        let text = overview(0, 0, vec![String::from("voice-37.wav")])
            .summary("~/Downloads", 92)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("1 sound or picture is missing in anki")
                && text.contains("Tools → Check Media"),
            "the summary hid the import blocker or its repair action"
        );
    }
}
