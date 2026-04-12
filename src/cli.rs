//! CLI orchestration placeholders for the Rust rewrite baseline.

use std::ffi::OsString;

use anyhow::Result;

/// Execute the bootstrap CLI entrypoint.
pub fn run<I>(_args: I) -> Result<u8>
where
    I: IntoIterator<Item = OsString>,
{
    Ok(0)
}
