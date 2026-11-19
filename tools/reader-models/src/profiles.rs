//! Pack the word frequencies and character n-grams for ambiguous writing systems.
use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Source model codes by writing system, including Croatian evidence for Serbian Latin.
pub(super) const GROUPS: [(&str, &str); 3] = [
    (
        "latin",
        "cs de en es fi fr hr hu id it la nl pl pt ro sv tr vi",
    ),
    ("cyrillic", "ru uk sr"),
    ("arabic", "ar fa"),
];

fn source(registry: &Path, name: &str) -> Result<PathBuf> {
    ["a", "b"]
        .into_iter()
        .map(|part| {
            registry
                .join(format!("langidentify-models-lite-{part}-0.1.0/data"))
                .join(name)
        })
        .find(|path| path.is_file())
        .with_context(|| format!("missing lite model {name}"))
}

fn weights(path: &Path) -> Result<Vec<(String, u16)>> {
    let mut lines = BufReader::new(GzDecoder::new(File::open(path)?)).lines();
    let header = lines.next().context("missing lite model header")??;
    let mut totals = BTreeMap::<usize, f64>::new();
    for field in header.split_whitespace() {
        if let Some((length, counts)) = field.split_once(':')
            && let Some((_, total)) = counts.split_once('/')
        {
            totals.insert(length.parse()?, total.parse()?);
        }
    }
    let mut rows = Vec::new();
    for line in lines {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, value) = line
            .rsplit_once(' ')
            .context("missing lite model frequency")?;
        let probability = if value.starts_with('-') {
            value.parse::<f64>()?
        } else {
            (value.parse::<f64>()?
                / totals
                    .get(&word.chars().count())
                    .context("missing n-gram length")?)
            .ln()
        };
        if probability >= -12.0 {
            ensure!(
                probability <= 0.0,
                "positive log probability in {}",
                path.display()
            );
            let weight = format!("{:.0}", (12.0 + probability) * 256.0).parse::<u16>()?;
            rows.push((word.to_owned(), weight));
        }
    }
    Ok(rows)
}

/// Write one compressed table per shared writing system.
pub(super) fn pack(registry: &Path, out: &Path) -> Result<()> {
    for (group, codes) in GROUPS {
        let mut rows = BTreeMap::<(char, String), Vec<String>>::new();
        for (index, code) in codes.split_whitespace().enumerate() {
            for (kind, prefix) in [('g', "ngrams"), ('w', "topwords")] {
                let filename = format!("{prefix}-{code}.txt.gz");
                for (word, weight) in weights(&source(registry, &filename)?)? {
                    rows.entry((kind, word))
                        .or_default()
                        .push(format!("{index}:{weight}"));
                }
            }
        }
        let mut text = String::from(codes);
        text.push('\n');
        for ((kind, word), weights) in rows {
            use std::fmt::Write;
            writeln!(&mut text, "{kind}{word}\t{}", weights.join(","))?;
        }
        super::gzip(&out.join(format!("{group}.gz")), text.as_bytes())?;
    }
    Ok(())
}
