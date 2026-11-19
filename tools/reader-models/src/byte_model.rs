//! Keep the shared-script class columns and original feature automaton from langid-rs.
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

fn number(input: &mut impl Read) -> Result<usize> {
    let mut value = [0; 4];
    input.read_exact(&mut value)?;
    Ok(usize::try_from(u32::from_le_bytes(value))?)
}

fn bytes(input: &mut impl Read, size: usize) -> Result<Vec<u8>> {
    let mut data = vec![0; size];
    input.read_exact(&mut data)?;
    Ok(data)
}

fn count(output: &mut Vec<u8>, n: usize) -> Result<()> {
    output.extend_from_slice(&u32::try_from(n)?.to_le_bytes());
    Ok(())
}

/// Write the per-feature weights and priors for the supported ambiguous classes.
pub(super) fn pack(registry: &Path, out: &Path) -> Result<()> {
    let mut input = BufReader::new(File::open(registry.join("langid-rs-1.1.0/src/model.bin"))?);
    let rows = number(&mut input)?;
    let columns = number(&mut input)?;
    let weights = bytes(&mut input, 4 * rows * columns)?;
    ensure!(
        number(&mut input)? == columns,
        "langid model priors and weights disagree"
    );
    let priors = bytes(&mut input, 4 * columns)?;
    let states = number(&mut input)?;
    let moves = bytes(&mut input, 2 * states)?;
    ensure!(
        number(&mut input)? == columns,
        "langid model classes and weights disagree"
    );
    let mut codes = Vec::new();
    for _ in 0..columns {
        let len = number(&mut input)?;
        codes.push(String::from_utf8(bytes(&mut input, len)?)?);
    }
    let allowed: BTreeSet<_> = super::profiles::GROUPS
        .iter()
        .flat_map(|(_, codes)| codes.split_whitespace())
        .chain(["ja", "ko", "zh"])
        .collect();
    let retained: Vec<_> = codes
        .iter()
        .enumerate()
        .filter(|(_, code)| allowed.contains(code.as_str()))
        .collect();
    ensure!(
        retained.len() == allowed.len(),
        "langid model lacks at least one shared-script class"
    );
    let mut output = Vec::new();
    count(&mut output, rows)?;
    count(&mut output, retained.len())?;
    for row in weights
        .chunks_exact(4 * columns)
        .chain(std::iter::once(priors.as_slice()))
    {
        for &(i, _) in &retained {
            output.extend_from_slice(&row[4 * i..4 * (i + 1)]);
        }
    }
    count(&mut output, states)?;
    output.extend_from_slice(&moves);
    for (_, code) in retained {
        ensure!(code.len() == 2, "unexpected langid class {code}");
        output.extend_from_slice(code.as_bytes());
    }
    input
        .read_to_end(&mut output)
        .context("reading byte feature emissions")?;
    super::gzip(&out.join("byte-ngrams.gz"), &output)
}
