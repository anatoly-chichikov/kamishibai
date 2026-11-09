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
mod overview;
mod screen;
mod terminal;

/// Exact source, deck and output choices for Anki history preparation.
#[derive(Clone, Debug, Args)]
pub(super) struct ExportArgs {
    /// Anki profile to read.
    #[arg(long, value_name = "NAME", conflicts_with = "collection")]
    pub(super) profile: Option<String>,
    /// Path to a collection.anki2 file.
    #[arg(long, value_name = "PATH")]
    pub(super) collection: Option<PathBuf>,
    /// Full deck name (repeatable).
    #[arg(long = "deck", value_name = "NAME")]
    pub(super) decks: Vec<String>,
    /// Folder for the file.
    #[arg(long, value_name = "DIR")]
    pub(super) out: Option<PathBuf>,
    /// Save without the guided screen.
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
        match result {
            Ok(terminal::Exit::InputEnded(error)) => Err(error.into()),
            Ok(terminal::Exit::Cancelled) => {
                println!("export cancelled, nothing saved");
                Ok(())
            }
            _ if cancelled.verify().is_err() => Ok(()),
            result => result.map(|_| ()),
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
        .ok_or_else(|| {
            error::usage_hint(
                "no Downloads folder found",
                "choose a folder with --out DIR",
            )
        })?;
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
    let next = "in Kamishibai web: choose your language pair → open your account → import from kamishibai cli";
    match render {
        Render::Json => println!(
            "{}",
            serde_json::to_string(
                &json!({ "ok": true, "profile": profile, "anki": "closed", "preview": preview, "save_to": directory, "save_command": command, "file": file, "next": if args.save { next } else { "show this preview to the user; save only after consent" } })
            )?
        ),
        Render::Text => {
            let mut output = io::stdout().lock();
            let overview = overview::Overview::from_preview(&preview);
            writeln!(
                output,
                "export from Anki · {profile} · {}",
                if file.is_some() { "saved" } else { "not saved" }
            )?;
            match file {
                Some(file) => {
                    overview.write(&mut output, "file", &file.to_string_lossy())?;
                    writeln!(output, "next, in Kamishibai web")?;
                    writeln!(output, "  1. choose your language pair")?;
                    writeln!(output, "  2. open your account")?;
                    writeln!(output, "  3. import from kamishibai cli")?;
                }
                None => {
                    overview.write(&mut output, "folder", &directory.to_string_lossy())?;
                    writeln!(output, "to save, run: {}", invocation(args))?;
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
    let roots = history::roots();
    let mut paths = history::discover(&roots).map_err(refusal)?;
    if paths.is_empty() {
        return Err(error::usage_hint(
            format!(
                "no Anki profiles found in {}",
                roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "choose your collection.anki2 with --collection PATH",
        ));
    }
    if let Some(name) = &args.profile {
        let available = profile_names(&paths);
        paths.retain(|path| {
            path.parent()
                .and_then(Path::file_name)
                .is_some_and(|folder| folder == std::ffi::OsStr::new(name))
        });
        if paths.is_empty() {
            return Err(error::usage_hint(
                format!("Anki profile not found: {name}"),
                format!("choose one with --profile NAME: {available}"),
            ));
        }
    }
    Ok(paths)
}

fn profile_names(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| {
            path.parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn collection(args: &ExportArgs) -> Result<PathBuf> {
    let profiles = profiles(args)?;
    if let [path] = profiles.as_slice() {
        return Ok(std::env::current_dir()?.join(path));
    }
    Err(error::usage_hint(
        "several Anki profiles found",
        format!(
            "choose one with --profile NAME: {}",
            profile_names(&profiles)
        ),
    ))
}

fn refusal(failure: HistoryError) -> anyhow::Error {
    match failure {
        HistoryError::Open => error::not_ready_hint(failure.to_string(), "close Anki first"),
        HistoryError::Changed => {
            error::not_ready_hint(failure.to_string(), "keep Anki closed while exporting")
        }
        HistoryError::Selection(_) => {
            error::usage_hint(failure.to_string(), "use the full deck name with --deck")
        }
        HistoryError::Collection(_) => error::usage(failure.to_string()),
        HistoryError::Destination => {
            error::usage_hint(failure.to_string(), "choose another folder with --out DIR")
        }
        HistoryError::Schema(_) => {
            error::operational_hint(failure.to_string(), "update kamishibai")
        }
        HistoryError::Oversized => {
            error::operational_hint(failure.to_string(), "choose fewer decks with --deck NAME")
        }
        HistoryError::Io(error) if error.kind() == io::ErrorKind::Interrupted => {
            anyhow::anyhow!("export cancelled, nothing saved")
        }
        _ => error::operational_hint(failure.to_string(), "check the Anki and output folders"),
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

fn invocation(args: &ExportArgs) -> String {
    let mut command = String::from("kamishibai export");
    if let Some(profile) = &args.profile {
        command.push_str(&format!(" --profile {}", argument(profile)));
    }
    if let Some(collection) = &args.collection {
        command.push_str(&format!(
            " --collection {}",
            argument(&collection.to_string_lossy())
        ));
    }
    for deck in &args.decks {
        command.push_str(&format!(" --deck {}", argument(deck)));
    }
    if let Some(out) = &args.out {
        command.push_str(&format!(" --out {}", argument(&out.to_string_lossy())));
    }
    command.push_str(" --save");
    command
}

fn argument(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./:@%+=,-".contains(character))
    {
        value.to_owned()
    } else {
        quote(value)
    }
}
