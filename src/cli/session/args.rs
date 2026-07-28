//! The clap grammar for the session subcommands: the `Command` enum and every
//! argument struct. Behaviour lives in `mod.rs` and `curate.rs`; this file is
//! only the parse surface, so its fields are `pub(super)` for the handlers to read.

use std::fmt;
use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand};

use crate::cli::console::SensePolicy;

/// One non-interactive session operation.
#[derive(Debug, Subcommand)]
pub(in crate::cli) enum Command {
    /// Print the version-matched console contract embedded in this binary.
    AgentContract,
    /// Understand `--word`s (or import a cards JSON) and create a session.
    New(NewArgs),
    /// Open an existing session in the interactive TUI.
    Open(IdArg),
    /// Choose which senses of a card become cards (1-based, comma-separated).
    Select(SelectArgs),
    /// Exclude one card from generation, keeping it in the understanding.
    Exclude(CardArg),
    /// Ask Gemini to add senses to one card from a note.
    Correct(CorrectArgs),
    /// Start the managed background worker that generates and publishes.
    Generate(GenerateArgs),
    /// Print a session's phase and per-card progress (no Gemini).
    Status(StatusArgs),
    /// Drop committed cards' cached artifacts, then regenerate and republish
    /// them (with --note, Gemini first rewrites the card).
    Regenerate(RegenerateArgs),
    /// Print a session's published cards and artifact paths.
    Result(ResultArgs),
    /// Stop a session's running worker.
    Cancel(IdArg),
    /// List all sessions.
    Ls(LsArgs),
    /// Delete a session, optionally its cached cards too.
    Rm(RmArgs),
    /// Print the cache directory and exit.
    CachePath,
    /// Save or show persisted preferences: known language and Gemini API key.
    Config(ConfigArgs),
    /// Internal: run the detached generation worker for a session.
    #[command(name = "__run", hide = true)]
    Worker(WorkerArgs),
}

/// Arguments for `new`: exactly one input form — repeated `--word`s, a `--words`
/// file, or a `--build` cards JSON (whose entries carry the language pair).
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("input").required(true).args(["word", "words", "build"])))]
pub(in crate::cli) struct NewArgs {
    /// One word to learn (repeat the flag for more, one card per word).
    #[arg(long = "word", value_name = "WORD")]
    pub(super) word: Vec<String>,
    /// Read words from this file (one per line), or `-` for stdin.
    #[arg(long, value_name = "FILE")]
    pub(super) words: Option<String>,
    /// Import a strict cards JSON path (or `-` for stdin) and skip
    /// understanding; conflicts with --known, --learning, and --senses.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["known", "learning", "senses"])]
    pub(super) build: Option<PathBuf>,
    /// Language you already know and explain from (defaults to your saved preference).
    #[arg(long, value_name = "LANG")]
    pub(super) known: Option<String>,
    /// Language you are learning (defaults to autodetection).
    #[arg(long, value_name = "LANG")]
    pub(super) learning: Option<String>,
    /// How many senses of each word are selected initially.
    #[arg(long, value_name = "WHICH", default_value = "primary")]
    pub(super) senses: SensePolicy,
    /// Output directory for the deck and report (defaults from
    /// KAMISHIBAI_OUTPUT, then Documents/Kamishibai).
    #[arg(short, long, value_name = "DIR")]
    pub(super) out: Option<PathBuf>,
    /// Use this session id instead of a minted one.
    #[arg(long, value_name = "NAME")]
    pub(super) id: Option<String>,
    /// Start the background worker immediately after creating the session.
    #[arg(long)]
    pub(super) generate: bool,
}

/// Arguments for `generate`.
#[derive(Debug, Args)]
pub(in crate::cli) struct GenerateArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// Run in the foreground, streaming progress, instead of detaching.
    #[arg(long)]
    pub(super) wait: bool,
}

/// Arguments for `config`: with no flags it shows the saved preferences; with
/// `--known`/`--key` it saves them. `--key -` reads the key from stdin; an
/// empty `--key ""` clears the saved key.
#[derive(Args)]
pub(in crate::cli) struct ConfigArgs {
    /// Save this as your known (native) language, validated against the catalog.
    #[arg(long, value_name = "LANG")]
    pub(super) known: Option<String>,
    /// Save this Gemini API key (`-` reads it from stdin, empty clears it).
    #[arg(long, value_name = "KEY")]
    pub(super) key: Option<String>,
}

impl fmt::Debug for ConfigArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigArgs")
            .field("known", &self.known)
            .field("key", &self.key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Arguments for `select`.
#[derive(Debug, Args)]
pub(in crate::cli) struct SelectArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// The term of the card to select senses for.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
    /// The 1-based sense numbers to turn into cards (comma-separated).
    #[arg(long, value_name = "N", value_delimiter = ',', num_args = 1.., required = true)]
    pub(super) sense: Vec<usize>,
}

/// Arguments for `correct`.
#[derive(Debug, Args)]
pub(in crate::cli) struct CorrectArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// The term of the card to add senses to.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
    /// The instruction describing the senses to add.
    #[arg(long, value_name = "NOTE")]
    pub(super) note: String,
}

/// Arguments for `status`.
#[derive(Debug, Args)]
pub(in crate::cli) struct StatusArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
}

/// Arguments for `result`.
#[derive(Debug, Args)]
pub(in crate::cli) struct ResultArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
}

/// A bare session id.
#[derive(Debug, Args)]
pub(in crate::cli) struct IdArg {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
}

/// Arguments for the hidden `__run` worker entrypoint: the spawning parent
/// always passes the id, so it never goes through session resolution.
#[derive(Debug, Args)]
pub(in crate::cli) struct WorkerArgs {
    /// The session id.
    pub(super) id: String,
}

/// A session id plus the term of one card.
#[derive(Debug, Args)]
pub(in crate::cli) struct CardArg {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// The term of the card to act on.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
}

/// Arguments for `ls` (no options).
#[derive(Debug, Args)]
pub(in crate::cli) struct LsArgs {}

/// Arguments for `regenerate`: every unfinished card with `--failed`, or one
/// card by `--card` — optionally rewritten by Gemini first with `--note`.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").required(true).args(["failed", "card"])))]
pub(in crate::cli) struct RegenerateArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// Regenerate every card that has not finished.
    #[arg(long)]
    pub(super) failed: bool,
    /// Regenerate one card by its term.
    #[arg(long, value_name = "TERM")]
    pub(super) card: Option<String>,
    /// With --card, ask Gemini to rewrite the card from this instruction first
    /// (requires --card; conflicts with --failed).
    #[arg(
        long,
        value_name = "NOTE",
        requires = "card",
        conflicts_with = "failed"
    )]
    pub(super) note: Option<String>,
    /// Run in the foreground, streaming progress, instead of detaching.
    #[arg(long)]
    pub(super) wait: bool,
}

/// Arguments for `rm`.
#[derive(Debug, Args)]
pub(in crate::cli) struct RmArgs {
    /// The session id (omit it to use the only — or only unfinished — session).
    pub(super) id: Option<String>,
    /// Also delete the session's cached card folders.
    #[arg(long)]
    pub(super) cache: bool,
}
