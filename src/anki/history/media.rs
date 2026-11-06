//! Media references follow the web reader's HTML decoding and filename rules.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::HistoryError;
use super::manifest::{Media, Note};

/// Referenced media rows, their private source paths and unresolved filenames.
pub(super) struct MediaFiles {
    /// Manifest rows retain each distinct referenced filename.
    pub(super) rows: Vec<Media>,
    /// One source file per hash supplies archive content.
    pub(super) files: BTreeMap<String, PathBuf>,
    /// Missing or unsafe names remain visible in the preview.
    pub(super) missing: Vec<String>,
}

/// Resolve only selected-note media without following files outside the media folder.
pub(super) fn collect(folder: &Path, notes: &[Note]) -> Result<MediaFiles, HistoryError> {
    let mut found = MediaFiles {
        rows: Vec::new(),
        files: BTreeMap::new(),
        missing: Vec::new(),
    };
    let root = if folder.try_exists()? {
        Some(folder.canonicalize()?)
    } else {
        None
    };
    for name in notes
        .iter()
        .flat_map(|note| note.fields.iter().flat_map(|field| references(field)))
        .collect::<BTreeSet<_>>()
    {
        if !filename(&name) || root.is_none() {
            found.missing.push(name);
            continue;
        }
        let path = folder.join(&name);
        if !path.try_exists()? {
            found.missing.push(name);
            continue;
        }
        let path = path.canonicalize()?;
        if path.parent() != root.as_deref() || !path.is_file() {
            found.missing.push(name);
            continue;
        }
        let (sha256, size) = digest(&mut File::open(&path)?)?;
        found.files.insert(sha256.clone(), path);
        found.rows.push(Media { name, sha256, size });
    }
    Ok(found)
}

/// Accept a single portable media filename without filesystem traversal.
#[must_use]
pub(super) fn filename(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// Stream a content fingerprint and its byte count without buffering the file.
pub(super) fn digest(input: &mut impl Read) -> Result<(String, u64), HistoryError> {
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        size += u64::try_from(count).expect("invariant: buffer length fits in u64");
    }
    Ok((hex(&hash.finalize()), size))
}

/// Encode a content fingerprint using lowercase hexadecimal digits.
#[must_use]
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            result.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "nbsp" => Some(' '),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| {
                    entity
                        .strip_prefix('#')
                        .and_then(|number| number.parse().ok())
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                result.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                result.push('&');
                rest = &tail[1..];
            }
        }
    }
    result.push_str(rest);
    result
}

/// Lists every media file name a raw field references through `[sound:]` or `<img src>`.
pub(super) fn references(field: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = field;
    while let Some(position) = rest.find("[sound:") {
        let tail = &rest[position..];
        names.extend(sound(tail));
        rest = &tail["[sound:".len()..];
    }
    let lower = field.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(position) = lower[offset..].find("<img") {
        let start = offset + position;
        names.extend(picture(&field[start..]));
        offset = start + "<img".len();
    }
    names
}

fn sound(field: &str) -> Option<String> {
    let start = field.find("[sound:")? + "[sound:".len();
    let end = field[start..].find(']')? + start;
    Some(decode(&field[start..end])).filter(|name| !name.is_empty())
}

fn picture(field: &str) -> Option<String> {
    let lower = field.to_ascii_lowercase();
    let tag = lower.find("<img")?;
    let source = lower[tag..].find("src=")? + tag + "src=".len();
    let rest = &field[source..];
    let name = match rest.chars().next()? {
        quote @ ('"' | '\'') => {
            let inner = &rest[1..];
            &inner[..inner.find(quote)?]
        }
        _ => {
            let end = rest
                .find(|character: char| {
                    character.is_whitespace() || character == '>' || character == '/'
                })
                .unwrap_or(rest.len());
            &rest[..end]
        }
    };
    Some(decode(name)).filter(|name| !name.is_empty())
}
