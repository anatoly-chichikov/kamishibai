//! The clap grammar for the session subcommands: the `Command` enum and every
//! argument struct. Behaviour lives in `mod.rs` and `curate.rs`; this file is
//! only the parse surface, so its fields are `pub(super)` for the handlers to read.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::cli::console::SensePolicy;

/// One non-interactive session operation.
#[derive(Debug, Subcommand)]
pub(in crate::cli) enum Command {
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
    /// Drop a committed card's cached artifacts so the next generate rebuilds them.
    Regenerate(RegenerateArgs),
    /// Re-roll one committed card from a note (a later curation discards the fix).
    Fix(FixArgs),
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
    /// Internal: run the detached generation worker for a session.
    #[command(name = "__run", hide = true)]
    Worker(IdArg),
}

/// Arguments for `new`.
#[derive(Debug, Args)]
pub(in crate::cli) struct NewArgs {
    /// One word to learn (repeat the flag for more, one card per word).
    #[arg(long = "word", value_name = "WORD")]
    pub(super) word: Vec<String>,
    /// Read words from this file (one per line), or `-` for stdin.
    #[arg(long, value_name = "FILE")]
    pub(super) words: Option<String>,
    /// Import a strict cards JSON path (or `-` for stdin) and skip understanding.
    #[arg(long, value_name = "FILE")]
    pub(super) build: Option<PathBuf>,
    /// Native language you explain from (defaults to your saved preference).
    #[arg(long, value_name = "LANG")]
    pub(super) from: Option<String>,
    /// Target language you are learning (defaults to autodetection).
    #[arg(long, value_name = "LANG")]
    pub(super) to: Option<String>,
    /// How many senses of each word are selected initially.
    #[arg(long, value_name = "WHICH", default_value = "primary")]
    pub(super) senses: SensePolicy,
    /// Output directory for the deck and report (defaults to ./kamishibai-out).
    #[arg(short, long, value_name = "DIR")]
    pub(super) out: Option<PathBuf>,
    /// Use this session id instead of a minted one.
    #[arg(long, value_name = "NAME")]
    pub(super) id: Option<String>,
    /// Start the background worker immediately after creating the session.
    #[arg(long)]
    pub(super) generate: bool,
    /// Print only the session id (no understood-senses preview).
    #[arg(short, long)]
    pub(super) quiet: bool,
}

/// Arguments for `generate`.
#[derive(Debug, Args)]
pub(in crate::cli) struct GenerateArgs {
    /// The session id.
    pub(super) id: String,
    /// Run in the foreground, streaming progress, instead of detaching.
    #[arg(long)]
    pub(super) wait: bool,
    /// Suppress progress; print only the final paths.
    #[arg(short, long)]
    pub(super) quiet: bool,
}

/// Arguments for `select`.
#[derive(Debug, Args)]
pub(in crate::cli) struct SelectArgs {
    /// The session id.
    pub(super) id: String,
    /// The term of the card to select senses for.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
    /// The 1-based sense numbers to turn into cards (comma-separated).
    #[arg(long, value_name = "N", value_delimiter = ',', num_args = 1..)]
    pub(super) sense: Vec<usize>,
}

/// Arguments for `correct`.
#[derive(Debug, Args)]
pub(in crate::cli) struct CorrectArgs {
    /// The session id.
    pub(super) id: String,
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
    /// The session id.
    pub(super) id: String,
    /// Print only the phase word.
    #[arg(short, long)]
    pub(super) quiet: bool,
}

/// Arguments for `result`.
#[derive(Debug, Args)]
pub(in crate::cli) struct ResultArgs {
    /// The session id.
    pub(super) id: String,
    /// Print only the deck/pdf/dir paths (no card bodies).
    #[arg(short, long)]
    pub(super) quiet: bool,
    /// Print only the deck path.
    #[arg(long, conflicts_with_all = ["pdf", "dir"])]
    pub(super) deck: bool,
    /// Print only the report PDF path.
    #[arg(long, conflicts_with = "dir")]
    pub(super) pdf: bool,
    /// Print only the output directory path.
    #[arg(long)]
    pub(super) dir: bool,
}

/// A bare session id.
#[derive(Debug, Args)]
pub(in crate::cli) struct IdArg {
    /// The session id.
    pub(super) id: String,
}

/// A session id plus the term of one card.
#[derive(Debug, Args)]
pub(in crate::cli) struct CardArg {
    /// The session id.
    pub(super) id: String,
    /// The term of the card to act on.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
}

/// Arguments for `ls`.
#[derive(Debug, Args)]
pub(in crate::cli) struct LsArgs {
    /// Print only the session ids, one per line.
    #[arg(short, long)]
    pub(super) quiet: bool,
}

/// Arguments for `regenerate`.
#[derive(Debug, Args)]
pub(in crate::cli) struct RegenerateArgs {
    /// The session id.
    pub(super) id: String,
    /// Regenerate every card that has not finished.
    #[arg(long)]
    pub(super) failed: bool,
    /// Regenerate one card by its term.
    #[arg(long, value_name = "TERM")]
    pub(super) card: Option<String>,
}

/// Arguments for `fix`.
#[derive(Debug, Args)]
pub(in crate::cli) struct FixArgs {
    /// The session id.
    pub(super) id: String,
    /// The term of the card to re-roll.
    #[arg(long, value_name = "TERM")]
    pub(super) card: String,
    /// The instruction describing what to change.
    #[arg(long, value_name = "NOTE")]
    pub(super) note: String,
}

/// Arguments for `rm`.
#[derive(Debug, Args)]
pub(in crate::cli) struct RmArgs {
    /// The session id.
    pub(super) id: String,
    /// Also delete the session's cached card folders.
    #[arg(long)]
    pub(super) cache: bool,
}
