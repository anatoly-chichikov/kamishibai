//! The `config` verb: persist or show the user's preferences (known language and
//! Gemini API key) from the console, so neither has to be re-supplied every run
//! nor set through the interactive TUI Welcome. With no flags it shows the saved
//! state; `--known`/`--key` save. The key value is never printed back.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::error::{operational_hint, operational_retryable_hint, usage_hint};
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
        store.read()?
    };
    if matches!(render, Render::Json) {
        return json::emit(&ConfigDoc::of(&prefs, store.path()));
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
    let known = match args.known.as_deref() {
        Some(code) => {
            validate_language(code)?;
            Some(code.to_uppercase())
        }
        None => None,
    };
    store.read()?;
    let key = KeyChange::resolve(args.key.as_deref())?;
    Ok(store.update(|prefs| {
        let prefs = match known.as_deref() {
            Some(code) => prefs.adopt(code),
            None => prefs,
        };
        key.apply(prefs)
    })?)
}

enum KeyChange {
    Unchanged,
    Clear,
    Save(String),
}

impl KeyChange {
    fn resolve(key: Option<&str>) -> Result<Self> {
        let Some(key) = key else {
            return Ok(Self::Unchanged);
        };
        let key = read_key(key)?;
        if key.is_empty() {
            return Ok(Self::Clear);
        }
        GeminiClient::new(key.as_str(), HttpTransport::credential())
            .probe_key()
            .map_err(credential_error)?;
        Ok(Self::Save(key))
    }

    fn apply(&self, prefs: Preferences) -> Preferences {
        match self {
            Self::Unchanged => prefs,
            Self::Clear => prefs.without_api_key(),
            Self::Save(key) => prefs.with_api_key(key.as_str()),
        }
    }
}

fn credential_error(error: crate::gemini::CredentialProbeError) -> anyhow::Error {
    if error.rejects_key() {
        return usage_hint(
            error.to_string(),
            "Check the key and retry with: kamishibai config --key - --json",
        );
    }
    if error.retryable() {
        return operational_retryable_hint(
            error.to_string(),
            "Retry later without changing the saved preferences",
        );
    }
    if error.model_unavailable() {
        return operational_hint(
            error.to_string(),
            "Update Kamishibai or verify that the configured model is enabled",
        );
    }
    operational_hint(
        error.to_string(),
        "Retry after checking Gemini service availability",
    )
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

/// Print a plain-language confirmation of what was saved; the key value is never
/// echoed, and the language is reported in its canonical uppercase form.
fn report_set(args: &ConfigArgs, prefs: &Preferences) {
    if args.known.is_some() {
        println!(
            "Saved {} as your known language — new won't ask for --known anymore.",
            prefs.startup_language().to_uppercase()
        );
    }
    if args.key.is_some() {
        if prefs.api_key.is_some() {
            println!(
                "Checked your key against Gemini and saved it — you won't need to export GEMINI_API_KEY."
            );
        } else {
            println!("Cleared your saved Gemini API key.");
        }
    }
}

/// Print the saved preferences as two plain lines; each unsaved setting carries
/// the one command that saves it. The key is reported only as present or absent.
fn report_show(prefs: &Preferences) {
    if prefs.requires_language_choice() {
        println!(
            "Known language: not saved — new keeps asking for --known (kamishibai config --known en)"
        );
    } else {
        println!(
            "Known language: {}",
            prefs.startup_language().to_uppercase()
        );
    }
    if prefs.api_key.is_some() {
        println!("Gemini API key: saved");
    } else {
        println!(
            "Gemini API key: not saved — or export GEMINI_API_KEY (kamishibai config --key -)"
        );
    }
}

/// The `--json` projection of the saved preferences: the canonical uppercase
/// `known` (`null` until one is saved) and `key_saved`, never the key value.
#[derive(Serialize)]
struct ConfigDoc {
    ok: bool,
    known: Option<String>,
    key_saved: bool,
    credential_source: CredentialSource,
    credential_present: bool,
    preferences_path: String,
}

impl ConfigDoc {
    /// Project current preferences into the JSON document.
    fn of(prefs: &Preferences, path: &Path) -> Self {
        let key_saved = prefs
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty());
        let credential_source = CredentialSource::of(key_saved);
        Self {
            ok: true,
            known: (!prefs.requires_language_choice())
                .then(|| prefs.startup_language().to_uppercase()),
            key_saved,
            credential_present: !matches!(credential_source, CredentialSource::None),
            credential_source,
            preferences_path: path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CredentialSource {
    Environment,
    Saved,
    None,
}

impl CredentialSource {
    fn of(key_saved: bool) -> Self {
        if std::env::var("GEMINI_API_KEY")
            .ok()
            .is_some_and(|key| !key.trim().is_empty())
        {
            return Self::Environment;
        }
        if key_saved {
            return Self::Saved;
        }
        Self::None
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
            ("RU", false),
            "saving --known must persist the language uppercased as a confirmed choice"
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
            serde_json::to_string(&ConfigDoc::of(&prefs, Path::new("/preferences.json")))
                .expect("serialize must succeed");
        assert!(
            !document.contains("super-secret"),
            "the config document must report key_saved, never the key value"
        );
    }

    #[test]
    fn config_arguments_debug_redacts_the_literal_key() {
        let args = ConfigArgs {
            known: Some(String::from("en")),
            key: Some(String::from("debug-secret-argument")),
        };
        let rendered = format!("{args:?}");
        assert_eq!(
            (
                rendered.contains("debug-secret-argument"),
                rendered.contains("[REDACTED]")
            ),
            (false, true),
            "ConfigArgs Debug exposed the literal API key"
        );
    }
}
