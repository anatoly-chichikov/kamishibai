//! Local checks for the reader sentence displayed on a generated card.

use std::collections::HashMap;

use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use super::Language;
use crate::languages::native_scripted;

impl Language {
    /// Check the reader sentence for copied learning text and unlicensed foreign names.
    #[must_use]
    pub(crate) fn translates_sentence(
        &self,
        learning: &str,
        reader: &str,
        term: &str,
        approved: &str,
    ) -> bool {
        let target = terms(learning);
        let source = terms(reader);
        if source.is_empty() || target.is_empty() || majority_shared(&target, &source) {
            return false;
        }
        let spelling = terms(term);
        let native_equivalent = native_scripted(term, &self.known)
            && (spelling == terms(approved)
                || (self.known.contains(&Script::Latin) && term.trim().chars().count() == 1));
        if !native_equivalent
            && !spelling.is_empty()
            && source
                .windows(spelling.len())
                .any(|window| window == spelling)
        {
            return false;
        }
        let original: Vec<_> = boundaries(learning).collect();
        let segments: Vec<_> = boundaries(reader)
            .map(|part| (part, native_scripted(part, &self.known)))
            .collect();
        segments
            .iter()
            .any(|(part, native)| *native && part.chars().any(char::is_alphabetic))
            && segments
                .chunk_by(|left, right| left.1 == right.1)
                .all(|run| {
                    run[0].1
                        || original.windows(run.len()).any(|window| {
                            window.iter().copied().eq(run.iter().map(|(part, _)| *part))
                                && run
                                    .iter()
                                    .all(|(part, _)| part.chars().any(char::is_alphabetic))
                        })
                })
    }
}

fn boundaries(text: &str) -> impl Iterator<Item = &str> {
    text.split_word_bounds()
        .filter(|part| !part.trim().is_empty())
}

fn terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for word in text
        .unicode_words()
        .filter(|word| word.chars().any(char::is_alphabetic))
    {
        if word.chars().any(|ch| {
            matches!(
                ch.script(),
                Script::Han | Script::Hiragana | Script::Katakana | Script::Thai
            )
        }) {
            terms.extend(word.graphemes(true).map(str::to_lowercase));
        } else {
            terms.push(word.to_lowercase());
        }
    }
    terms
}

fn majority_shared(target: &[String], source: &[String]) -> bool {
    let mut words = HashMap::new();
    for term in target {
        *words.entry(term).or_insert(0_usize) += 1;
    }
    let mut shared = 0;
    for term in source {
        if let Some(count) = words.get_mut(term).filter(|count| **count > 0) {
            *count -= 1;
            shared += 1;
        }
    }
    shared > target.len().min(source.len()) / 2
}

#[cfg(test)]
mod tests {
    use super::Language;

    #[test]
    fn matching_foreign_text_cannot_license_untranslated_words_or_a_copied_sentence() {
        let language = Language::from_codes("ru", "en").unwrap();
        let target = "Please order the repairs at Ford before we leave for New York.";
        assert_eq!(
            [
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Ford перед нашим отъездом в New York.",
                    "order",
                    "заказать"
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Volvo перед нашим отъездом.",
                    "order",
                    "заказать"
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт перед нашим отъездом в York New.",
                    "order",
                    "заказать"
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Ford перед нашим отъездом.",
                    "Ford",
                    "автоконцерн"
                ),
                language.translates_sentence(
                    target,
                    "Срочно Please order the repairs at Ford before we leave.",
                    "New York",
                    "город"
                ),
            ],
            [true, false, false, false, false],
            "a missing or reordered name, untranslated study term or copied sentence passed the reader check"
        );
    }

    #[test]
    fn shared_writing_cannot_hide_most_of_the_learning_sentence_or_its_study_phrase() {
        let language = Language::from_codes("fr", "en").unwrap();
        let target = "We need to get the car fixed before the weekend.";
        assert_eq!(
            [
                language.translates_sentence(
                    target,
                    "Il faut faire réparer la voiture avant le week-end.",
                    "get the car fixed",
                    "faire réparer la voiture"
                ),
                language.translates_sentence(
                    target,
                    "Vite, we need to get the car fixed avant samedi.",
                    "weekend",
                    "fin de semaine"
                ),
                language.translates_sentence(
                    target,
                    "Il faut get the car fixed avant la fin de la semaine.",
                    "get the car fixed",
                    "faire réparer la voiture"
                ),
            ],
            [true, false, false],
            "a shared alphabet concealed most of the learning sentence or its study phrase"
        );
    }
}
