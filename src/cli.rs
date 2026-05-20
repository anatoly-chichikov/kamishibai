//! TUI entrypoint for the word-first kamishibai flow.
//!
//! The CLI module owns process arguments and startup decisions. The interactive
//! shell, live card generator, terminal loop, and startup card loader
//! live in focused submodules so the entrypoint stays small.

mod batch;
mod card_workflow;
mod host;
mod live_generator;
mod shell;
mod terminal;

use std::path::PathBuf;

use anyhow::Result;

use batch::StartupCards;
use terminal::run_tui;

use crate::config::{Preferences, default_store};
use crate::runtime::locations::SystemContext;
use crate::session::LanguagePair;
use crate::tui::{App, KeySource, WelcomeStage};

/// Execute the TUI and translate failures into a process exit code.
///
/// Without arguments the TUI starts on the empty `Your Words` screen and runs
/// the full intake to deck generation flow. With one positional argument, a
/// strict-schema vocabulary JSON document is loaded and generation starts from
/// the `Your Cards` screen.
pub fn run() -> u8 {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if args.next().is_some() {
        eprintln!("usage: kamishibai [path-to-vocabulary.json]");
        return 2;
    }
    let outcome = match first {
        None => start(),
        Some(path) => start_with_batch(PathBuf::from(path)),
    };
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("kamishibai: {error}");
            1
        }
    }
}

fn start() -> Result<()> {
    let store = default_store(&SystemContext)?;
    let preferences = store.read().unwrap_or_default();
    let env_key = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    let app = startup_app(&preferences, env_key);
    run_tui(app, None)
}

fn startup_app(preferences: &Preferences, env_key: Option<String>) -> App {
    let saved_key = preferences.api_key.clone().filter(|key| !key.is_empty());
    let pair = LanguagePair::new(
        String::from("en"),
        preferences.startup_language().to_string(),
    );
    let app = App::new(pair);
    let needs_language = preferences.requires_language_choice();
    let needs_key = env_key.is_none() && saved_key.is_none();
    if needs_language || needs_key {
        let (source, key) = if let Some(env) = env_key.as_deref() {
            (KeySource::Env, String::from(env))
        } else if let Some(saved) = saved_key.as_deref() {
            (KeySource::Restored, String::from(saved))
        } else {
            (KeySource::Empty, String::new())
        };
        let stage = if needs_language {
            WelcomeStage::PickLanguage
        } else {
            WelcomeStage::EnterKey
        };
        app.opening_welcome_at(stage, source, key)
    } else {
        app
    }
}

fn start_with_batch(path: PathBuf) -> Result<()> {
    let (app, drafts) = StartupCards::load(path.as_path())?.into_parts();
    run_tui(app, Some(drafts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::Screen;

    #[test]
    fn env_key_does_not_skip_unconfirmed_language_choice() {
        let app = startup_app(
            &Preferences::default(),
            Some(String::from("123456789012345678901234567890")),
        );
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().support().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::PickLanguage,
                KeySource::Env,
                String::from("en"),
            ),
            "GEMINI_API_KEY must not skip the explicit first-run language choice"
        );
    }

    #[test]
    fn saved_key_does_not_skip_unconfirmed_language_choice() {
        let preferences = Preferences::default().with_api_key("123456789012345678901234567890");
        let app = startup_app(&preferences, None);
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().support().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::PickLanguage,
                KeySource::Restored,
                String::from("en"),
            ),
            "a saved API key without a confirmed language must still start on language choice"
        );
    }

    #[test]
    fn confirmed_language_and_env_key_skip_welcome() {
        let app = startup_app(
            &Preferences::new("de"),
            Some(String::from("123456789012345678901234567890")),
        );
        assert_eq!(
            (app.screen(), app.pair().support().to_string()),
            (Screen::YourWords, String::from("de")),
            "only an explicitly confirmed language may skip Welcome when an env key exists"
        );
    }

    #[test]
    fn confirmed_language_without_key_starts_on_key_stage() {
        let app = startup_app(&Preferences::new("ru"), None);
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().support().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::EnterKey,
                KeySource::Empty,
                String::from("ru"),
            ),
            "a confirmed language with no key must ask only for the missing key"
        );
    }
}
