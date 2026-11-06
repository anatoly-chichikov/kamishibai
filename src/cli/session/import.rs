//! Thin console delivery for the library's read-only Anki history workflow.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::json;
use time::OffsetDateTime;

use crate::anki::history::{self, HistoryError, SystemAnkiProcess};
use crate::cli::error;

use super::Render;
use super::args::{AnkiArgs, ImportCommand};

/// Route a parsed import source without constructing a generation session.
pub(super) fn import(command: &ImportCommand, render: Render) -> Result<()> {
    match command {
        ImportCommand::Anki(args) => anki(args, render),
    }
}

fn anki(args: &AnkiArgs, render: Render) -> Result<()> {
    let path = collection(args)?;
    let directory = args
        .out
        .clone()
        .or_else(dirs::download_dir)
        .ok_or_else(|| error::usage_hint("Downloads directory is unavailable", "pass --out DIR"))?;
    let directory = std::env::current_dir()?.join(directory);
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
                .save(
                    &directory,
                    OffsetDateTime::now_utc(),
                    env!("CARGO_PKG_VERSION"),
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
    let next = "Kamishibai web → your account → import from anki";
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

fn collection(args: &AnkiArgs) -> Result<PathBuf> {
    if let Some(path) = &args.collection {
        return Ok(std::env::current_dir()?.join(path));
    }
    let profiles = history::discover(&history::roots()).map_err(refusal)?;
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
    let candidates = profiles
        .iter()
        .filter(|path| {
            args.profile.as_ref().is_none_or(|name| {
                path.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|folder| folder == std::ffi::OsStr::new(name))
            })
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [path] => Ok(std::env::current_dir()?.join(path)),
        [] => Err(error::usage_hint(
            "Anki profile not found",
            "pass --collection PATH to a closed collection.anki2 file",
        )),
        _ => Err(error::usage_hint(
            "several Anki profiles found",
            format!("choose --profile NAME ({choices}) or --collection PATH"),
        )),
    }
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
        "kamishibai import anki --collection {} --out {}",
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
