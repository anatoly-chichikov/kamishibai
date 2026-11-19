//! Offline language and word-form evidence for generated reader sentences.
mod byte_ngrams;
mod profile;

use profile::Profiles;
use snowball_stemmers_rs::{Algorithm, Stemmer};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Vocabulary for distinguishing the reader from other languages with the same writing system.
pub(crate) struct ReaderVocabulary<'a> {
    profiles: Option<&'static Profiles>,
    reader: &'a str,
    learning: &'a str,
}

impl<'a> ReaderVocabulary<'a> {
    /// Select only the models for the reader's script.
    #[must_use]
    pub(crate) fn new(reader: &'a str, learning: &'a str, scripts: &[Script]) -> Self {
        Self {
            profiles: Profiles::for_scripts(scripts),
            reader,
            learning,
        }
    }
    /// Verify the language after shared names and approved homographs have been removed.
    #[must_use]
    pub(crate) fn recognizes(&self, residual: &str, sentence: &str) -> bool {
        let homograph = residual != sentence;
        let mut words = residual.unicode_words();
        let single = words.next().filter(|_| words.next().is_none());
        let residual = single.unwrap_or_else(|| residual.trim());
        let Some(profiles) = self.profiles else {
            return match self.reader {
                "ja" if residual
                    .chars()
                    .any(|c| matches!(c.script(), Script::Hiragana | Script::Katakana)) =>
                {
                    true
                }
                "ko" if residual.chars().any(|c| c.script() == Script::Hangul) => true,
                "ja" | "zh" | "ko" => winner(residual) == self.reader,
                _ => true,
            };
        };
        let text = without_macrons(residual);
        let reading = profiles.read_text(&text);
        let best = reading.best.map(|index| profiles.code(index));
        if self.reader == "sr" {
            return best == Some("hr")
                || (winner(residual) == "hr" && winner(&serbian_cyrillic(residual)) == "sr");
        }
        if best == Some(self.reader) {
            return true;
        }
        let ranking = byte_ngrams::rank(residual);
        if ranking[0].0 == self.reader {
            return true;
        }
        let Some(leading) = reading.best else {
            return false;
        };
        let reader = profiles
            .index(self.reader)
            .expect("invariant: the reader must have a profile in its script");
        let distance = reading.scores[leading] - reading.scores[reader];
        let close = distance <= std::f64::consts::LN_2;
        if homograph {
            let plausible = close
                || (single.is_some()
                    && distance <= 3.0 * std::f64::consts::LN_2
                    && ranking[0].1 - ranking[1].1 <= 2.0 * std::f32::consts::LN_2);
            if plausible
                && (profiles.read_text(sentence).best == Some(reader)
                    || winner(sentence) == self.reader)
            {
                return true;
            }
        }
        if !close {
            return false;
        }
        if text.unicode_words().count() <= 3 && reading.words[reader] > reading.words[leading] {
            return true;
        }
        if !ranking.iter().any(|(code, score)| {
            *code == self.reader && ranking[0].1 - score <= 2.0 * std::f32::consts::LN_2
        }) {
            return false;
        }
        let mut distinct = HashSet::new();
        text.unicode_words()
            .filter(|word| word.chars().count() >= 3 && distinct.insert(word.to_lowercase()))
            .filter(|word| profiles.read_text(word).best == Some(reader))
            .take(2)
            .count()
            == 2
    }

    /// Test whether a copied word is attested only on the learning side.
    #[must_use]
    pub(crate) fn foreign(&self, word: &str, copied: bool) -> bool {
        let Some(profiles) = self.profiles.filter(|_| copied && word.chars().count() > 2) else {
            return false;
        };
        let index = |code: &str| profiles.index(if code == "sr" { "hr" } else { code });
        let Some(study) = index(self.learning) else {
            return false;
        };
        let reader =
            index(self.reader).expect("invariant: the reader must have a profile in its script");
        let attested = |lang: &str, i| {
            profiles.contains(word, i)
                || (lang == "la" && profiles.contains(&without_macrons(word), i))
        };
        attested(self.learning, study) && !attested(self.reader, reader)
    }
}

/// Recognize an uncapitalized label such as "npm" when no word profile knows its spelling.
pub(crate) fn unlisted_label(word: &str) -> bool {
    let Some(letter) = word.chars().find(|c| c.is_lowercase()) else {
        return false;
    };
    Profiles::for_scripts(&[letter.script()])
        .is_some_and(|profile| !profile.lists(word) && !profile.lists(&without_macrons(word)))
}

fn winner(text: &str) -> &'static str {
    byte_ngrams::rank(text)[0].0
}
fn without_macrons(text: &str) -> String {
    text.nfd()
        .filter(|letter| *letter != '\u{0304}')
        .nfc()
        .collect()
}
fn serbian_cyrillic(text: &str) -> String {
    let text = text
        .to_lowercase()
        .replace("dž", "џ")
        .replace("lj", "љ")
        .replace("nj", "њ");
    text.chars()
        .map(|letter| {
            "abcčćdđefghijklmnoprsštuvzž"
                .chars()
                .zip("абцчћдђефгхијклмнопрсштувзж".chars())
                .find_map(|(latin, cyrillic)| (latin == letter).then_some(cyrillic))
                .unwrap_or(letter)
        })
        .collect()
}

/// Match study words by their Snowball stems, including paradigms that change stems.
pub(crate) struct StudyStems {
    snowball: Option<Stemmer>,
    alternates: HashMap<String, &'static str>,
}

impl StudyStems {
    /// Index alternate stems of the same paradigm in the selected study language.
    pub(crate) fn from_code(code: &str) -> Self {
        let snowball = stemmer(code);
        let mut alternates = HashMap::new();
        for group in paradigms(code) {
            for word in *group {
                let stem = snowball
                    .as_ref()
                    .map_or(Cow::Borrowed(*word), |s| s.stem(word));
                alternates.insert(stem.into_owned(), group[0]);
            }
        }
        Self {
            snowball,
            alternates,
        }
    }

    /// Return the same key for any recognized member of a study-word paradigm.
    pub(crate) fn stem<'word>(&self, word: &'word str) -> Cow<'word, str> {
        let stem = self
            .snowball
            .as_ref()
            .map_or(Cow::Borrowed(word), |s| s.stem(word));
        self.alternates
            .get(stem.as_ref())
            .copied()
            .map_or(stem, Cow::Borrowed)
    }
}

fn paradigms(code: &str) -> &'static [&'static [&'static str]] {
    match code {
        "cs" => &[&[
            "být", "jsem", "jsi", "je", "jsme", "jste", "jsou", "byl", "byla", "bylo", "byli",
            "byly", "budu", "bude",
        ]],
        "de" => &[&["datum", "daten"]],
        "en" => &[
            &["person", "people"],
            &["child", "children"],
            &["man", "men"],
            &["woman", "women"],
            &["mouse", "mice"],
            &["goose", "geese"],
            &["tooth", "teeth"],
            &["foot", "feet"],
        ],
        "fr" => &[&["œil", "yeux"]],
        "it" => &[
            &["buono", "buona", "migliore", "migliori"],
            &["cattivo", "peggiore", "peggiori"],
        ],
        "nl" => &[&["zijn", "ben", "bent", "is", "was", "waren", "geweest"]],
        "pl" => &[&[
            "człowiek",
            "ludzie",
            "ludzi",
            "ludziom",
            "ludźmi",
            "ludziach",
        ]],
        "ru" => &[&["человек", "люди", "людей", "людям", "людьми", "людях"]],
        "tr" => &[&["gitmek", "git", "gidiyor", "gider", "gitti", "gidecek"]],
        _ => &[],
    }
}

fn stemmer(code: &str) -> Option<Stemmer> {
    use Algorithm as A;
    let algorithm = match code {
        "ar" => A::Arabic,
        "hy" => A::Armenian,
        "cs" => A::Czech,
        "nl" => A::Dutch,
        "en" => A::English,
        "fi" => A::Finnish,
        "fr" => A::French,
        "de" => A::German,
        "el" => A::Greek,
        "hi" => A::Hindi,
        "hu" => A::Hungarian,
        "id" => A::Indonesian,
        "it" => A::Italian,
        "fa" => A::Persian,
        "pl" => A::Polish,
        "pt" => A::Portuguese,
        "ro" => A::Romanian,
        "ru" => A::Russian,
        "sr" => A::Serbian,
        "es" => A::Spanish,
        "sv" => A::Swedish,
        "tr" => A::Turkish,
        "uk" => A::Ukrainian,
        _ => return None,
    };
    Some(Stemmer::create(algorithm))
}

#[cfg(test)]
mod tests {
    use super::{Profiles, byte_ngrams};
    use crate::languages::{catalog, explanation_scripts};
    use unicode_script::Script;

    #[test]
    fn a_shared_writing_system_cannot_leave_a_reader_without_a_model() {
        let codes = catalog().codes();
        let classes = byte_ngrams::rank("");
        let missing: Vec<_> = codes
            .iter()
            .copied()
            .filter(|&code| {
                let scripts = explanation_scripts(code);
                let shared = codes.iter().any(|&other| {
                    other != code
                        && explanation_scripts(other)
                            .iter()
                            .any(|script| scripts.contains(script))
                });
                let word = if code == "sr" { "hr" } else { code };
                let profile = Profiles::for_scripts(&scripts);
                let has_words = profile.is_some_and(|table| table.index(word).is_some());
                let has_bytes = classes.iter().any(|&(class, _)| class == code);
                has_bytes != shared || has_words != (shared && !scripts.contains(&Script::Han))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "reader coverage disagreed with the catalog scripts for {missing:?}"
        );
    }
}
