//! Command-line entrypoint: the interactive TUI and the session-based console.
//!
//! With no arguments kamishibai opens the TUI; a bare JSON path opens the TUI on
//! a prebuilt batch. Everything non-interactive is a session subcommand
//! (`new`/`generate`/`status`/…) owned by the `session` module; this file only
//! parses arguments and routes them.

mod batch;
mod bridge;
mod card_workflow;
mod console;
mod error;
mod host;
mod live_generator;
mod session;
mod shell;
mod terminal;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

const SCHEMA_HELP: &str = "\
AGENT CONTRACT:
  Driving this from a script or agent? The full machine-readable contract —
  every command, flag, JSON shape, and exit code — is llms.txt at the repo root.
  Fetch and read it before integrating:
  https://raw.githubusercontent.com/anatoly-chichikov/kamishibai/main/llms.txt

EXAMPLES:
  kamishibai                                       open the interactive TUI
  kamishibai new --word bank --learning en         understand words, create a session
  kamishibai select --card bank --sense 2          keep only the 2nd sense of a card
  kamishibai exclude --card spring                 drop one card from the plan
  kamishibai generate                              generate + publish in the background
  kamishibai status                                progress (no Gemini)
  kamishibai result                                the finished cards + deck/pdf paths
  kamishibai result --json                         the paths/cards as JSON (for scripts)
  kamishibai regenerate --failed                   retry the cards that did not finish
  kamishibai regenerate --card bank --note \"…\"     re-roll one card from an instruction
  kamishibai new --build cards.json --generate     import a cards JSON and start at once
  kamishibai cards.json                            open the TUI on a prebuilt batch
  kamishibai cache-path                            print the cache directory

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
native-speaker audio, and manga-style illustrations.";

/// Turn a list of words into an illustrated Anki deck — sentences,
/// native-speaker audio, and manga-style art.
#[derive(Debug, Parser)]
#[command(
    name = "kamishibai",
    version,
    about = "Turn a list of words into an illustrated Anki deck — sentences, native-speaker audio, manga-style art.",
    after_help = "Agent or script? Read the machine contract: https://raw.githubusercontent.com/anatoly-chichikov/kamishibai/main/llms.txt",
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
    let cli = Cli::parse();
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
    use clap::CommandFactory;
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
        let hidden = !Cli::command()
            .render_long_help()
            .to_string()
            .contains("__run");
        assert!(
            parsed && hidden,
            "__run must parse but never appear in the help"
        );
    }

    #[test]
    fn long_help_documents_the_cards_json_schema() {
        assert!(
            Cli::command()
                .render_long_help()
                .to_string()
                .contains("WORDS_JSON format"),
            "long help must keep documenting the strict cards JSON schema"
        );
    }

    #[test]
    fn version_reports_the_release_version() {
        assert_eq!(
            Cli::command().get_version(),
            Some(env!("CARGO_PKG_VERSION")),
            "the CLI must report the current release version"
        );
    }
}
