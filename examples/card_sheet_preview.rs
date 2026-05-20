//! Render a ten-card preview sheet from the local Gemini cache.
//!
//! Walks `~/Library/Caches/kamishibai/meta-*/*.json` (the flat card-meta
//! payload that `LiveCardGenerator::store_card_meta` writes after every Gemini Pro
//! pass), reconstructs each `VocabularyEntry`, and pairs it with the
//! matching `manga-{lang}/{digest}.jpg` panel using the same MD5 key the
//! production flow computes. No network calls. Use it as the visual
//! regression check after editing `src/report/cards.rs`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

use kamishibai::report::{CardSheet, Thumbnail};
use kamishibai::runtime::locations::{SystemContext, cache_root};
use kamishibai::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyEntry, VocabularySource, VocabularyTarget,
};

const CAP: usize = 10;
const MIN_CARDS: usize = 4;

/// Flat schema persisted to `meta-{lang}/<digest>.json` by the publish flow.
#[derive(Debug, Deserialize)]
struct MetaRecord {
    term: String,
    meaning: String,
    pronunciation: String,
    transcription: String,
    importance: u8,
    source_sentence: String,
    source_lang: String,
    source_highlight: String,
    source_hint: String,
    source_context: String,
    target_sentence: String,
    target_lang: String,
}

impl MetaRecord {
    /// Build one strict `VocabularyEntry` out of the cached flat payload.
    fn into_entry(self) -> Result<VocabularyEntry> {
        Ok(VocabularyEntry {
            term: NonEmptyText::new(self.term)?,
            meaning: NonEmptyText::new(self.meaning)?,
            pronunciation: NonEmptyText::new(self.pronunciation)?,
            transcription: NonEmptyText::new(self.transcription)?,
            importance: Importance::new(self.importance)?,
            source: VocabularySource {
                sentence: NonEmptyText::new(self.source_sentence)?,
                lang: LanguageCode::new(self.source_lang)?,
                highlight: NonEmptyText::new(self.source_highlight)?,
                hint: NonEmptyText::new(self.source_hint)?,
                context: NonEmptyText::new(self.source_context)?,
            },
            target: VocabularyTarget {
                sentence: NonEmptyText::new(self.target_sentence)?,
                lang: LanguageCode::new(self.target_lang)?,
            },
        })
    }
}

/// One ready-to-render preview candidate: entry plus the on-disk manga path.
#[derive(Debug)]
struct Candidate {
    digest: String,
    entry: VocabularyEntry,
    picture: PathBuf,
}

/// Return the first twelve hex chars of `md5(lang + "\0" + target_sentence)`.
/// Matches `src/generation/picture.rs` and the inline computation in
/// `LiveCardGenerator::store_card_meta`.
fn manga_digest(target_lang: &str, target_sentence: &str) -> String {
    let payload = format!("{}\0{}", target_lang, target_sentence);
    let full = format!("{:x}", md5::compute(payload));
    full[..12].to_string()
}

/// Collect every meta record whose manga panel also exists on disk.
fn collect_candidates(cache: &Path) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    let meta_dirs = fs::read_dir(cache)
        .map_err(|err| anyhow!("cache root '{}' is unreadable: {err}", cache.display()))?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("meta-"))
        });
    for meta_dir in meta_dirs {
        let lang = meta_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("meta-"))
            .map(str::to_string)
            .ok_or_else(|| anyhow!("meta dir '{}' has no language tag", meta_dir.display()))?;
        let manga_dir = cache.join(format!("manga-{lang}"));
        if !manga_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&meta_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path)?;
            let record: MetaRecord = match serde_json::from_str(raw.as_str()) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let digest = manga_digest(record.target_lang.as_str(), record.target_sentence.as_str());
            let picture = manga_dir.join(format!("{digest}.jpg"));
            if !picture.is_file() {
                continue;
            }
            let entry = match record.into_entry() {
                Ok(value) => value,
                Err(_) => continue,
            };
            out.push(Candidate {
                digest,
                entry,
                picture,
            });
        }
    }
    out.sort_by(|a, b| a.digest.cmp(&b.digest));
    Ok(out)
}

fn main() -> Result<()> {
    let context = SystemContext;
    let cache = cache_root(&context)?;
    if !cache.is_dir() {
        bail!(
            "cache root '{}' does not exist — run the TUI once to populate it",
            cache.display()
        );
    }
    let mut candidates = collect_candidates(cache.as_path())?;
    if candidates.len() < MIN_CARDS {
        bail!(
            "cache holds {} card(s) with matching manga panels — need at least {} to preview \
             (run the TUI to populate it, cache at '{}')",
            candidates.len(),
            MIN_CARDS,
            cache.display()
        );
    }
    candidates.truncate(CAP);
    let langs: BTreeSet<String> = candidates
        .iter()
        .map(|item| item.entry.target.lang.as_str().to_string())
        .collect();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = manifest.join("target").join("card-sheet-preview");
    fs::create_dir_all(&out)?;
    let mut sheet = CardSheet::new();
    for candidate in &candidates {
        sheet.append(&candidate.entry, Some(candidate.picture.clone()));
    }
    let pdf = out.join("preview.pdf");
    sheet.save(&pdf, &Thumbnail::new(1024))?;
    println!(
        "wrote {} bytes -> {} ({} real cards, target langs: {})",
        fs::metadata(&pdf)?.len(),
        pdf.display(),
        candidates.len(),
        langs.into_iter().collect::<Vec<_>>().join(", ")
    );
    for candidate in &candidates {
        println!(
            "  · {} [{}] → {}",
            candidate.entry.term.as_str(),
            candidate.entry.target.lang.as_str(),
            candidate.picture.display()
        );
    }
    Ok(())
}
