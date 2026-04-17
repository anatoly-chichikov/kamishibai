//! End-to-end CLI entrypoint for deck and report generation.

use std::ffi::OsString;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use time::OffsetDateTime;
use time::format_description::parse;

use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::gemini::GeminiClient;
use crate::generation::{DeckBuilder, GeneratorCatalog};
use crate::languages::{ReportLabels, naming};
use crate::report::{Report, ReportFonts, Thumbnail, VocabularyLayout};
use crate::runtime::diagnosis::{DiagnosisSelector, Display};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::runtime::progress::{AppProgress, ProgressSelector};
use crate::vocabulary::VocabularyDocument;

/// Parsed CLI arguments for the application contract.
#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "kamishibai",
    about = "Convert schema-driven vocabulary JSON to an illustrated Anki deck",
    long_about = None,
    after_help = "Examples:\n  kamishibai\n  kamishibai my-words.json\n  kamishibai --deck \"Core Pack\" my-words.json\n  kamishibai --out ./kamishibai-out --cache ~/.cache/kamishibai my-words.json"
)]
pub struct Arguments {
    /// Optional deck name override.
    #[arg(long)]
    pub deck: Option<String>,
    /// Directory for generated output files.
    #[arg(long = "out")]
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
        Err(CliError::Failure { message, path }) => {
            diagnosis.show(message.as_str(), path.as_deref());
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
    let document = VocabularyDocument::load(&input)
        .map_err(|error| CliError::handled(error.to_string(), Some(input.clone())))?;
    let client =
        GeminiClient::from_env().map_err(|error| CliError::handled(error.to_string(), None))?;
    let decknaming = naming(args.deck.as_deref(), document.entries.as_slice());
    let generators = GeneratorCatalog::new(
        client,
        resolved
            .cache()
            .map_err(|error| CliError::handled(error.to_string(), None))?,
    );
    let model = CardModel::new().model();
    let container = VocabularyDeck::new(
        StableId::new(decknaming.name.as_str()).value(),
        decknaming.name.as_str(),
        VocabularyNote::new(model),
        Vec::<PathBuf>::new(),
    );
    let progress = ProgressSelector::new(if crate::runtime::progress::uses_stdout() {
        std::io::stdout().is_terminal()
    } else {
        std::io::stderr().is_terminal()
    })
    .selected();
    let mut builder = DeckBuilder::new(generators, container, progress);
    let (failed, processed) = builder.process(document.entries.as_slice());
    let output = resolved
        .output()
        .map_err(|error| CliError::handled(error.to_string(), None))?;
    fs::create_dir_all(&output)
        .map_err(|error| CliError::handled(error.to_string(), Some(output.clone())))?;
    let stamp = stamp().map_err(|error| CliError::handled(error.to_string(), None))?;
    let apkg = output.join(format!("{}_{}.apkg", decknaming.prefix, stamp));
    builder
        .deck()
        .save(&apkg)
        .map_err(|error| CliError::handled(error.to_string(), Some(apkg.clone())))?;
    let mut report = Report::new(
        VocabularyLayout::new(ReportLabels::default()),
        ReportFonts::default(),
    );
    for (entry, imagepath) in processed.clone() {
        report.append(&entry, Some(imagepath));
    }
    let pdf = output.join(format!("{}_{}.pdf", decknaming.prefix, stamp));
    report
        .save(&pdf, &Thumbnail::new(150))
        .map_err(|error| CliError::handled(error.to_string(), Some(pdf.clone())))?;
    builder.progress_mut().result("Anki deck", &apkg);
    builder.progress_mut().result("Report", &pdf);
    builder.progress_mut().result("Output", &output);
    builder.progress_mut().finish(
        document.entries.len() - failed.len(),
        document.entries.len(),
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
