//! Integration flow for `Your words -> detected target language -> What I understood`.
//!
//! This test is UI-agnostic and drives the contract layer only:
//! 1. A raw blob arrives from the first screen.
//! 2. A deterministic detector picks the target language.
//! 3. A mocked LLM pass produces confirmed word candidates.
//! 4. A persisted `my language` preference is picked up at batch start.
//! 5. A `LanguagePair` carries both directions into the next phase.
//!
//! No real LLM calls or network operations are performed.

use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::languages::catalog;
use kamishibai::session::{LanguagePair, ScriptDetection, TargetDetection, TargetGuess};
use tempfile::tempdir;

struct MockedUnderstanding {
    yielded: Vec<String>,
}

impl MockedUnderstanding {
    fn pass(&self, _raw: &str, _target: &str) -> Vec<String> {
        self.yielded.clone()
    }
}

#[test]
fn flow_resolves_pair_and_confirms_candidates_without_network_calls() {
    let preferences_home = tempdir().expect("must create preferences home");
    let store = PreferenceStore::at(
        preferences_home
            .path()
            .join("kamishibai")
            .join("preferences.json"),
    );
    store
        .write(&Preferences::new("ru"))
        .expect("must persist my language");
    let raw = "whilst\nat the end\nin the end\nwreck";
    let persisted = store.read().expect("must reload my language").my_language;
    let guess = ScriptDetection
        .detect(raw, &catalog())
        .expect("detection must succeed");
    let pair = LanguagePair::new(guess.code(), persisted.as_str());
    let candidates = MockedUnderstanding {
        yielded: vec![
            String::from("whilst"),
            String::from("at the end"),
            String::from("in the end"),
            String::from("wreck"),
        ],
    }
    .pass(raw, pair.target());
    assert_eq!(
        (guess, pair.label(), candidates.len(),),
        (TargetGuess::new("en", false), String::from("EN → RU"), 4,),
        "flow must detect target, honor persisted support language, and forward confirmed candidates"
    );
}

#[test]
fn flow_uses_english_support_on_first_run() {
    let preferences_home = tempdir().expect("must create preferences home");
    let store = PreferenceStore::at(
        preferences_home
            .path()
            .join("kamishibai")
            .join("preferences.json"),
    );
    let persisted = store
        .read()
        .expect("missing file must collapse to defaults")
        .my_language;
    let pair = LanguagePair::new("ru", persisted.as_str());
    assert_eq!(
        pair.label(),
        "RU → EN",
        "first-run my language must surface as English in the session pair"
    );
}
