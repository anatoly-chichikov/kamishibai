//! Rebuild the CLI's embedded reader-language data from pinned, extracted crates.
mod byte_model;
mod profiles;
mod verbs;
use anyhow::{Result, ensure};
use flate2::{Compression, GzBuilder};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn gzip(path: &Path, data: &[u8]) -> Result<()> {
    let mut output = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(fs::File::create(path)?, Compression::best());
    output.write_all(data)?;
    output.finish()?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 2,
        "expected an extracted Cargo registry directory and an output directory"
    );
    let profiles = args[1].join("src/languages/reader/data");
    fs::create_dir_all(&profiles)?;
    profiles::pack(&args[0], &profiles)?;
    byte_model::pack(&args[0], &profiles)?;
    verbs::pack(&args[0], &args[1].join("crates/reader-verbs"))
}
