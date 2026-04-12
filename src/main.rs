//! Binary entrypoint for the kamishibai Rust rewrite baseline.

use std::process::ExitCode;

/// Execute the CLI wrapper and translate errors into a process exit code.
fn main() -> ExitCode {
    ExitCode::from(kamishibai::cli::run(std::env::args_os().skip(1)))
}
