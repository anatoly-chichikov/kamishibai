//! Stored ZIP v1 writing, bounded JSON and full content-address verification.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::manifest::Manifest;
use super::summary::DeckSummary;
use super::{HistoryError, Preview, SaveGate, Snapshot, media};

const SNAPSHOT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    version: u32,
    written_at: String,
    writer: String,
    snapshot: Content,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Content {
    sha256: String,
    size: u64,
}

struct LimitedBytes {
    bytes: Vec<u8>,
    limit: u64,
}

impl Write for LimitedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let size = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .and_then(|size| u64::try_from(size).ok());
        if size.is_none_or(|size| size > self.limit) {
            return Err(io::Error::other("snapshot exceeds its size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize within the web API size limit before any output file is created.
pub(super) fn encode(manifest: &Manifest) -> Result<Vec<u8>, HistoryError> {
    let mut output = LimitedBytes {
        bytes: Vec::new(),
        limit: SNAPSHOT_LIMIT,
    };
    serde_json::to_writer(&mut output, manifest).map_err(|error| {
        if error.is_io() {
            HistoryError::Oversized
        } else {
            HistoryError::Json(error)
        }
    })?;
    Ok(output.bytes)
}

/// Stage, verify and publish one archive under the next unused destination name.
pub(super) fn save(
    snapshot: &Snapshot,
    directory: &Path,
    written: OffsetDateTime,
    writer: &str,
    gate: &dyn SaveGate,
) -> Result<PathBuf, HistoryError> {
    let directory = destination(snapshot, directory)?;
    let bytes = encode(&snapshot.manifest)?;
    let header = Header {
        format: "kamishibai-anki-history".into(),
        version: 1,
        written_at: written
            .to_offset(time::UtcOffset::UTC)
            .format(&Rfc3339)
            .map_err(|error| HistoryError::Invalid(error.to_string()))?,
        writer: writer.into(),
        snapshot: Content {
            sha256: media::hex(&Sha256::digest(&bytes)),
            size: u64::try_from(bytes.len()).expect("invariant: bounded snapshot size fits in u64"),
        },
    };
    gate.verify()?;
    fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".kamishibai-anki-")
        .suffix(".part")
        .tempfile_in(&directory)?;
    let mut zip = ZipWriter::new(temporary.as_file_mut());
    zip.start_file("header.json", options(0))?;
    zip.write_all(&serde_json::to_vec(&header)?)?;
    zip.start_file("snapshot.json", options(header.snapshot.size))?;
    zip.write_all(&bytes)?;
    for (hash, path) in &snapshot.source.files {
        let mut file = File::open(path)?;
        zip.start_file(format!("media/{hash}"), options(file.metadata()?.len()))?;
        io::copy(&mut file, &mut zip)?;
    }
    zip.finish()?.sync_all()?;
    validate(temporary.path())?;
    let date = written.to_offset(time::UtcOffset::UTC).date();
    let stem = format!("kamishibai-anki-{date}");
    let mut number = 1_u64;
    loop {
        let name = if number == 1 {
            format!("{stem}.zip")
        } else {
            format!("{stem}-{number}.zip")
        };
        let path = directory.join(name);
        gate.verify()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                temporary = error.file
            }
            Err(error) => return Err(error.error.into()),
        }
        number = number
            .checked_add(1)
            .expect("invariant: output collision count fits in u64");
    }
}

pub(super) fn destination(snapshot: &Snapshot, directory: &Path) -> Result<PathBuf, HistoryError> {
    let directory = resolve(directory)?;
    let profile = &snapshot.source.profile;
    let root = profile
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("Anki2"))
        })
        .unwrap_or(profile);
    if directory.starts_with(root) {
        return Err(HistoryError::Destination);
    }
    Ok(directory)
}

fn resolve(directory: &Path) -> io::Result<PathBuf> {
    let absolute = std::env::current_dir()?.join(directory);
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::Normal(_) => {
                resolved.push(component.as_os_str());
                match resolved.canonicalize() {
                    Ok(path) => resolved = path,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        match resolved.symlink_metadata() {
                            Err(missing) if missing.kind() == io::ErrorKind::NotFound => {}
                            _ => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            component => resolved.push(component.as_os_str()),
        }
    }
    Ok(resolved)
}

fn options(size: u64) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o600)
        .large_file(size >= u64::from(u32::MAX))
}

/// Verify format v1, ZIP CRC, exact entry membership, snapshot size/hash and media.
/// Returns counts recalculated from the snapshot; no archive member is extracted.
pub fn validate(path: &Path) -> Result<Preview, HistoryError> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    if central_count(&mut File::open(path)?, archive.central_directory_start())? != archive.len() {
        return Err(invalid("duplicate archive entries"));
    }
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.compression() != CompressionMethod::Stored
            || file.encrypted()
            || file.is_dir()
            || file.is_symlink()
        {
            return Err(invalid("entries must be unencrypted stored files"));
        }
    }
    let header: Header = serde_json::from_slice(&bounded(&mut archive, "header.json", 65_536)?)?;
    if header.format != "kamishibai-anki-history"
        || header.version != 1
        || header.writer.is_empty()
        || OffsetDateTime::parse(&header.written_at, &Rfc3339).is_err()
    {
        return Err(invalid("unsupported header format or version"));
    }
    if header.snapshot.size > SNAPSHOT_LIMIT {
        return Err(HistoryError::Oversized);
    }
    let bytes = bounded(&mut archive, "snapshot.json", SNAPSHOT_LIMIT)?;
    if u64::try_from(bytes.len()).expect("invariant: bounded snapshot size fits in u64")
        != header.snapshot.size
        || media::hex(&Sha256::digest(&bytes)) != header.snapshot.sha256
    {
        return Err(invalid("snapshot hash or size differs from the header"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    membership(&manifest)?;
    let mut expected = BTreeMap::new();
    let mut names = BTreeSet::new();
    for file in &manifest.media {
        if !media::filename(&file.name)
            || !names.insert(&file.name)
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid("invalid or duplicate media name or hash"));
        }
        if expected
            .insert(format!("media/{}", file.sha256), file.size)
            .is_some_and(|size| size != file.size)
        {
            return Err(invalid("one media hash has conflicting sizes"));
        }
    }
    let entries: BTreeSet<_> = archive.file_names().map(str::to_owned).collect();
    let required: BTreeSet<_> = expected
        .keys()
        .cloned()
        .chain(["header.json".into(), "snapshot.json".into()])
        .collect();
    if entries != required {
        return Err(invalid("extra or missing archive entries"));
    }
    for (name, size) in expected {
        let mut file = archive.by_name(&name)?;
        if file.size() != size {
            return Err(invalid("media size differs from the snapshot"));
        }
        let (hash, actual) = media::digest(&mut file)?;
        if name != format!("media/{hash}") || actual != size {
            return Err(invalid("media hash or size differs from the snapshot"));
        }
    }
    let references: BTreeSet<_> = manifest
        .notes
        .iter()
        .flat_map(|note| {
            note.fields
                .iter()
                .flat_map(|field| media::references(field))
        })
        .collect();
    let missing = references
        .into_iter()
        .filter(|name| !names.contains(name))
        .collect();
    let decks = manifest
        .decks
        .iter()
        .map(|deck| DeckSummary::from_rows(deck, true, &manifest.cards, &manifest.revlog))
        .collect();
    Ok(Preview::new(&manifest, decks, missing, bytes.len()))
}

fn bounded(
    archive: &mut ZipArchive<File>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, HistoryError> {
    let file = archive.by_name(name)?;
    if file.size() > limit {
        return Err(invalid("JSON entry exceeds its size limit"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).expect("invariant: bounded JSON size fits in u64") > limit {
        return Err(invalid("JSON entry exceeds its size limit"));
    }
    Ok(bytes)
}

fn membership(manifest: &Manifest) -> Result<(), HistoryError> {
    let decks: BTreeSet<_> = manifest.decks.iter().map(|deck| deck.id).collect();
    let types: BTreeSet<_> = manifest.notetypes.iter().map(|kind| kind.id).collect();
    let notes: BTreeSet<_> = manifest.notes.iter().map(|note| note.id).collect();
    let cards: BTreeSet<_> = manifest.cards.iter().map(|card| card.id).collect();
    if manifest.collection.schema != 18
        || !manifest.collection.profile.is_empty()
        || decks.len() != manifest.decks.len()
        || types.len() != manifest.notetypes.len()
        || notes.len() != manifest.notes.len()
        || cards.len() != manifest.cards.len()
        || manifest
            .notes
            .iter()
            .any(|note| !types.contains(&note.notetype))
        || manifest
            .cards
            .iter()
            .any(|card| !decks.contains(&card.deck) || !notes.contains(&card.note))
        || manifest
            .revlog
            .iter()
            .any(|review| !cards.contains(&review.card))
    {
        return Err(invalid(
            "snapshot has an unsupported collection or dangling rows",
        ));
    }
    Ok(())
}

/// Count the original central-directory entries because ZIP's name map deduplicates them.
fn central_count(file: &mut File, start: u64) -> Result<usize, HistoryError> {
    file.seek(SeekFrom::Start(start))?;
    let mut count = 0;
    loop {
        let mut signature = [0; 4];
        file.read_exact(&mut signature)?;
        match signature {
            [0x50, 0x4b, 0x05, 0x06] | [0x50, 0x4b, 0x06, 0x06] => return Ok(count),
            [0x50, 0x4b, 0x01, 0x02] => {
                let mut header = [0; 42];
                file.read_exact(&mut header)?;
                let length: u32 = [24, 26, 28]
                    .iter()
                    .map(|index| {
                        u32::from(u16::from_le_bytes([header[*index], header[*index + 1]]))
                    })
                    .sum();
                file.seek(SeekFrom::Current(i64::from(length)))?;
                count += 1;
            }
            _ => return Err(invalid("invalid central directory")),
        }
    }
}

fn invalid(message: &str) -> HistoryError {
    HistoryError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_json_cannot_allocate_past_its_limit() {
        let mut output = LimitedBytes {
            bytes: vec![1, 2, 3],
            limit: 7,
        };
        let result = output.write_all(&[4, 5, 6, 7, 8]);
        assert!(
            result.is_err() && output.bytes == [1, 2, 3],
            "bounded serialization allocated beyond its limit"
        );
    }
}
