//! Persisted user preference round-trip.

use std::fs;

use kamishibai::config::{PreferenceStore, Preferences};
use tempfile::tempdir;

#[test]
fn preference_store_persists_my_language_across_reads() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("ru"))
        .expect("write must succeed");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        restored,
        Preferences::new("ru"),
        "persisted my_language must survive a round trip"
    );
}

#[test]
fn preference_store_reports_default_english_on_first_run() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("missing.json"));
    let fresh = store.read().expect("read must succeed");
    assert_eq!(
        fresh,
        Preferences::default(),
        "missing preference file must collapse to English default"
    );
}

#[test]
fn stored_api_key_does_not_confirm_the_default_language() {
    let preferences = Preferences::default().with_api_key("123456789012345678901234567890");
    assert!(
        preferences.requires_language_choice(),
        "saving an API key alone must not mark the language as user-confirmed"
    );
}

#[test]
fn clearing_api_key_preserves_the_confirmed_language() {
    let preferences = Preferences::new("ru")
        .with_api_key("123456789012345678901234567890")
        .without_api_key();
    assert_eq!(
        (
            preferences.my_language,
            preferences.my_language_confirmed,
            preferences.api_key,
        ),
        (String::from("ru"), true, None),
        "clearing a rejected API key must not reset the confirmed language"
    );
}

#[test]
fn legacy_preference_without_confirmation_cannot_silently_pick_german() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path().parent().expect("store must have parent"))
        .expect("parent must be writable");
    fs::write(
        store.path(),
        r#"{"my_language":"de","api_key":"123456789012345678901234567890"}"#,
    )
    .expect("legacy preference must be writable");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        (
            restored.requires_language_choice(),
            restored.startup_language().to_string(),
        ),
        (true, String::from("en")),
        "legacy preferences without confirmation must not silently select German"
    );
}

#[test]
fn preferences_default_uses_english() {
    assert_eq!(
        Preferences::default().my_language,
        "en",
        "first-run my_language must default to English"
    );
}
