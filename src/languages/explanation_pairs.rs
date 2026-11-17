//! Complete explanation calibrations restricted to their declared language pair.

use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairExample {
    reader: String,
    learning: String,
    text: String,
}

/// Render complete examples only when both catalog language names match.
pub(crate) fn explanation_pair(reader: &str, learning: &str) -> Result<String> {
    let examples: Vec<PairExample> =
        serde_json::from_str(include_str!("../../assets/explanation_pairs.json"))
            .context("could not decode explanation pair examples")?;
    let catalog = super::catalog();
    let mut seen = BTreeSet::new();
    let mut result = String::new();
    for example in examples {
        let known = catalog.item(&example.reader)?.prompt;
        let target = catalog.item(&example.learning)?.prompt;
        ensure!(
            known != target && !example.text.trim().is_empty(),
            "invalid explanation pair {known} to {target}"
        );
        ensure!(
            seen.insert((known.clone(), target.clone())),
            "duplicate explanation pair {known} to {target}"
        );
        if known == reader && target == learning {
            result = format!(
                "\n\nComplete examples for this exact pair, reader {reader} and learning {learning}:\nThese examples calibrate scope, brevity and optional sections. Their reader prose is in {reader}; their marked phrases and example sentences are in {learning}. Copy the standard of accuracy, never their words or facts into another card. Notice that the first note stays within the sentence's sense, literal has no invented continuation, and empty sections stay empty.\n{}",
                example.text
            );
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::PairExample;

    #[test]
    fn calibration_cards_cannot_reuse_the_evaluation_terms() {
        let pairs: Vec<PairExample> =
            serde_json::from_str(include_str!("../../assets/explanation_pairs.json")).unwrap();
        let cards: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/explanation/cards.json"))
                .unwrap();
        let overlaps = pairs
            .into_iter()
            .flat_map(|pair| {
                let examples: Vec<serde_json::Value> = serde_json::from_str(&pair.text).unwrap();
                examples
                    .into_iter()
                    .filter(|example| {
                        cards
                            .iter()
                            .any(|card| card["term"] == example["card"]["term"])
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert!(
            overlaps.is_empty(),
            "calibration reused evaluation terms: {overlaps:?}"
        );
    }
}
