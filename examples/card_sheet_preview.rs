//! Render a ten-card preview sheet from the local Gemini cache.
//!
//! Walks `~/Library/Caches/kamishibai/cards/<pair>/<key>/` folders, reads the
//! `meta.json` each card stores, reconstructs its `VocabularyEntry`, and pairs it
//! with the sibling `picture.jpg` panel. No network calls. Use it as the
//! visual regression check after editing `src/report/cards.rs`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

use kamishibai::generation::artifact_cache::{ILLUSTRATION_FILE, META_FILE};
use kamishibai::report::{CardSheet, Thumbnail};
use kamishibai::runtime::locations::{SystemContext, cache_root};
use kamishibai::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyEntry, VocabularySource, VocabularyTarget,
};

const CAP: usize = 10;
const MIN_CARDS: usize = 4;

/// Flat schema persisted to `meta.json` inside each card folder by the publish flow.
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

/// Collect every card folder that holds both a meta record and its panel.
fn collect_candidates(cache: &Path) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    let cards = cache.join("cards");
    if !cards.is_dir() {
        return Ok(out);
    }
    let pair_dirs = fs::read_dir(&cards)
        .map_err(|err| anyhow!("cache '{}' is unreadable: {err}", cards.display()))?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_dir());
    for pair_dir in pair_dirs {
        for entry in fs::read_dir(&pair_dir)? {
            let card_dir = entry?.path();
            let meta_path = card_dir.join(META_FILE);
            let picture = card_dir.join(ILLUSTRATION_FILE);
            if !meta_path.is_file() || !picture.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&meta_path)?;
            let record: MetaRecord = match serde_json::from_str(raw.as_str()) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let entry = match record.into_entry() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let digest = card_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
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
