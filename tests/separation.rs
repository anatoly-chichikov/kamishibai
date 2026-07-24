//! Enforces one-way dependencies between domain, application, adapters, and delivery.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn application_sources() -> Vec<PathBuf> {
    rust_sources(root().join("src/application").as_path())
}

fn adapter_sources() -> Vec<PathBuf> {
    let root = root();
    let mut sources = vec![
        root.join("src/gemini/access.rs"),
        root.join("src/gemini/understanding.rs"),
    ];
    sources.extend(rust_sources(
        root.join("src/generation/card_production").as_path(),
    ));
    sources.extend(rust_sources(root.join("src/publishing").as_path()));
    sources
}

fn console_sources() -> Vec<PathBuf> {
    let root = root();
    let mut sources = vec![
        root.join("src/cli/error.rs"),
        root.join("src/cli/console.rs"),
        root.join("src/cli/jobs.rs"),
    ];
    sources.extend(rust_sources(root.join("src/cli/session").as_path()));
    sources
}

fn cli_sources_outside_wiring() -> Vec<PathBuf> {
    rust_sources(root().join("src/cli").as_path())
        .into_iter()
        .filter(|path| !path.ends_with("wiring.rs"))
        .collect()
}

fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory).expect("source directory must be readable") {
        let path = entry.expect("source entry must be readable").path();
        if path.is_dir() {
            sources.extend(rust_sources(path.as_path()));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources
}

fn offenders(sources: &[PathBuf], forbidden: &[&str]) -> Vec<String> {
    sources
        .iter()
        .filter(|path| {
            let text = normalized(path);
            forbidden.iter().any(|pattern| text.contains(pattern))
        })
        .map(|path| path.display().to_string())
        .collect()
}

fn normalized(path: &Path) -> String {
    fs::read_to_string(path)
        .expect("source must be readable")
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '{' && *character != '}')
        .collect()
}

#[test]
fn application_never_imports_delivery_or_concrete_adapters() {
    assert_eq!(
        offenders(
            application_sources().as_slice(),
            &[
                "crate::cli",
                "crate::tui",
                "crate::gemini",
                "crate::generation",
                "crate::publishing",
                "crate::anki",
                "crate::report",
                "crate::config",
                "crate::runtime",
                "crate::infrastructure",
            ],
        ),
        Vec::<String>::new(),
        "application use cases must not depend on delivery or concrete adapters"
    );
}

#[test]
fn adapters_never_import_delivery_surfaces() {
    assert_eq!(
        offenders(adapter_sources().as_slice(), &["crate::cli", "crate::tui"]),
        Vec::<String>::new(),
        "Gemini, production, and publishing adapters must not import delivery"
    );
}

#[test]
fn console_never_imports_the_tui() {
    assert_eq!(
        offenders(
            console_sources().as_slice(),
            &[
                "crate::tui",
                "super::bridge",
                "super::terminal",
                "super::shell",
                "super::batch",
            ],
        ),
        Vec::<String>::new(),
        "console and session delivery must not import the TUI"
    );
}

#[test]
fn workflow_adapters_are_composed_only_in_wiring() {
    assert_eq!(
        offenders(
            cli_sources_outside_wiring().as_slice(),
            &[
                "GeminiAccess",
                "GeminiUnderstanding",
                "GeminiCardProduction",
                "StudyPackagePublisher",
                "SystemPublicationClock",
            ],
        ),
        Vec::<String>::new(),
        "concrete workflow adapters escaped the CLI composition root"
    );
}
