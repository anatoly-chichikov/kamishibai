//! Shared-script language profiles built from the lite n-gram and vocabulary tables.
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::LazyLock;
use unicode_script::Script;
use unicode_segmentation::UnicodeSegmentation;

type Table = HashMap<String, Vec<(usize, f64)>>;

/// Character statistics and word frequencies for one shared writing system.
pub(super) struct Profiles {
    codes: Vec<String>,
    grams: Table,
    words: Table,
}

/// Scores and dictionary hits for one sentence across the available languages.
pub(super) struct Reading {
    /// Highest-scoring language when the text supplied positive evidence.
    pub(super) best: Option<usize>,
    /// Combined character and word scores in profile order.
    pub(super) scores: Vec<f64>,
    /// Number of word hits in each language's frequency table.
    pub(super) words: Vec<u32>,
}

impl Profiles {
    fn read(bytes: &[u8]) -> Self {
        let mut lines = BufReader::new(GzDecoder::new(bytes))
            .lines()
            .map(|line| line.expect("invariant: embedded reader data must decode"));
        let codes = lines
            .next()
            .expect("invariant: a reader profile must name its languages")
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let mut grams = Table::new();
        let mut words = Table::new();
        for line in lines {
            let (key, values) = line
                .split_once('\t')
                .expect("invariant: an embedded reader row must contain weights");
            let weights = values
                .split(',')
                .map(|entry| {
                    let (language, weight) = entry
                        .split_once(':')
                        .expect("invariant: each reader weight must name a language");
                    (
                        language
                            .parse()
                            .expect("invariant: reader language indices must be numeric"),
                        f64::from(
                            weight
                                .parse::<u16>()
                                .expect("invariant: reader weights must be integers"),
                        ) / 256.0,
                    )
                })
                .collect();
            let (kind, key) = key.split_at(1);
            let table = if kind == "g" { &mut grams } else { &mut words };
            table.insert(key.to_owned(), weights);
        }
        Self {
            codes,
            grams,
            words,
        }
    }

    /// Resolve the statistical profiles only for writing systems shared by CLI languages.
    pub(super) fn for_scripts(scripts: &[Script]) -> Option<&'static Self> {
        static LATIN: LazyLock<Profiles> =
            LazyLock::new(|| Profiles::read(include_bytes!("data/latin.gz")));
        static CYRILLIC: LazyLock<Profiles> =
            LazyLock::new(|| Profiles::read(include_bytes!("data/cyrillic.gz")));
        static ARABIC: LazyLock<Profiles> =
            LazyLock::new(|| Profiles::read(include_bytes!("data/arabic.gz")));
        match scripts {
            [Script::Arabic] => Some(&ARABIC),
            [Script::Cyrillic] => Some(&CYRILLIC),
            [Script::Latin] => Some(&LATIN),
            _ => None,
        }
    }

    /// Locate a language in this script's score vector.
    pub(super) fn index(&self, code: &str) -> Option<usize> {
        self.codes.iter().position(|c| c == code)
    }

    /// Return the trained language code at a score position.
    pub(super) fn code(&self, index: usize) -> &str {
        &self.codes[index]
    }

    /// Look up a single copied word in the frequency table for both card languages.
    pub(super) fn contains(&self, word: &str, index: usize) -> bool {
        self.words
            .get(word)
            .is_some_and(|weights| weights.iter().any(|(language, _)| *language == index))
    }

    /// Combine character n-gram and word likelihoods across this writing system.
    pub(super) fn read_text(&self, text: &str) -> Reading {
        let mut grams = Evidence::new(self.codes.len());
        let mut words = Evidence::new(self.codes.len());
        let mut examined = 0u32;
        let lowercase = text.to_lowercase().replace('’', "'");
        for word in lowercase.unicode_words().filter(|word| {
            word.chars().all(|c| {
                c.is_alphabetic() || c == '\'' || unicode_normalization::char::is_combining_mark(c)
            })
        }) {
            let mut offsets: Vec<_> = word.char_indices().map(|(at, _)| at).collect();
            let length = offsets.len();
            offsets.push(word.len());
            if length > 1 || !word.is_ascii() {
                examined += 1;
                if !words.record(self.words.get(word))
                    && let Some((before, after)) = word.split_once('\'')
                {
                    words.record(self.words.get(before));
                    words.record(self.words.get(after));
                }
            }
            for size in (1..=length.min(5)).rev() {
                let mut covered = true;
                for start in 0..=length - size {
                    covered &=
                        grams.record(self.grams.get(&word[offsets[start]..offsets[start + size]]));
                }
                if covered && size <= 3 {
                    break;
                }
            }
        }
        let best_word = words.normalize();
        grams.normalize();
        let ratio = f64::from(best_word) / f64::from(examined.max(1));
        let factor = ratio * if ratio >= 0.5 { 0.8 } else { 0.5 };
        let scores: Vec<_> = grams
            .sums
            .iter()
            .zip(&words.sums)
            .map(|(g, w)| g * (1.0 - factor) + w * factor)
            .collect();
        let best = (0..scores.len())
            .min_by(|&a, &b| scores[b].total_cmp(&scores[a]))
            .filter(|&language| scores[language] > 0.0);
        Reading {
            best,
            scores,
            words: words.hits,
        }
    }
}

struct Evidence {
    sums: Vec<f64>,
    hits: Vec<u32>,
}

impl Evidence {
    fn new(languages: usize) -> Self {
        Self {
            sums: vec![0.0; languages],
            hits: vec![0; languages],
        }
    }
    fn record(&mut self, weights: Option<&Vec<(usize, f64)>>) -> bool {
        if let Some(weights) = weights {
            for &(language, weight) in weights {
                self.sums[language] += weight;
                self.hits[language] += 1;
            }
            true
        } else {
            false
        }
    }
    fn normalize(&mut self) -> u32 {
        let most = self.hits.iter().copied().max().unwrap_or(0);
        for sum in &mut self.sums {
            *sum /= f64::from(most.max(1));
        }
        most
    }
}

#[cfg(test)]
mod tests {
    use super::Profiles;
    use unicode_script::Script;

    #[test]
    fn an_empty_reading_cannot_choose_the_first_language_in_a_script() {
        assert_eq!(
            Profiles::for_scripts(&[Script::Latin])
                .unwrap()
                .read_text("...")
                .best,
            None,
            "an empty reading picked a language without evidence"
        );
    }
}
