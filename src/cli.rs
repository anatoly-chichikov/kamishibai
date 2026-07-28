//! Command-line entrypoint: the interactive TUI and the session-based console.
//!
//! With no arguments kamishibai opens the TUI; a bare JSON path opens the TUI on
//! a prebuilt batch. Everything non-interactive is a session subcommand
//! (`new`/`generate`/`status`/…) owned by the `session` module; this file only
//! parses arguments and routes them.

mod batch;
mod bridge;
mod console;
mod contract;
mod error;
mod host;
mod jobs;
mod session;
mod shell;
mod terminal;
mod wiring;

use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser};

const SHORT_CONTRACT_HELP: &str =
    "Agent or script? Run `kamishibai agent-contract` for this binary's console contract.";
const SCHEMA_HELP: &str = concat!(
    "\
AGENT CONTRACT:
  Run `kamishibai agent-contract` first. It prints the complete contract
  embedded in this binary, including its matching application version.
  The same release-pinned document is available at:
  https://raw.githubusercontent.com/anatoly-chichikov/kamishibai/v",
    env!("CARGO_PKG_VERSION"),
    "/llms.txt

FIRST-TIME HEADLESS SETUP:
  known = my/source language; learning = target language.
  For a personal machine, save known plus a verified key:

  kamishibai config --known RU --json
  printf '%s' \"$GEMINI_API_KEY\" | kamishibai config --key - --json
  kamishibai config --json

  For CI/shared/ephemeral agents, do not save the key:

  GEMINI_API_KEY=\"$SECRET\" kamishibai new --word chat --known RU --learning FR --json

EXAMPLES:
  kamishibai                                       open the interactive TUI
  kamishibai agent-contract                        print this binary's agent contract
  kamishibai new --word bank --learning EN --json  understand words, create a session
  kamishibai select --card bank --sense 2 --json   keep only the 2nd sense of a card
  kamishibai exclude --card spring --json          drop one card from the plan
  kamishibai generate --json                       generate + publish in the background
  kamishibai status --json                         progress (no Gemini)
  kamishibai result --json                         the paths/cards as JSON (for scripts)
  kamishibai regenerate --failed --json            retry cards that did not finish
  kamishibai new --build cards.json --json         import a cards JSON without Gemini intake
  kamishibai cards.json                            open the TUI on a prebuilt batch
  kamishibai cache-path --json                     print the cache directory

  The session id is optional everywhere: an omitted id means the only session,
  or the only unfinished one; with several candidates the command lists the
  newest five instead and exits 5.

OUTPUT:
  Two modes: plain text (default, for humans) and --json (after any session verb,
  for machines — exactly one JSON document on stdout, success or error envelope).
  Agents should use --json; plain text prints nothing bare to capture. Language
  codes are uppercase in all output, ids, cache paths, and JSON. Exit codes are
  identical in both modes for any invocation valid in both.

EXIT CODES:
  0 ok · 2 usage · 3 no such session · 4 not ready yet · 5 ambiguous session · 1 other error

ENVIRONMENT:
  GEMINI_API_KEY   the Gemini API key; it wins over a key saved through the
                   Welcome screen, and need not be set when a saved key exists
  KAMISHIBAI_DATA  platform data home override; appends kamishibai/preferences.json
  KAMISHIBAI_CACHE exact cache-root override
  KAMISHIBAI_OUTPUT exact output-root override; relative values resolve from cwd

WORDS_JSON format (for `new --build`; all fields required, unknown fields rejected):
{
  \"entries\": [
    {
      \"term\": \"lantern\",
      \"meaning\": \"a portable lamp\",
      \"pronunciation\": \"LAN-tern\",
      \"transcription\": \"/lantern/\",
      \"importance\": 7,
      \"source\": {
        \"sentence\": \"I carried a lantern through the dark hallway.\",
        \"lang\": \"en\",
        \"highlight\": \"lantern\",
        \"hint\": \"portable light\",
        \"context\": \"a simple everyday sentence\"
      },
      \"target\": { \"sentence\": \"Ich trug eine Laterne durch den dunklen Flur.\", \"lang\": \"de\" }
    }
  ]
}
A `new --build` session uses these fields to generate an Anki .apkg, a printable PDF,
native-speaker audio, and manga-style illustrations."
);

/// Turn a list of words into an illustrated Anki deck — sentences,
/// native-speaker audio, and manga-style art.
#[derive(Debug, Parser)]
#[command(
    name = "kamishibai",
    version,
    about = "Turn a list of words into an illustrated Anki deck — sentences, native-speaker audio, manga-style art.",
    after_help = SHORT_CONTRACT_HELP,
    after_long_help = SCHEMA_HELP,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// A prebuilt cards JSON path opens the interactive TUI on those cards.
    #[arg(value_name = "WORDS")]
    input: Option<String>,
    /// Print one JSON document on stdout instead of plain text (place it after
    /// the session verb; interactive runs refuse it).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<session::Command>,
}

/// Build the clap command used by the binary and contract grammar checks.
#[must_use]
pub fn command() -> clap::Command {
    Cli::command()
}

fn parse() -> Result<Cli, clap::Error> {
    let mut matches = command().try_get_matches()?;
    Cli::from_arg_matches_mut(&mut matches).map_err(|failure| {
        let mut grammar = command();
        failure.format(&mut grammar)
    })
}

/// Parse arguments and execute the selected flow, returning a process exit code.
///
/// Every refusal carries its exit code (`error.rs`): 2 — the invocation is
/// wrong, 3 — no such session, 4 — not ready yet. Any other error is an
/// operational failure and exits 1; success exits 0. The codes are identical
/// in both render modes for any invocation valid in both (`--json` grammar
/// conflicts are themselves usage refusals): the `kamishibai:` stderr line
/// always appears, and `--json` additionally prints the machine-readable
/// envelope on stdout.
pub fn run() -> u8 {
    let cli = match parse() {
        Ok(cli) => cli,
        Err(failure) => {
            let json =
                std::env::args_os().any(|argument| argument == std::ffi::OsStr::new("--json"));
            if !failure.use_stderr() {
                failure
                    .print()
                    .expect("invariant: clap help and version output must print");
                return 0;
            }
            let message = failure.to_string();
            failure
                .print()
                .expect("invariant: clap usage error output must print");
            if json {
                println!("{}", error::json_line(&error::usage(message.trim())));
            }
            return u8::try_from(failure.exit_code())
                .expect("invariant: clap process exit codes fit in u8");
        }
    };
    match execute(&cli) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("kamishibai: {error:#}");
            if let Some(hint) = error::hint_of(&error) {
                eprintln!("{hint}");
            }
            if cli.json {
                println!("{}", error::json_line(&error));
            }
            error::exit_code(&error).unwrap_or(1)
        }
    }
}

fn execute(cli: &Cli) -> Result<()> {
    let render = if cli.json {
        session::Render::Json
    } else {
        session::Render::Text
    };
    match &cli.command {
        Some(session::Command::AgentContract) => {
            if cli.json {
                return Err(error::usage(
                    "agent-contract is a text document; --json does not apply",
                ));
            }
            contract::print()
        }
        Some(command) => session::handle(command, render, &bridge::TuiOpener),
        None => {
            if cli.json {
                return Err(error::usage(
                    "--json applies only to non-interactive session commands",
                ));
            }
            match &cli.input {
                Some(path) => terminal::start_with_batch(PathBuf::from(path)),
                None => terminal::start(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::Command;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("arguments must parse")
    }

    #[test]
    fn bare_invocation_opens_the_tui() {
        let cli = parse(&["kamishibai"]);
        assert!(
            cli.command.is_none() && cli.input.is_none(),
            "bare kamishibai must carry no command and no input"
        );
    }

    #[test]
    fn a_bare_json_path_routes_to_the_tui_batch() {
        let cli = parse(&["kamishibai", "cards.json"]);
        assert!(
            cli.command.is_none() && cli.input.as_deref() == Some("cards.json"),
            "a bare positional path must stay a positional for the TUI batch"
        );
    }

    #[test]
    fn new_parses_to_the_new_command() {
        assert!(
            matches!(
                parse(&["kamishibai", "new", "--word", "wreck", "--learning", "fr"]).command,
                Some(Command::New(_))
            ),
            "new must parse to the New command"
        );
    }

    #[test]
    fn agent_contract_parses_to_the_contract_command() {
        assert!(
            matches!(
                parse(&["kamishibai", "agent-contract"]).command,
                Some(Command::AgentContract)
            ),
            "agent-contract must parse to the embedded contract command"
        );
    }

    #[test]
    fn first_time_agent_commands_keep_the_documented_grammar() {
        let normalize = |document: &str| {
            document
                .split_whitespace()
                .filter(|word| *word != "\\")
                .collect::<Vec<_>>()
                .join(" ")
        };
        let documents = [
            ("llms.txt", normalize(include_str!("../llms.txt"))),
            ("README.md", normalize(include_str!("../README.md"))),
            (
                "docs/cards-json.md",
                normalize(include_str!("../docs/cards-json.md")),
            ),
            ("root --help", normalize(SCHEMA_HELP)),
        ];
        let checks = [
            (0, "kamishibai agent-contract"),
            (0, "kamishibai config --json"),
            (0, "kamishibai config --known RU --json"),
            (0, "kamishibai config --key - --json"),
            (
                0,
                "kamishibai new --word chat --known RU --learning FR --json",
            ),
            (0, "kamishibai generate --json"),
            (0, "kamishibai status --json"),
            (0, "kamishibai result --json"),
            (1, "kamishibai agent-contract"),
            (2, "kamishibai new --build cards.json --json"),
            (2, "kamishibai generate --wait --json"),
            (2, "kamishibai result --json"),
            (3, "kamishibai agent-contract"),
            (3, "kamishibai config --known RU --json"),
            (3, "kamishibai config --key - --json"),
            (
                3,
                "kamishibai new --word chat --known RU --learning FR --json",
            ),
            (3, "kamishibai generate --json"),
            (3, "kamishibai status --json"),
            (3, "kamishibai result --json"),
            (3, "kamishibai new --build cards.json --json"),
        ];
        let failures = checks
            .iter()
            .filter(|(document, command)| {
                !documents[*document].1.contains(*command)
                    || Cli::try_parse_from(command.split_whitespace()).is_err()
            })
            .map(|(document, command)| (documents[*document].0, *command))
            .collect::<Vec<_>>();
        assert!(
            failures.is_empty(),
            "documented first-time commands no longer parse: {failures:?}"
        );
    }

    #[test]
    fn open_parses_to_the_open_command() {
        assert!(
            matches!(
                parse(&["kamishibai", "open", "fr-1"]).command,
                Some(Command::Open(_))
            ),
            "open must parse to the Open command"
        );
    }

    #[test]
    fn generate_parses_to_the_generate_command() {
        assert!(
            matches!(
                parse(&["kamishibai", "generate", "fr-1"]).command,
                Some(Command::Generate(_))
            ),
            "generate must parse to the Generate command"
        );
    }

    #[test]
    fn select_parses_to_the_select_command() {
        assert!(
            matches!(
                parse(&[
                    "kamishibai",
                    "select",
                    "fr-1",
                    "--card",
                    "bank",
                    "--sense",
                    "1,2"
                ])
                .command,
                Some(Command::Select(_))
            ),
            "select must parse to the Select command"
        );
    }

    #[test]
    fn regenerate_with_a_note_parses_to_the_regenerate_command() {
        assert!(
            matches!(
                parse(&[
                    "kamishibai",
                    "regenerate",
                    "fr-1",
                    "--card",
                    "bank",
                    "--note",
                    "use the river sense"
                ])
                .command,
                Some(Command::Regenerate(_))
            ),
            "regenerate with a note must parse to the Regenerate command"
        );
    }

    #[test]
    fn a_session_verb_with_no_id_parses_with_an_absent_id() {
        assert!(
            matches!(
                parse(&["kamishibai", "status"]).command,
                Some(Command::Status(_))
            ),
            "status without an id must parse, not fail on a missing positional"
        );
    }

    #[test]
    fn select_without_an_id_parses_its_flags_only() {
        assert!(
            matches!(
                parse(&["kamishibai", "select", "--card", "bank", "--sense", "2"]).command,
                Some(Command::Select(_))
            ),
            "select without an id must parse with its flags intact"
        );
    }

    #[test]
    fn generate_with_only_wait_parses_with_an_absent_id() {
        assert!(
            matches!(
                parse(&["kamishibai", "generate", "--wait"]).command,
                Some(Command::Generate(_))
            ),
            "generate --wait without an id must parse, not fail on a missing positional"
        );
    }

    #[test]
    fn the_worker_subcommand_still_requires_an_id() {
        assert!(
            Cli::try_parse_from(["kamishibai", "__run"]).is_err(),
            "the hidden worker entrypoint must keep its id mandatory"
        );
    }

    #[test]
    fn status_with_no_id_routes_to_the_subcommand_not_the_tui_input() {
        let cli = parse(&["kamishibai", "status"]);
        assert!(
            cli.input.is_none() && matches!(cli.command, Some(Command::Status(_))),
            "a bare status must stay a subcommand, never fall back to the TUI batch positional"
        );
    }

    #[test]
    fn a_trailing_json_flag_parses_on_a_session_verb() {
        assert!(
            parse(&["kamishibai", "status", "fr-1", "--json"]).json,
            "--json placed after the verb must parse into JSON mode"
        );
    }

    #[test]
    fn a_leading_json_flag_before_the_verb_is_refused_at_parse() {
        assert!(
            Cli::try_parse_from(["kamishibai", "--json", "status", "fr-1"]).is_err(),
            "--json before the verb must stay a parse error, pinning the documented flag position"
        );
    }

    #[test]
    fn the_worker_subcommand_parses_yet_stays_hidden() {
        let parsed = matches!(
            parse(&["kamishibai", "__run", "fr-1"]).command,
            Some(Command::Worker(_))
        );
        let hidden = !command().render_long_help().to_string().contains("__run");
        assert!(
            parsed && hidden,
            "__run must parse but never appear in the help"
        );
    }

    #[test]
    fn long_help_documents_the_cards_json_schema() {
        assert!(
            command()
                .render_long_help()
                .to_string()
                .contains("WORDS_JSON format"),
            "long help must keep documenting the strict cards JSON schema"
        );
    }

    #[test]
    fn version_reports_the_release_version() {
        assert_eq!(
            command().get_version(),
            Some(env!("CARGO_PKG_VERSION")),
            "the CLI must report the current release version"
        );
    }
}
