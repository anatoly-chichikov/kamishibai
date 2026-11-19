//! Offline language and word-form evidence for generated reader sentences.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, OnceLock};

use langidentify::alphabet::ALL_ALPHABETS;
use langidentify::{Alphabet, Detector, Language, Model};
use snowball_stemmers_rs::{Algorithm, Stemmer};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Languages whose vocabulary can occur in the reader's writing system.
pub(crate) struct ReaderVocabulary {
    detector: Option<Detector>,
    reader: Language,
    learning: Language,
}

impl ReaderVocabulary {
    /// Load the embedded models for the reader's script and select the card's languages.
    pub(crate) fn from_codes(known: &str, learning: &str) -> Self {
        let reader = Language::from_string(known);
        Self {
            detector: (!matches!(
                reader,
                Language::Korean
                    | Language::Japanese
                    | Language::ChineseSimplified
                    | Language::ChineseTraditional
            ))
            .then(|| detector(latin_profile(reader))),
            reader,
            learning: Language::from_string(learning),
        }
    }

    /// Decide whether the words left after removing designations read in the expected language.
    pub(crate) fn recognizes(&mut self, residual: &str, sentence: &str) -> bool {
        let has_homograph = residual != sentence;
        let mut words = residual.unicode_words();
        let single = words.next().filter(|_| words.next().is_none());
        let residual = single.unwrap_or_else(|| residual.trim());
        let Some(model) = self.detector.as_mut() else {
            return self.reader.iso_code().starts_with(secondary(residual));
        };
        let text = without_macrons(residual);
        let detected = model.detect(&text);
        if self.reader == Language::Serbian {
            return detected == Language::Croatian
                || (secondary(residual) == "hr" && secondary(&serbian_cyrillic(residual)) == "sr");
        }
        if detected == self.reader || secondary(residual) == self.reader.iso_code() {
            return true;
        }
        let Some(reader) = model.model().lang_index(self.reader) else {
            return false;
        };
        let Some(best) = model.model().lang_index(detected) else {
            return false;
        };
        let results = model.results();
        let distance = results.total_scores[best] - results.total_scores[reader];
        let hits = &results.scores.tw_hits_per_lang;
        let more_native_words = hits[reader] > hits[best];
        let close = distance <= std::f64::consts::LN_2;
        if has_homograph {
            let plausible = close
                || (single.is_some() && distance <= 3.0 * std::f64::consts::LN_2 && {
                    let ranking = identifier().rank(residual);
                    ranking[0].1 - ranking[1].1 <= 2.0 * std::f32::consts::LN_2
                });
            if plausible
                && (model.detect(sentence) == self.reader
                    || secondary(sentence) == self.reader.iso_code())
            {
                return true;
            }
        }
        if !close {
            return false;
        }
        if text.unicode_words().count() <= 3 && more_native_words {
            return true;
        }
        let ranking = identifier().rank(residual);
        if !ranking.iter().any(|(code, score)| {
            *code == self.reader.iso_code() && ranking[0].1 - score <= 2.0 * std::f32::consts::LN_2
        }) {
            return false;
        }
        let mut distinct = HashSet::new();
        text.unicode_words()
            .filter(|word| word.chars().count() >= 3 && distinct.insert(word.to_lowercase()))
            .filter(|word| model.detect(word) == self.reader)
            .take(2)
            .count()
            == 2
    }

    /// Reject a copied ordinary word when the language models attest it only on the learning side.
    pub(crate) fn foreign(&mut self, word: &str, copied: bool) -> bool {
        let Some(detector) = self
            .detector
            .as_mut()
            .filter(|_| copied && word.chars().count() > 2)
        else {
            return false;
        };
        let Some(study) = detector.model().lang_index(latin_profile(self.learning)) else {
            return false;
        };
        let reader = detector
            .model()
            .lang_index(latin_profile(self.reader))
            .expect("invariant: the reader model must be present");
        detector.detect(word);
        let hits = &detector.results().scores.tw_hits_per_lang;
        let mut native = hits[reader] > 0;
        let mut foreign = hits[study] > 0;
        if self.reader == Language::Latin || self.learning == Language::Latin {
            let unmarked = without_macrons(word);
            if unmarked != word {
                detector.detect(&unmarked);
                let hits = &detector.results().scores.tw_hits_per_lang;
                native |= self.reader == Language::Latin && hits[reader] > 0;
                foreign |= self.learning == Language::Latin && hits[study] > 0;
            }
        }
        foreign && !native
    }
}

fn detector(language: Language) -> Detector {
    static MODELS: LazyLock<HashMap<Alphabet, OnceLock<Arc<Model>>>> = LazyLock::new(|| {
        ALL_ALPHABETS
            .into_iter()
            .map(|alphabet| (alphabet, OnceLock::new()))
            .collect()
    });
    let alphabet = language.alphabets()[0];
    let model = MODELS[&alphabet].get_or_init(|| {
        let languages: Vec<_> = super::catalog()
            .codes()
            .into_iter()
            .map(Language::from_string)
            .chain([Language::Croatian])
            .filter(|language| language.uses_alphabet(alphabet))
            .collect();
        Arc::new(
            Model::load_lite(&languages)
                .expect("invariant: the embedded language models must load"),
        )
    });
    Detector::new(Arc::clone(model))
}

fn without_macrons(text: &str) -> String {
    text.nfd()
        .filter(|letter| *letter != '\u{0304}')
        .nfc()
        .collect()
}

fn secondary(text: &str) -> &'static str {
    identifier().classify(text).map_or("und", |(code, _)| code)
}

fn identifier() -> &'static langid_rs::Model {
    static MODEL: LazyLock<langid_rs::Model> = LazyLock::new(|| {
        let mut model = langid_rs::Model::load(false)
            .expect("invariant: the embedded language identifier must load");
        let codes = super::catalog()
            .codes()
            .map(str::to_owned)
            .into_iter()
            .chain([String::from("hr")])
            .collect();
        assert!(
            model.set_langs(Some(codes)).is_ok(),
            "invariant: the language identifier must cover every CLI language"
        );
        model
    });
    &MODEL
}

fn latin_profile(language: Language) -> Language {
    match language {
        Language::Serbian => Language::Croatian,
        model => model,
    }
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

/// Resolve the available Snowball rules for the language of the study expression.
pub(crate) fn stemmer(code: &str) -> Option<Stemmer> {
    use Algorithm as A;
    let algorithm = match Language::from_string(code) {
        Language::Arabic => A::Arabic,
        Language::Armenian => A::Armenian,
        Language::Czech => A::Czech,
        Language::Dutch => A::Dutch,
        Language::English => A::English,
        Language::Finnish => A::Finnish,
        Language::French => A::French,
        Language::German => A::German,
        Language::Greek => A::Greek,
        Language::Hindi => A::Hindi,
        Language::Hungarian => A::Hungarian,
        Language::Indonesian => A::Indonesian,
        Language::Italian => A::Italian,
        Language::Persian => A::Persian,
        Language::Polish => A::Polish,
        Language::Portuguese => A::Portuguese,
        Language::Romanian => A::Romanian,
        Language::Russian => A::Russian,
        Language::Serbian => A::Serbian,
        Language::Spanish => A::Spanish,
        Language::Swedish => A::Swedish,
        Language::Turkish => A::Turkish,
        Language::Ukrainian => A::Ukrainian,
        _ => return None,
    };
    Some(Stemmer::create(algorithm))
}
