//! Reverse study verbs with the four productive paradigms provided by Ablaut.
mod reverse;

/// A study language with productive reverse verb analysis.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Language {
    /// Analyze German verbs.
    Deu,
    /// Analyze English verbs.
    Eng,
    /// Analyze French verbs.
    Fra,
    /// Analyze Spanish verbs.
    Spa,
}
use Language as Lang;

enum Conjugation {
    Deu(Box<ablaut::table::Table>),
    Eng(Box<ablaut::eng::Table>),
}

fn conjugate(lemma: &str, lang: Lang) -> Option<Conjugation> {
    match lang {
        Lang::Deu => ablaut::Verb::from_infinitive(lemma)
            .ok()
            .map(|verb| Conjugation::Deu(Box::new(ablaut::table::Table::build(&verb)))),
        Lang::Eng => ablaut::eng::Verb::from_infinitive(lemma)
            .ok()
            .map(|verb| Conjugation::Eng(Box::new(ablaut::eng::Table::build(&verb)))),
        Lang::Fra | Lang::Spa => {
            unreachable!("invariant: French and Spanish use their variant tables directly")
        }
    }
}

fn ord(lang: Lang) -> usize {
    match lang {
        Lang::Deu => 0,
        Lang::Eng => 1,
        Lang::Fra => 2,
        Lang::Spa => 3,
    }
}

fn lexicon_lemmas(lang: Lang) -> Vec<&'static str> {
    let lemmas = match lang {
        Lang::Deu => include_str!("lemmas/deu.txt"),
        Lang::Eng => include_str!("lemmas/eng.txt"),
        Lang::Fra => include_str!("lemmas/fra.txt"),
        Lang::Spa => include_str!("lemmas/spa.txt"),
    };
    lemmas.lines().collect()
}

impl Language {
    /// Select the paradigm for a canonical study-language code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "de" => Some(Self::Deu),
            "en" => Some(Self::Eng),
            "fr" => Some(Self::Fra),
            "es" => Some(Self::Spa),
            _ => None,
        }
    }

    /// Return all verified infinitives of one written word.
    #[must_use]
    pub fn infinitives(self, form: &str) -> Vec<String> {
        reverse::reverse(form, self)
            .into_iter()
            .map(|entry| entry.infinitive)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_selected_paradigms_cannot_lose_an_upstream_analysis() {
        let samples = [
            ("de", "arbeiten arbeitetest fragte fragen sein war widersprach angehalten abfahren führest geschwommen verringret"),
            ("en", "go went gone stop stopped running drove driven sunrose travelling tries taken flew lay"),
            ("fr", "manger mangeons allais irai retrouvait retrouverons appellerons paye vendons allées conduites abstiendra"),
            ("es", "hablaríamos estudiaba estudiaremos escribieron fueran dijeran dijesen inscritas devuelto averigüé conozco"),
        ];
        let missing: Vec<_> = samples
            .into_iter()
            .flat_map(|(code, words)| {
                words.split_whitespace().filter(move |word| {
                    let expected: Vec<_> =
                        ablaut::reverse(word, ablaut::Lang::from_code(code).unwrap())
                            .into_iter()
                            .map(|entry| entry.infinitive)
                            .collect();
                    super::Language::from_code(code).unwrap().infinitives(word) != expected
                })
            })
            .collect();
        assert!(
            missing.is_empty(),
            "reverse analyses diverged for {missing:?}"
        );
    }
}
