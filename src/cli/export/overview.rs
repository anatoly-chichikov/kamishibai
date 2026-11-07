//! Human decision summary projected from the existing history document.

use serde::Deserialize;

use crate::anki::history::Preview;

/// Keep the export screen's copy independent from the console summary.
#[derive(Deserialize)]
pub(super) struct Overview {
    decks: Vec<Deck>,
    counts: Totals,
    missing_media: Vec<String>,
    snapshot_size: u64,
}

#[derive(Deserialize)]
struct Deck {
    name: String,
    selected: bool,
    cards: usize,
    reviews: usize,
}

#[derive(Deserialize)]
struct Totals {
    #[serde(flatten)]
    rows: Rows,
    bytes: u64,
}

#[derive(Deserialize)]
struct Rows {
    notes: usize,
    cards: usize,
    reviews: usize,
}

impl Overview {
    /// Project the archive's typed machine document without changing its contract.
    #[must_use]
    pub(super) fn from_preview(preview: &Preview) -> Self {
        serde_json::from_value(
            serde_json::to_value(preview)
                .expect("invariant: the history preview serializes to a JSON document"),
        )
        .expect("invariant: the history preview contains its documented counts and decks")
    }

    /// Describe the selected learning material, file size and consequential omissions.
    #[must_use]
    pub(super) fn describe(&self) -> String {
        let mut lines = vec![
            format!(
                "{} · {}",
                quantity(self.counts.rows.notes, "word"),
                quantity(self.counts.rows.cards, "card")
            ),
            quantity(self.counts.rows.reviews, "review"),
            format!(
                "from {}",
                self.decks
                    .iter()
                    .filter(|deck| deck.selected)
                    .map(|deck| deck.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            format!(
                "ZIP · about {}",
                size(self.counts.bytes + self.snapshot_size)
            ),
        ];
        let cards = self
            .decks
            .iter()
            .filter(|deck| !deck.selected)
            .map(|deck| deck.cards)
            .sum();
        let reviews = self
            .decks
            .iter()
            .filter(|deck| !deck.selected)
            .map(|deck| deck.reviews)
            .sum();
        if cards != 0 || reviews != 0 {
            lines.push(format!(
                "left out: {} · {}",
                quantity(cards, "card"),
                quantity(reviews, "review")
            ));
        }
        if !self.missing_media.is_empty() {
            lines.push(format!(
                "{} missing · web import blocked",
                quantity(self.missing_media.len(), "file")
            ));
            lines.push(String::from("In Anki: Tools → Check Media"));
        }
        lines.join("\n")
    }
}

fn quantity(count: usize, noun: &str) -> String {
    format!(
        "{} {noun}{}",
        number(count),
        if count == 1 { "" } else { "s" }
    )
}

fn number(count: usize) -> String {
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

fn size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KiB", bytes.div_ceil(1024).max(1))
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{} MiB", bytes.div_ceil(1024 * 1024))
    } else {
        let tenths = (u128::from(bytes) * 10).div_ceil(1024 * 1024 * 1024);
        format!("{}.{} GiB", tenths / 10, tenths % 10)
    }
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
        assert_eq!(
            overview(0, 0, Vec::new()).describe(),
            "1,216 words · 1,216 cards\n11,854 reviews\nfrom French::Daily\nZIP · about 845 MiB",
            "the decision summary lost the learning material or added empty bookkeeping"
        );
    }

    #[test]
    fn excluded_learning_history_cannot_disappear_from_the_summary() {
        assert!(
            overview(17, 139, Vec::new())
                .describe()
                .contains("left out: 17 cards · 139 reviews"),
            "excluded cards and reviews were hidden"
        );
    }

    #[test]
    fn missing_media_cannot_look_ready_for_web_import() {
        assert!(
            overview(0, 0, vec![String::from("voice-37.wav")])
                .describe()
                .contains("1 file missing · web import blocked\nIn Anki: Tools → Check Media"),
            "the summary hid the import blocker or its repair action"
        );
    }
}
