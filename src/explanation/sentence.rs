//! Local admission rule for the reader sentence on the front of a generated card.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use anyhow::Result;
use caseless::Caseless;
use kamishibai_reader_verbs::Language as VerbLanguage;
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use crate::languages::{
    ReaderVocabulary, StudyStems, catalog, explanation_scripts, native_scripted, unlisted_label,
};

/// The language pair used to validate either proposed version of the Anki front.
pub(crate) struct ReaderField {
    known: Vec<Script>,
    reader: &'static str,
    learning: &'static str,
}

impl ReaderField {
    /// Resolve the two supported languages for a generated card.
    pub(crate) fn from_codes(reader: &str, learning: &str) -> Result<Self> {
        let catalog = catalog();
        Ok(Self {
            known: explanation_scripts(reader),
            reader: catalog.borrowed(reader)?.code,
            learning: catalog.borrowed(learning)?.code,
        })
    }

    /// Validate a proposed front against the back, study expression and the card's approved translations.
    pub(crate) fn translates_sentence(
        &self,
        learning: &str,
        reader: &str,
        highlight: &str,
        term: &str,
        approved: &[&str],
    ) -> bool {
        let learning: String = learning.nfkc().collect();
        let reader: String = reader.nfkc().collect();
        let term: String = term.nfkc().collect();
        let target = tokens(&learning);
        let source = tokens(&reader);
        let study = tokens(&term);
        let study = StudyExpression::new(study, self.learning);
        if study.words.is_empty() || source.is_empty() || target.is_empty() {
            return false;
        }
        let approved: Vec<_> = approved.iter().map(|text| fold(text.trim())).collect();
        if target
            .iter()
            .map(|w| &w.folded)
            .eq(source.iter().map(|w| &w.folded))
        {
            return false;
        }
        let Some(shared) = study.designations(&target, &source, &reader) else {
            return false;
        };
        let highlight = fold(highlight.trim());
        let marked = tokens(&highlight);
        if !marked.is_empty()
            && marked
                .iter()
                .all(|word| shared.contains(word.folded.as_str()))
            && !approved.contains(&highlight)
            && !target
                .windows(study.words.len())
                .any(|part| study.matches_span(part))
        {
            return false;
        }
        let Some(ignored) = study.exemptions(&reader, &source, &approved, &shared, true) else {
            return false;
        };
        if study.words.len() > 1
            && source.iter().zip(&ignored).any(|(word, exempt)| {
                !exempt && study.contains(word) && target.iter().any(|part| part.shares_label(word))
            })
        {
            return false;
        }
        let ignored_target = study
            .exemptions(&learning, &target, &approved, &shared, false)
            .expect("invariant: the study sentence can contain its own expression");
        let original = retained(&target, &ignored_target);
        let translated = retained(&source, &ignored);
        if !translated
            .iter()
            .any(|w| w.chars().any(char::is_alphabetic))
            || !original.iter().any(|w| w.chars().any(char::is_alphabetic))
            || majority_shared(&original, &translated)
            || translated.iter().any(|w| !native_scripted(w, &self.known))
        {
            return false;
        }
        let gloss: Vec<_> = approved
            .iter()
            .flat_map(|text| tokens(text))
            .map(|word| word.folded)
            .collect();
        let vocabulary = ReaderVocabulary::new(self.reader, self.learning, &self.known);
        if translated
            .iter()
            .any(|w| !gloss.iter().any(|g| g == w) && vocabulary.foreign(w, original.contains(w)))
        {
            return false;
        }
        let residual = remaining(&reader, &source, &ignored);
        let names: Vec<_> = source
            .iter()
            .map(|word| shared.contains(word.folded.as_str()))
            .collect();
        vocabulary.recognizes(&residual, &remaining(&reader, &source, &names))
            || (ignored == names
                && translated.len() <= 3
                && translated.iter().all(|w| gloss.iter().any(|g| g == w)))
    }
}

struct Word<'a> {
    at: usize,
    text: &'a str,
    folded: String,
    infinitives: OnceCell<Vec<String>>,
}

impl Word<'_> {
    fn shares_label(&self, other: &Self) -> bool {
        self.folded == other.folded
            && (self.is_designation() || other.is_designation() || unlisted_label(&self.folded))
    }

    fn is_designation(&self) -> bool {
        self.text
            .chars()
            .any(|ch| ch.is_uppercase() || ch.is_numeric())
    }

    fn infinitives(&self, language: VerbLanguage) -> &[String] {
        self.infinitives
            .get_or_init(|| language.infinitives(&self.folded))
    }
}

fn fold(text: &str) -> String {
    text.nfkc().default_case_fold().nfc().collect()
}

fn tokens(text: &str) -> Vec<Word<'_>> {
    let mut words = Vec::new();
    for (at, word) in text.unicode_word_indices() {
        if word.chars().any(|ch| {
            matches!(
                ch.script(),
                Script::Han | Script::Hiragana | Script::Katakana | Script::Thai
            )
        }) {
            words.extend(word.grapheme_indices(true).map(|(offset, part)| Word {
                at: at + offset,
                text: part,
                folded: fold(part),
                infinitives: OnceCell::new(),
            }));
        } else {
            words.push(Word {
                at,
                text: word,
                folded: fold(word),
                infinitives: OnceCell::new(),
            });
        }
    }
    words
}

struct StudyExpression<'a> {
    words: Vec<Word<'a>>,
    morphology: StudyStems,
    language: Option<VerbLanguage>,
}

impl<'a> StudyExpression<'a> {
    fn new(words: Vec<Word<'a>>, code: &str) -> Self {
        Self {
            words,
            morphology: StudyStems::from_code(code),
            language: VerbLanguage::from_code(code),
        }
    }

    fn matches(&self, lemma: &Word<'_>, surface: &Word<'_>) -> bool {
        if lemma.folded == surface.folded
            || self.morphology.stem(&lemma.folded) == self.morphology.stem(&surface.folded)
        {
            return true;
        }
        if lemma.text.chars().next().map(|c| c.script())
            != surface.text.chars().next().map(|c| c.script())
        {
            return false;
        }
        self.language.is_some_and(|language| {
            let inflections = surface.infinitives(language);
            inflections.contains(&lemma.folded)
                || lemma
                    .infinitives(language)
                    .iter()
                    .any(|base| base == &surface.folded || inflections.contains(base))
        })
    }

    fn contains(&self, word: &Word<'_>) -> bool {
        self.words.iter().any(|lemma| self.matches(lemma, word))
    }

    fn matches_span(&self, span: &[Word<'_>]) -> bool {
        self.words
            .iter()
            .zip(span)
            .all(|(lemma, word)| self.matches(lemma, word))
    }

    fn designations<'text>(
        &self,
        target: &'text [Word<'_>],
        source: &[Word<'_>],
        reader: &str,
    ) -> Option<HashSet<&'text str>> {
        let shared: HashSet<_> = target
            .iter()
            .filter_map(|word| {
                (source.iter().any(|other| word.shares_label(other)) && !self.contains(word))
                    .then_some(word.folded.as_str())
            })
            .collect();
        for run in source.chunk_by(|a, b| {
            shared.contains(a.folded.as_str())
                && shared.contains(b.folded.as_str())
                && reader[a.at + a.text.len()..b.at]
                    .chars()
                    .all(char::is_whitespace)
        }) {
            if run.len() > 1
                && !target.windows(run.len()).any(|part| {
                    part.iter()
                        .zip(run)
                        .all(|(left, right)| left.folded == right.folded)
                })
            {
                return None;
            }
        }
        Some(shared)
    }

    fn exemptions(
        &self,
        text: &str,
        words: &[Word<'_>],
        approved: &[String],
        names: &HashSet<&str>,
        reader: bool,
    ) -> Option<Vec<bool>> {
        let mut masked: Vec<_> = words
            .iter()
            .map(|word| names.contains(word.folded.as_str()))
            .collect();
        for (start, part) in words.windows(self.words.len()).enumerate() {
            let last = &part[part.len() - 1];
            let span = &text[part[0].at..last.at + last.text.len()];
            let confirmed = approved.contains(&fold(span));
            if (reader || confirmed) && self.matches_span(part) {
                if !confirmed {
                    return None;
                }
                masked[start..start + part.len()].fill(true);
            }
        }
        Some(masked)
    }
}

fn retained<'words>(words: &'words [Word<'_>], skip: &[bool]) -> Vec<&'words str> {
    words
        .iter()
        .zip(skip)
        .filter_map(|(word, omit)| (!omit).then_some(word.folded.as_str()))
        .collect()
}

fn remaining(text: &str, words: &[Word<'_>], skip: &[bool]) -> String {
    let mut start = 0;
    let mut result = String::new();
    for (word, _) in words.iter().zip(skip).filter(|(_, omit)| **omit) {
        result.push_str(&text[start..word.at]);
        result.push(' ');
        start = word.at + word.text.len();
    }
    result.push_str(&text[start..]);
    result
}

fn majority_shared(target: &[&str], source: &[&str]) -> bool {
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
    use super::ReaderField;

    fn misclassified_designations(verdict: &str) -> Vec<&'static str> {
        include_str!("../../tests/fixtures/explanation/reader_designations.tsv")
            .lines()
            .filter(|row| {
                let [
                    expected,
                    reader,
                    learning,
                    term,
                    meaning,
                    back,
                    front,
                    highlight,
                ] = <[&str; 8]>::try_from(row.split('\t').collect::<Vec<_>>()).unwrap();
                expected == verdict
                    && ReaderField::from_codes(reader, learning)
                        .unwrap()
                        .translates_sentence(back, front, highlight, term, &[meaning])
                        != (expected == "allow")
            })
            .collect()
    }

    #[test]
    fn shared_names_cannot_conceal_study_forms_or_untranslated_words() {
        let leaked = misclassified_designations("deny");
        assert!(
            leaked.is_empty(),
            "untranslated words reached the front: {leaked:?}"
        );
    }

    #[test]
    fn shared_names_cannot_be_lost_to_case_or_punctuation() {
        let refused = misclassified_designations("allow");
        assert!(
            refused.is_empty(),
            "legitimate names blocked the translation: {refused:?}"
        );
    }

    #[test]
    fn reader_homographs_cannot_authorize_a_second_spelling_a_copy_or_a_foreign_sentence() {
        let field = ReaderField::from_codes("pt", "es").unwrap();
        let target = "Los gatos duermen esta tarde en la cocina.";
        let wrong: Vec<_> = [
            (
                "Os ｇａｔｏｓ estão dormindo hoje na cozinha.",
                "ＧＡＴＯＳ",
                true,
            ),
            ("Os gatos estão dormindo hoje na cozinha.", "gato", false),
            (
                "Os gatos estão dormindo hoje na cozinha.",
                "um gato ou muitos gatos",
                false,
            ),
            (
                "Os gatos viram um gato na cozinha esta manhã.",
                "gatos",
                false,
            ),
            ("O gatos walked.", "gatos", false),
            (target, "gatos", false),
            (
                "The gatos are sleeping in the kitchen this afternoon.",
                "gatos",
                false,
            ),
        ]
        .into_iter()
        .filter(|(front, gloss, expected)| {
            field.translates_sentence(target, front, "gatos", "gato", &[gloss]) != *expected
        })
        .collect();
        assert!(
            wrong.is_empty(),
            "the exact approved surface licensed an unrelated spelling or surrounding text: {wrong:?}"
        );
    }

    #[test]
    fn approving_a_homograph_cannot_supply_the_language_of_its_surrounding_words() {
        let field = ReaderField::from_codes("pt", "es").unwrap();
        assert!(
            !field.translates_sentence(
                "El gato duerme.",
                "The gato.",
                "gato",
                "gato",
                &["gato", "the"]
            ),
            "another gloss licensed English text around the approved Portuguese homograph"
        );
    }

    #[test]
    fn two_related_languages_cannot_hide_the_reader_evidence_in_a_short_translation() {
        let field = ReaderField::from_codes("uk", "en").unwrap();
        assert_eq!(
            [
                field.translates_sentence("The cat sleeps.", "Кіт спить.", "Кіт", "cat", &["кіт"]),
                field.translates_sentence("The cat sleeps.", "Кот спит.", "Кот", "cat", &["кіт"]),
            ],
            [true, false],
            "the reader vocabulary failed to distinguish the Ukrainian and Russian short sentences"
        );
    }

    #[test]
    fn a_shared_preposition_cannot_exclude_a_latin_reader_sentence() {
        let field = ReaderField::from_codes("la", "nl").unwrap();
        assert!(
            field.translates_sentence(
                "De kat slaapt de hele middag in de zon.",
                "Feles per totum pomeridium in sole dormit.",
                "Feles",
                "kat",
                &["feles"]
            ),
            "a shared preposition excluded a recorded Latin translation"
        );
    }

    #[test]
    fn a_portuguese_reader_cannot_lose_the_script_evidence_of_a_latin_sentence() {
        let field = ReaderField::from_codes("pt", "pl").unwrap();
        assert!(
            !field.translates_sentence(
                "Wczoraj rano moja siostra kupiła na dworcu dwa bilety na pociąg.",
                "Herī māne soror mea duās tesserās ferriviāriās in statiōne ēmit.",
                "ēmit",
                "kupić",
                &["comprou"],
            ),
            "discarding macrons misclassified a Latin sentence as Portuguese"
        );
    }

    #[test]
    fn a_dutch_plural_on_a_swedish_front_cannot_use_approval_for_a_different_form() {
        let field = ReaderField::from_codes("sv", "nl").unwrap();
        let target = "De kat slaapt.";
        assert_eq!(
            [
                field.translates_sentence(target, "Katten sover.", "Katten", "kat", &["katt"]),
                field.translates_sentence(target, "Katten sover.", "Katten", "kat", &["katten"]),
                field.translates_sentence(target, "En katt sover.", "katt", "kat", &["katt"]),
            ],
            [false, true, true],
            "the Swedish definite article hid the Dutch plural or lost its approved spelling"
        );
    }

    #[test]
    fn reader_homographs_cannot_lose_unicode_case_folding() {
        let field = ReaderField::from_codes("fr", "de").unwrap();
        let target = "Die Straße zum Bahnhof ist heute gesperrt.";
        let front = "Dans notre ville la STRASSE est fermée aux voitures depuis hier.";
        assert_eq!(
            [
                field.translates_sentence(target, front, "STRASSE", "Straße", &["Straße"]),
                field.translates_sentence(target, front, "STRASSE", "Straße", &["rue"])
            ],
            [true, false],
            "the card gloss did not control a full Unicode case-fold match"
        );
    }

    #[test]
    fn reader_fields_cannot_leak_learning_forms_or_admit_a_third_language_across_930_pairs() {
        let samples: Vec<_> = include_str!("../../tests/fixtures/explanation/reader_sentences.tsv")
            .lines()
            .map(|line| <[&str; 4]>::try_from(line.split('\t').collect::<Vec<_>>()).unwrap())
            .collect();
        let mut misses = Vec::new();
        let mut pairs = 0;
        for (index, &[known, _, native, reader]) in samples.iter().enumerate() {
            for &[study, term, inflected, target] in samples.iter().filter(|row| row[0] != known) {
                let language = ReaderField::from_codes(known, study).unwrap();
                let third = samples
                    .iter()
                    .cycle()
                    .skip(index + 1)
                    .take(samples.len())
                    .find(|row| {
                        row[0] != study
                            && row[0] != known
                            && crate::languages::explanation_scripts(row[0]) == language.known
                    })
                    .or_else(|| {
                        samples
                            .iter()
                            .find(|row| row[0] != study && row[0] != known)
                    })
                    .unwrap();
                let almost = target.replacen(inflected, native, 1);
                let embedded = reader.replacen(native, inflected, 1);
                for (kind, text, expected) in [
                    ("translation", reader, true),
                    ("copy", target, false),
                    ("near_copy", almost.as_str(), false),
                    ("inflection", embedded.as_str(), false),
                    ("other_language", third[3], false),
                ] {
                    if language.translates_sentence(
                        target,
                        text,
                        if expected { native } else { text },
                        term,
                        &[native],
                    ) != expected
                    {
                        misses.push(format!("{known}/{study}/{kind}"));
                    }
                }
                pairs += 1;
            }
        }
        assert_eq!(
            (pairs, misses),
            (930, Vec::new()),
            "the reader field misclassified these directed pairs"
        );
    }

    #[test]
    fn reader_fields_cannot_license_an_inflection_absent_from_the_learning_sentence() {
        let cases = [
            (
                "fr",
                "en",
                "go",
                "I go home before lunch.",
                "Je suis went chez moi avant midi.",
                "rentrer",
            ),
            (
                "fr",
                "en",
                "went",
                "She went to the library.",
                "Hier elle était gone avant mon arrivée.",
                "allée",
            ),
            (
                "en",
                "fr",
                "allait",
                "Elle allait souvent au marché.",
                "Tomorrow I irai to the market with my sister.",
                "was going",
            ),
            (
                "fr",
                "en",
                "take a walk",
                "I usually take a walk along the river.",
                "Hier nous took a walk le long de la rivière.",
                "se promener",
            ),
            (
                "en",
                "fr",
                "chanter",
                "Nous chantons dans la cour.",
                "Every day the children chantaient in the courtyard.",
                "sing",
            ),
            (
                "en",
                "fr",
                "aller",
                "Nous allons souvent au marché le matin.",
                "Tomorrow I irai to the market with my sister.",
                "go",
            ),
            (
                "fr",
                "en",
                "be",
                "They are already home.",
                "Hier il was très fatigué après sa journée.",
                "être",
            ),
            (
                "uk",
                "ru",
                "прогулка",
                "После прогулки мы идём домой.",
                "Він задоволений прогулками біля озера.",
                "прогулянка",
            ),
        ];
        let leaked: Vec<_> = cases
            .into_iter()
            .filter(|(known, learning, term, target, source, gloss)| {
                ReaderField::from_codes(known, learning)
                    .unwrap()
                    .translates_sentence(target, source, source, term, &[gloss])
            })
            .collect();
        assert!(
            leaked.is_empty(),
            "a different inflection of the study term reached the reader: {leaked:?}"
        );
    }

    #[test]
    fn a_regular_form_outside_the_exception_lexicon_cannot_reach_the_front() {
        let leaks: Vec<_> = [
            ("fr", "manger", "Il aime manger dehors.", "mangeons", "eat"),
            (
                "de",
                "arbeiten",
                "Sie arbeiten im Garten.",
                "arbeitetest",
                "work",
            ),
            (
                "de",
                "fragte",
                "Sie fragte den Nachbarn.",
                "fragen",
                "asked",
            ),
        ]
        .into_iter()
        .filter(|&(language, term, back, form, meaning)| {
            let front = format!("The children saw the word {form} in the garden this morning.");
            ReaderField::from_codes("en", language)
                .unwrap()
                .translates_sentence(back, &front, form, term, &[meaning])
        })
        .collect();
        assert!(
            leaks.is_empty(),
            "a regular study verb reached the front: {leaks:?}"
        );
    }

    #[test]
    fn a_capitalized_word_inside_a_study_phrase_cannot_be_exempted_as_a_name() {
        let field = ReaderField::from_codes("fr", "en").unwrap();
        let target = "Contact Ford when you arrive at the factory.";
        let leaks = ["contact Ford", "contact ford"].map(|term| {
            field.translates_sentence(
                target,
                "Ford me rappellera demain matin.",
                "rappellera",
                term,
                &["contacter la société"],
            )
        });
        assert_eq!(
            leaks,
            [false, false],
            "a proper name from the study phrase survived as an independent exception"
        );
    }

    #[test]
    fn shared_designations_cannot_license_a_token_of_the_study_expression() {
        let language = ReaderField::from_codes("ru", "en").unwrap();
        assert_eq!(
            [
                language.translates_sentence(
                    "The A320 landed at JFK.",
                    "В JFK приземлился A320.",
                    "приземлился",
                    "land",
                    &["приземлиться"]
                ),
                language.translates_sentence(
                    "Please contact Ford today.",
                    "Свяжитесь с Ford сегодня.",
                    "Свяжитесь с Ford",
                    "contact Ford",
                    &["связаться с компанией"]
                ),
                language.translates_sentence(
                    "I work at Ford.",
                    "Я работаю в Ford.",
                    "Ford",
                    "Ford",
                    &["Ford"]
                ),
            ],
            [true, false, true],
            "a designation lost its exception or smuggled a study token onto the front"
        );
    }

    #[test]
    fn short_sentences_with_names_cannot_hide_the_language_of_the_remaining_words() {
        let language = ReaderField::from_codes("fr", "en").unwrap();
        let target = "Ford acquired IBM.";
        assert_eq!(
            [
                language.translates_sentence(
                    target,
                    "Ford a racheté IBM.",
                    "a racheté",
                    "acquire",
                    &["racheter"]
                ),
                language.translates_sentence(
                    target,
                    "Ford bought IBM.",
                    "bought",
                    "acquire",
                    &["racheter"]
                ),
                language.translates_sentence(
                    target,
                    "Ford IBM.",
                    "Ford IBM",
                    "acquire",
                    &["racheter"]
                ),
            ],
            [true, false, false],
            "shared names outweighed the French or English predicate, or supplied the whole front"
        );
    }

    #[test]
    fn matching_foreign_text_cannot_license_untranslated_words_or_a_copied_sentence() {
        let language = ReaderField::from_codes("ru", "en").unwrap();
        let target = "Please order the repairs at Ford before we leave for New York.";
        assert_eq!(
            [
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Ford перед нашим отъездом в New York.",
                    "Закажи",
                    "order",
                    &["заказать"]
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Volvo перед нашим отъездом.",
                    "Закажи",
                    "order",
                    &["заказать"]
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт перед нашим отъездом в York New.",
                    "Закажи",
                    "order",
                    &["заказать"]
                ),
                language.translates_sentence(
                    target,
                    "Закажи ремонт в Ford перед нашим отъездом.",
                    "Ford",
                    "Ford",
                    &["автоконцерн"]
                ),
                language.translates_sentence(
                    target,
                    "Срочно Please order the repairs at Ford before we leave.",
                    "the repairs",
                    "New York",
                    &["город"]
                ),
            ],
            [true, false, false, false, false],
            "a missing or reordered name, untranslated study term or copied sentence passed the reader check"
        );
    }

    #[test]
    fn shared_writing_cannot_hide_most_of_the_learning_sentence_or_its_study_phrase() {
        let language = ReaderField::from_codes("fr", "en").unwrap();
        let target = "We need to get the car fixed before the weekend.";
        assert_eq!(
            [
                language.translates_sentence(
                    target,
                    "Il faut faire réparer la voiture avant le week-end.",
                    "faire réparer la voiture",
                    "get the car fixed",
                    &["faire réparer la voiture"]
                ),
                language.translates_sentence(
                    target,
                    "Vite, we need to get the car fixed avant samedi.",
                    "samedi",
                    "weekend",
                    &["fin de semaine"]
                ),
                language.translates_sentence(
                    target,
                    "Il faut get the car fixed avant la fin de la semaine.",
                    "get the car fixed",
                    "get the car fixed",
                    &["faire réparer la voiture"]
                ),
            ],
            [true, false, false],
            "a shared alphabet concealed most of the learning sentence or its study phrase"
        );
    }
}
