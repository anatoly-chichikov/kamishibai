//! Persistent user preferences.

mod preferences;
mod store;

pub use preferences::Preferences;
pub use store::{PreferenceStore, PreferenceStoreError, default_store};

pub(crate) const DEFAULT_MY_LANGUAGE: &str = "en";
