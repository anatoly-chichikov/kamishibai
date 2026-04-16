//! End-to-end CLI entrypoint for deck and report generation.

use std::ffi::OsString;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use time::OffsetDateTime;
use time::format_description::parse;

use crate::application::media::Pipeline;
use crate::domain::profile::naming;
use crate::infrastructure::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::infrastructure::gemini::GeminiClient;
use crate::infrastructure::input::{Vocabulary, VocabularyMapping};
use crate::infrastructure::media::Media;
use crate::infrastructure::paths::{LocationArgs, Locations, SystemContext};
use crate::infrastructure::report::{Report, Thumbnail, VocabularyLayout};
use crate::presentation::diagnosis::{DiagnosisSelector, Display};
use crate::presentation::progress::{AppProgress, ProgressSelector};
use crate::profile::{Fonts, Labels};

/// Parsed CLI arguments for the application contract.
#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "kamishibai",
    about = "Convert schema-driven vocabulary JSON to an illustrated Anki deck",
    long_about = None,
    after_help = "Examples:\n  kamishibai\n  kamishibai my-words.json\n  kamishibai --deck \"Core Pack\" my-words.json\n  kamishibai --output ./output --cache ~/.cache/kamishibai my-words.json"
)]
pub struct Arguments {
    /// Optional deck name override.
    #[arg(long)]
    pub deck: Option<String>,
    /// Directory for generated output files.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Directory for reusable media cache.
    #[arg(long)]
    pub cache: Option<PathBuf>,
    /// Optional path to the vocabulary JSON file.
    pub path: Option<PathBuf>,
}

/// One handled CLI failure with an optional path context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliError {
    Failure {
        message: String,
        path: Option<PathBuf>,
    },
    Interrupted,
}

impl CliError {
    /// Create one handled CLI failure.
    pub fn handled(message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self::Failure {
            message: message.into(),
            path,
        }
    }

    /// Return the human-facing failure message.
    pub fn message(&self) -> &str {
        match self {
            Self::Failure { message, .. } => message.as_str(),
            Self::Interrupted => "Interrupted",
        }
    }

    /// Return the optional filesystem path context.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Failure { path, .. } => path.as_deref(),
            Self::Interrupted => None,
        }
    }
}

/// Parse CLI arguments into the public argument shape.
pub fn arguments<I, T>(args: I) -> Result<Arguments>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Ok(Arguments::try_parse_from(
        std::iter::once(OsString::from("kamishibai")).chain(args.into_iter().map(Into::into)),
    )?)
}

/// Translate one application body and diagnosis printer into an exit code.
pub fn handle<F, D>(main: F, mut diagnosis: D) -> u8
where
    F: FnOnce() -> Result<(), CliError>,
    D: Display,
{
    match main() {
        Ok(()) => 0,
        Err(CliError::Interrupted) => 130,
        Err(error) => {
            diagnosis.show(error.message(), error.path());
            1
        }
    }
}

/// Run the application logic for one CLI invocation.
pub fn main<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = arguments(args).map_err(|error| CliError::handled(error.to_string(), None))?;
    let resolved = Locations::new(location_args(&args), SystemContext);
    let input = resolved
        .input()
        .map_err(|error| CliError::handled(error.to_string(), None))?;
    let vocabulary = Vocabulary::new(input.clone(), VocabularyMapping);
    let document = vocabulary
        .document()
        .map_err(|error| CliError::handled(error.to_string(), Some(input.clone())))?;
    let entries = vocabulary
        .entries(Some(&document))
        .map_err(|error| CliError::handled(error.to_string(), Some(input.clone())))?;
    let client =
        GeminiClient::from_env().map_err(|error| CliError::handled(error.to_string(), None))?;
    let decknaming = naming(args.deck.as_deref(), entries.as_slice());
    let media = Media::new(
        client,
        resolved
            .cache()
            .map_err(|error| CliError::handled(error.to_string(), None))?,
    );
    let model = CardModel::new().model();
    let container = VocabularyDeck::new(
        StableId::new(decknaming.name()).value(),
        decknaming.name(),
        VocabularyNote::new(model),
        Vec::<PathBuf>::new(),
    );
    let progress = ProgressSelector::new(if crate::progress::uses_stdout() {
        std::io::stdout().is_terminal()
    } else {
        std::io::stderr().is_terminal()
    })
    .selected();
    let mut pipeline = Pipeline::new(media.clone(), media, container, progress);
    let (failed, processed) = pipeline.process(entries.as_slice());
    let output = resolved
        .output()
        .map_err(|error| CliError::handled(error.to_string(), None))?;
    fs::create_dir_all(&output)
        .map_err(|error| CliError::handled(error.to_string(), Some(output.clone())))?;
    let stamp = stamp().map_err(|error| CliError::handled(error.to_string(), None))?;
    let apkg = output.join(format!("{}_{}.apkg", decknaming.prefix(), stamp));
    pipeline
        .deck()
        .save(&apkg)
        .map_err(|error| CliError::handled(error.to_string(), Some(apkg.clone())))?;
    let mut report = Report::new(VocabularyLayout::new(Labels::default()), Fonts::default());
    for (entry, imagepath) in processed.clone() {
        report.append(&entry, Some(imagepath));
    }
    let pdf = output.join(format!("{}_{}.pdf", decknaming.prefix(), stamp));
    report
        .save(&pdf, &Thumbnail::new(150))
        .map_err(|error| CliError::handled(error.to_string(), Some(pdf.clone())))?;
    pipeline.progress_mut().result("Anki deck", &apkg);
    pipeline.progress_mut().result("Report", &pdf);
    pipeline.progress_mut().result("Output", &output);
    pipeline.progress_mut().finish(
        entries.len() - failed.len(),
        entries.len(),
        failed.as_slice(),
    );
    Ok(())
}

/// Execute the CLI and translate handled failures into exit codes.
pub fn run<I, T>(args: I) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    handle(
        || main(args),
        DiagnosisSelector::new(std::io::stderr().is_terminal()).selected(),
    )
}

/// Return one location argument bundle from parsed CLI arguments.
fn location_args(args: &Arguments) -> LocationArgs {
    LocationArgs {
        path: args.path.clone(),
        output: args.output.clone(),
        cache: args.cache.clone(),
    }
}

/// Return one local timestamp string for output filenames.
fn stamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc()
        .format(parse("[year]-[month]-[day]_[hour][minute][second]")?.as_slice())?)
}
