//! Standalone guided and console delivery for read-only Anki history export.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::Args;

use anyhow::Result;
use serde_json::json;
use time::OffsetDateTime;

use crate::anki::history::{self, HistoryError, SaveGate, SystemAnkiProcess};
use crate::cli::error;

use super::session::Render;

mod input;
mod interrupt;
mod screen;
mod terminal;

/// Exact source, deck and output choices for Anki history preparation.
#[derive(Clone, Debug, Args)]
pub(super) struct ExportArgs {
    /// Exact profile folder name; otherwise the screen offers a choice.
    #[arg(long, value_name = "NAME", conflicts_with = "collection")]
    pub(super) profile: Option<String>,
    /// Explicit collection.anki2 path instead of profile discovery.
    #[arg(long, value_name = "PATH")]
    pub(super) collection: Option<PathBuf>,
    /// Exact full deck name, repeatable; defaults to Kamishibai model decks.
    #[arg(long = "deck", value_name = "NAME")]
    pub(super) decks: Vec<String>,
    /// Output directory; defaults to Downloads and is untouched during preview.
    #[arg(long, value_name = "DIR")]
    pub(super) out: Option<PathBuf>,
    /// Save without opening the guided screen; suitable for scripts.
    #[arg(long)]
    pub(super) save: bool,
}

/// Run the separate export screen in a terminal, or the explicit console workflow.
pub(super) fn run(args: &ExportArgs, render: Render) -> Result<()> {
    let cancelled = interrupt::Cancellation::new();
    let _signals = interrupt::Signals::from_cancellation(&cancelled)?;
    if render == Render::Text
        && !args.save
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
    {
        let result = terminal::run(args, directory(args)?, cancelled.clone());
        if cancelled.verify().is_err() {
            Ok(())
        } else {
            result
        }
    } else {
        anki(args, render, &cancelled)
    }
}

fn directory(args: &ExportArgs) -> Result<PathBuf> {
    let directory = args
        .out
        .clone()
        .or_else(dirs::download_dir)
        .ok_or_else(|| error::usage_hint("Downloads directory is unavailable", "pass --out DIR"))?;
    Ok(std::env::current_dir()?.join(directory))
}

fn anki(args: &ExportArgs, render: Render, cancelled: &interrupt::Cancellation) -> Result<()> {
    let path = collection(args)?;
    let directory = directory(args)?;
    let snapshot = history::read(
        &path,
        &args.decks,
        &SystemAnkiProcess,
        &history::application_version(),
    )
    .map_err(refusal)?;
    let preview = snapshot.preview().map_err(refusal)?;
    let file = if args.save {
        Some(
            snapshot
                .save_guarded(
                    &directory,
                    OffsetDateTime::now_utc(),
                    env!("CARGO_PKG_VERSION"),
                    cancelled,
                )
                .map_err(refusal)?,
        )
    } else {
        None
    };
    let profile = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let command = save_command(&path, &directory, &args.decks);
    let next = "in the web: import from kamishibai cli";
    match render {
        Render::Json => println!(
            "{}",
            serde_json::to_string(
                &json!({ "ok": true, "profile": profile, "anki": "closed", "preview": preview, "save_to": directory, "save_command": command, "file": file, "next": if args.save { next } else { "show this preview to the user; save only after consent" } })
            )?
        ),
        Render::Text => {
            let mut output = io::stdout().lock();
            writeln!(output, "your Anki history · {profile} · Anki closed")?;
            preview.write(&mut output)?;
            match file {
                Some(file) => {
                    writeln!(output, "file: {}", file.display())?;
                    writeln!(output, "next: {next}")?;
                }
                None => {
                    writeln!(output, "save to: {}", directory.display())?;
                    writeln!(output, "after reviewing and agreeing, run: {command}")?;
                }
            }
        }
    }
    Ok(())
}

fn profiles(args: &ExportArgs) -> Result<Vec<PathBuf>> {
    if let Some(path) = &args.collection {
        return Ok(vec![std::env::current_dir()?.join(path)]);
    }
    let profiles = history::discover(&history::roots())
        .map_err(refusal)?
        .into_iter()
        .filter(|path| {
            args.profile.as_ref().is_none_or(|name| {
                path.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|folder| folder == std::ffi::OsStr::new(name))
            })
        })
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        return Err(error::usage_hint(
            "Anki profile not found",
            "pass --collection PATH to a closed collection.anki2 file",
        ));
    }
    Ok(profiles)
}

fn collection(args: &ExportArgs) -> Result<PathBuf> {
    let profiles = profiles(args)?;
    if let [path] = profiles.as_slice() {
        return Ok(std::env::current_dir()?.join(path));
    }
    let choices = profiles
        .iter()
        .map(|path| {
            path.parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(error::usage_hint(
        "several Anki profiles found",
        format!("choose --profile NAME ({choices}) or --collection PATH"),
    ))
}

fn refusal(failure: HistoryError) -> anyhow::Error {
    match failure {
        HistoryError::Open | HistoryError::Changed => error::not_ready_hint(
            failure.to_string(),
            "sync, quit Anki, run again; if WAL remains, open Anki and quit it normally",
        ),
        HistoryError::Selection(_) => error::usage_hint(
            failure.to_string(),
            "choose --profile NAME, --collection PATH or an exact --deck NAME",
        ),
        HistoryError::Destination => error::usage_hint(
            failure.to_string(),
            "pass --out DIR outside the Anki profile and Anki2 folder",
        ),
        HistoryError::Schema(_) => error::operational_hint(
            failure.to_string(),
            "update kamishibai before reading this Anki schema",
        ),
        HistoryError::Oversized => {
            error::operational_hint(failure.to_string(), "select fewer decks with --deck NAME")
        }
        _ => error::operational_hint(
            failure.to_string(),
            "check the collection and output directory, then run again",
        ),
    }
}

fn save_command(path: &Path, directory: &Path, decks: &[String]) -> String {
    let mut command = format!(
        "kamishibai export --collection {} --out {}",
        quote(&path.to_string_lossy()),
        quote(&directory.to_string_lossy())
    );
    for deck in decks {
        command.push_str(&format!(" --deck {}", quote(deck)));
    }
    command.push_str(" --save");
    command
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
