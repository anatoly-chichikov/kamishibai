//! Binary entrypoint for the kamishibai Rust rewrite baseline.

use std::process::ExitCode;

/// Execute the CLI wrapper and translate errors into a process exit code.
fn main() -> ExitCode {
    match kamishibai::cli::run(std::env::args_os()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
