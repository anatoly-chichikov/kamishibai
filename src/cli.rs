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

use std::ffi::{OsStr, OsString};
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
    run_with_args(std::env::args_os().skip(1))
}

fn run_with_args<I>(args: I) -> u8
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let first = args.next();
    if args.next().is_some() {
        eprintln!(
            "usage: kamishibai [WORDS_JSON]   # optional; without it kamishibai opens the TUI"
        );
        return 2;
    }
    let outcome = match first {
        None => start(),
        Some(path) if is_flag(path.as_os_str(), "--help") || is_flag(path.as_os_str(), "-h") => {
            println!("{}", help());
            return 0;
        }
        Some(path) if is_flag(path.as_os_str(), "--version") || is_flag(path.as_os_str(), "-V") => {
            println!("{}", version());
            return 0;
        }
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

fn is_flag(value: &OsStr, flag: &str) -> bool {
    value == OsStr::new(flag)
}

fn version() -> String {
    format!("kamishibai {}", env!("CARGO_PKG_VERSION"))
}

fn help() -> &'static str {
    concat!(
        "Turn a list of words into an illustrated Anki deck — sentences, native-speaker audio, manga-style art.\n\n",
        "Usage: kamishibai [WORDS_JSON]\n\n",
        "Arguments:\n",
        "  [WORDS_JSON]  Optional path to a pre-built words JSON. If omitted, kamishibai walks you through the TUI.\n\n",
        "Options:\n",
        "  -h, --help     Print help\n",
        "  -V, --version  Print version\n\n",
        "With WORDS_JSON:\n",
        "  Bring your own JSON with the required fields. kamishibai skips word entry,\n",
        "  then uses its prompts to generate an Anki .apkg, a printable PDF,\n",
        "  native-speaker audio, and manga-style illustrations.\n\n",
        "WORDS_JSON format:\n",
        "{\n",
        "  \"entries\": [\n",
        "    {\n",
        "      \"term\": \"lantern\",\n",
        "      \"meaning\": \"a portable lamp\",\n",
        "      \"pronunciation\": \"LAN-tern\",\n",
        "      \"transcription\": \"/lantern/\",\n",
        "      \"importance\": 7,\n",
        "      \"source\": {\n",
        "        \"sentence\": \"I carried a lantern through the dark hallway.\",\n",
        "        \"lang\": \"en\",\n",
        "        \"highlight\": \"lantern\",\n",
        "        \"hint\": \"portable light\",\n",
        "        \"context\": \"a simple everyday sentence\"\n",
        "      },\n",
        "      \"target\": {\n",
        "        \"sentence\": \"Ich trug eine Laterne durch den dunklen Flur.\",\n",
        "        \"lang\": \"de\"\n",
        "      }\n",
        "    }\n",
        "  ]\n",
        "}\n\n",
        "JSON rules:\n",
        "  - entries must contain at least one item\n",
        "  - all fields are required; unknown fields are rejected\n",
        "  - text fields and lang values must be non-empty strings\n",
        "  - importance must be an integer from 1 to 10"
    )
}

fn start() -> Result<()> {
    let store = default_store(&SystemContext)?;
    let preferences = store.read().unwrap_or_default();
    let app = startup_app(&preferences);
    run_tui(app, None)
}

fn startup_app(preferences: &Preferences) -> App {
    let saved_key = preferences.api_key.clone().filter(|key| !key.is_empty());
    let pair = LanguagePair::new(
        String::from("en"),
        preferences.startup_language().to_string(),
    );
    let app = App::new(pair);
    let needs_language = preferences.requires_language_choice();
    let needs_key = saved_key.is_none();
    if needs_language || needs_key {
        let (source, key) = if let Some(saved) = saved_key.as_deref() {
            (KeySource::Restored, String::from(saved))
        } else {
            (KeySource::Empty, String::new())
        };
        let stage = if needs_language {
            WelcomeStage::PickLanguage
        } else {
            WelcomeStage::EnterKey
        };
        app.opening_welcome_at(stage, source, key, env_has_gemini_key())
    } else {
        app
    }
}

/// Return whether `GEMINI_API_KEY` is present and non-empty. The key is never
/// loaded into the Welcome buffer implicitly — this only decides whether the
/// key step offers the `load from env` action.
fn env_has_gemini_key() -> bool {
    std::env::var("GEMINI_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
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
    fn env_key_is_not_loaded_at_startup() {
        let app = startup_app(&Preferences::default());
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
                KeySource::Empty,
                String::from("en"),
            ),
            "GEMINI_API_KEY must not be treated as loaded until the user asks for it"
        );
    }

    #[test]
    fn saved_key_does_not_skip_unconfirmed_language_choice() {
        let preferences = Preferences::default().with_api_key("123456789012345678901234567890");
        let app = startup_app(&preferences);
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
    fn confirmed_language_and_saved_key_skip_welcome() {
        let app =
            startup_app(&Preferences::new("de").with_api_key("123456789012345678901234567890"));
        assert_eq!(
            (app.screen(), app.pair().support().to_string()),
            (Screen::YourWords, String::from("de")),
            "a confirmed language may skip Welcome only when a saved key exists"
        );
    }

    #[test]
    fn confirmed_language_without_key_starts_on_key_stage() {
        let app = startup_app(&Preferences::new("ru"));
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

    #[test]
    fn version_output_reports_the_release_version() {
        assert_eq!(
            version(),
            String::from("kamishibai 1.1.0"),
            "version output must report the current release version"
        );
    }

    #[test]
    fn help_output_documents_the_json_bypass_format() {
        assert!(
            help().contains("WORDS_JSON format:"),
            "help output must not hide the strict JSON bypass format"
        );
    }

    #[test]
    fn help_output_explains_what_json_bypass_generates() {
        assert!(
            help().contains("generate an Anki .apkg, a printable PDF"),
            "help output must not hide the artifacts generated from JSON input"
        );
    }
}
