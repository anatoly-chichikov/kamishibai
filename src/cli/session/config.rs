//! The `config` verb: persist or show the user's preferences (known language and
//! Gemini API key) from the console, so neither has to be re-supplied every run
//! nor set through the interactive TUI Welcome. With no flags it shows the saved
//! state; `--known`/`--key` save. The key value is never printed back.

use std::io::Read;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::error::usage;
use crate::config::{PreferenceStore, Preferences, default_store};
use crate::gemini::{GeminiClient, HttpTransport};
use crate::runtime::locations::SystemContext;

use super::args::ConfigArgs;
use super::{Render, json, validate_language};

/// Save the supplied preferences, or show the current ones when no flag is given.
pub(super) fn config(args: &ConfigArgs, render: Render) -> Result<()> {
    let store = default_store(&SystemContext)?;
    let setting = args.known.is_some() || args.key.is_some();
    let prefs = if setting {
        apply(args, &store)?
    } else {
        store.read().unwrap_or_default()
    };
    if matches!(render, Render::Json) {
        return json::emit(&ConfigDoc::of(&prefs));
    }
    if setting {
        report_set(args, &prefs);
    } else {
        report_show(&prefs);
    }
    Ok(())
}

/// Read current preferences, apply each supplied flag, and persist once.
fn apply(args: &ConfigArgs, store: &PreferenceStore) -> Result<Preferences> {
    let mut prefs = store.read().unwrap_or_default();
    if let Some(code) = args.known.as_deref() {
        validate_language(code)?;
        prefs = prefs.adopt(code);
    }
    if let Some(key) = args.key.as_deref() {
        prefs = apply_key(prefs, key)?;
    }
    store.write(&prefs)?;
    Ok(prefs)
}

/// Apply one `--key` value: empty clears the saved key, otherwise the key is
/// verified against Gemini before it is stored (a rejected key is refused).
fn apply_key(prefs: Preferences, key: &str) -> Result<Preferences> {
    let key = read_key(key)?;
    if key.is_empty() {
        return Ok(prefs.without_api_key());
    }
    GeminiClient::new(key.as_str(), HttpTransport::new())
        .validate_key()
        .map_err(|error| usage(format!("could not verify Gemini key: {error}")))?;
    Ok(prefs.with_api_key(key))
}

/// Resolve a `--key` argument to its literal value, reading stdin for `-` so the
/// key never has to appear in argv or the shell history.
fn read_key(key: &str) -> Result<String> {
    if key != "-" {
        return Ok(String::from(key));
    }
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .context("reading Gemini key from stdin")?;
    Ok(buffer.trim().to_string())
}

/// Print a human confirmation of what was saved; the language code also goes to
/// stdout so it stays capturable, and the key value is never echoed.
fn report_set(args: &ConfigArgs, prefs: &Preferences) {
    if let Some(code) = args.known.as_deref() {
        eprintln!("saved your language: {code}");
    }
    if args.key.is_some() {
        if prefs.api_key.is_some() {
            eprintln!("saved your Gemini API key");
        } else {
            eprintln!("cleared your Gemini API key");
        }
    }
    if let Some(code) = args.known.as_deref() {
        println!("{code}");
    }
}

/// Print the saved preferences as a readable block; the key is reported only as
/// present or absent.
fn report_show(prefs: &Preferences) {
    let hint = if prefs.requires_language_choice() {
        " (not set; defaulting)"
    } else {
        ""
    };
    let key = if prefs.api_key.is_some() {
        "saved"
    } else {
        "not saved"
    };
    println!("language  {}{hint}", prefs.startup_language());
    println!("key       {key}");
}

/// The `--json` projection of the saved preferences; it carries `key_saved`, never the key.
#[derive(Serialize)]
struct ConfigDoc {
    ok: bool,
    known: String,
    confirmed: bool,
    key_saved: bool,
}

impl ConfigDoc {
    /// Project current preferences into the JSON document.
    fn of(prefs: &Preferences) -> Self {
        Self {
            ok: true,
            known: prefs.startup_language().to_string(),
            confirmed: !prefs.requires_language_choice(),
            key_saved: prefs.api_key.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(dir: &TempDir) -> PreferenceStore {
        PreferenceStore::at(dir.path().join("preferences.json"))
    }

    #[test]
    fn saving_a_known_language_persists_it_confirmed() {
        let dir = TempDir::new().expect("tempdir must be created");
        let store = store(&dir);
        let args = ConfigArgs {
            known: Some(String::from("ru")),
            key: None,
        };
        apply(&args, &store).expect("saving the language must succeed");
        let restored = store.read().expect("read must succeed");
        assert_eq!(
            (
                restored.my_language.as_str(),
                restored.requires_language_choice()
            ),
            ("ru", false),
            "saving --known must persist the language as a confirmed choice"
        );
    }

    #[test]
    fn an_unknown_language_is_refused() {
        let dir = TempDir::new().expect("tempdir must be created");
        let store = store(&dir);
        let args = ConfigArgs {
            known: Some(String::from("zz")),
            key: None,
        };
        assert!(
            apply(&args, &store).is_err(),
            "an unknown language code must be refused, never saved"
        );
    }

    #[test]
    fn an_empty_key_clears_it_without_a_network_call() {
        let dir = TempDir::new().expect("tempdir must be created");
        let store = store(&dir);
        store
            .write(&Preferences::new("ru").with_api_key("secret"))
            .expect("seeding a saved key must succeed");
        let args = ConfigArgs {
            known: None,
            key: Some(String::new()),
        };
        apply(&args, &store).expect("clearing the key must succeed offline");
        let restored = store.read().expect("read must succeed");
        assert!(
            restored.api_key.is_none(),
            "an empty --key must clear the saved key without a network call"
        );
    }

    #[test]
    fn the_document_never_exposes_the_key_value() {
        let prefs = Preferences::new("ru").with_api_key("super-secret");
        let document =
            serde_json::to_string(&ConfigDoc::of(&prefs)).expect("serialize must succeed");
        assert!(
            !document.contains("super-secret"),
            "the config document must report key_saved, never the key value"
        );
    }
}
