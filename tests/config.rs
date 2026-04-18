//! Persisted user preference round-trip.

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
fn preferences_default_uses_english() {
    assert_eq!(
        Preferences::default().my_language,
        "en",
        "first-run my_language must default to English"
    );
}
