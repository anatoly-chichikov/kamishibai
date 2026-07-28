//! Version-matched agent contract embedded in every binary.

use std::io::{self, Write};

use anyhow::Result;

const CONTRACT: &str = include_str!("../../llms.txt");

/// Print the exact contract compiled into this binary.
pub(super) fn print() -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(CONTRACT.as_bytes())?;
    Ok(())
}
