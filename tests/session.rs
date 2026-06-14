//! Target-language detection and language pair contracts.

use kamishibai::languages::catalog;
use kamishibai::session::{LanguagePair, LearningDetection, LearningGuess, ScriptDetection};

#[test]
fn script_detection_picks_russian_on_cyrillic_dominant_blob() {
    let guess = ScriptDetection
        .detect("окно, стол, дождь", &catalog())
        .expect("detection must succeed");
    assert_eq!(
        guess,
        LearningGuess::new("ru", true),
        "cyrillic-dominant blob must yield a confident Russian guess"
    );
}

#[test]
fn script_detection_falls_back_to_english_for_latin_input() {
    let guess = ScriptDetection
        .detect("whilst\nat the end\nin the end\nwreck", &catalog())
        .expect("detection must succeed");
    assert_eq!(
        guess,
        LearningGuess::new("en", false),
        "Latin-script blob must surface an unsure English guess for later LLM confirmation"
    );
}

#[test]
fn script_detection_handles_mixed_chinese() {
    let guess = ScriptDetection
        .detect("日本 中国 学校", &catalog())
        .expect("detection must succeed");
    assert_eq!(
        guess,
        LearningGuess::new("zh", true),
        "Han-script blob must produce a confident Chinese guess"
    );
}

#[test]
fn language_pair_label_renders_uppercase_arrow() {
    let pair = LanguagePair::new("en", "ru");
    assert_eq!(
        pair.label(),
        "RU → EN",
        "language pair label must render target → support in uppercase"
    );
}
