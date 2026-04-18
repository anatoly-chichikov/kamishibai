//! Binary entrypoint for the canonical kamishibai runtime.

use std::process::ExitCode;

/// Execute the TUI wrapper and translate errors into a process exit code.
fn main() -> ExitCode {
    ExitCode::from(kamishibai::cli::run())
}
