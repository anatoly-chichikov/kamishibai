//! Standalone benchmark for the cached publish flow.
//!
//! Usage:
//!   cargo run --release --example bench_publish -- /path/to/batch.json
//!
//! Loads the JSON document, builds drafts as if every artifact had already
//! materialized in the on-disk cache, then drives publish() and prints a
//! breakdown of where the wall-clock time goes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use kamishibai::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use kamishibai::languages::{ReportLabels, catalog, naming};
use kamishibai::report::{Report, Thumbnail, VocabularyLayout, warm_fonts_async};
use kamishibai::session::{LanguagePair, from_entry, to_entry};
use kamishibai::vocabulary::{VocabularyDocument, VocabularyEntry};

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: bench_publish <batch.json>"))?;
    let path = PathBuf::from(path);
    let cache_root = home_cache()?.join("kamishibai");
    let output = std::env::temp_dir().join("kamishibai-bench-out");
    fs::create_dir_all(&output)?;

    if std::env::var_os("BENCH_WARM_FONTS").is_some() {
        eprintln!("[bench] warming fonts in background");
        warm_fonts_async();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let t0 = Instant::now();
    let document = VocabularyDocument::load(&path)?;
    eprintln!(
        "[bench] load+validate {} entries: {:.2?}",
        document.entries.len(),
        t0.elapsed()
    );

    let pair = pair_from_document(&document)?;
    let target_lang = pair.target().to_string();
    let support_lang = pair.support().to_string();
    let entries: Vec<VocabularyEntry> = document.entries.clone();

    let item = catalog().item(target_lang.as_str())?;
    let manga_dir = cache_root.join(item.image_cache.as_str());
    let audio_dir = cache_root.join(item.audio_cache.as_str());

    let t1 = Instant::now();
    let drafts: Vec<_> = entries
        .iter()
        .map(|entry| from_entry(entry, pair.clone()))
        .collect();
    eprintln!("[bench] from_entry drafts: {:.2?}", t1.elapsed());

    let t2 = Instant::now();
    let decknaming = naming(None, entries.as_slice());
    let model = CardModel::new().model();
    let mut container = VocabularyDeck::new(
        StableId::new(decknaming.name.as_str()).value(),
        decknaming.name.as_str(),
        VocabularyNote::new(model),
        Vec::<PathBuf>::new(),
    );
    let mut report = Report::new(VocabularyLayout::new(ReportLabels::default()));
    let mut attached = 0usize;
    let mut missing = 0usize;
    for (draft, entry) in drafts.iter().zip(entries.iter()) {
        let target_sentence = entry.target.sentence.as_str();
        let audio_name = format!("{}.wav", &md5_hex(target_sentence)[..12]);
        let audio_path = audio_dir.join(&audio_name);
        let picture_digest = md5_hex(&format!("{}\0{}", target_lang, target_sentence));
        let picture_name = format!("{}.jpg", &picture_digest[..12]);
        let picture_path = manga_dir.join(&picture_name);
        if !audio_path.exists() || !picture_path.exists() {
            missing += 1;
            continue;
        }
        let real_entry = to_entry(draft)?;
        container.attach(audio_path.clone());
        container.attach(picture_path.clone());
        container.add(
            &real_entry,
            format!("[sound:{}]", audio_name).as_str(),
            format!("<img src='{}' style='{IMAGE_STYLE}'>", picture_name).as_str(),
        );
        report.append(&real_entry, Some(picture_path));
        attached += 1;
    }
    eprintln!(
        "[bench] assemble (attach+report.append) {} cards ({} missing): {:.2?}",
        attached,
        missing,
        t2.elapsed()
    );
    if attached == 0 {
        bail!(
            "no cached cards found for target lang '{target_lang}' in {}",
            cache_root.display()
        );
    }

    let t3 = Instant::now();
    let apkg = output.join(format!("{}_bench.apkg", decknaming.prefix));
    container.save(&apkg)?;
    eprintln!("[bench] container.save (apkg): {:.2?}", t3.elapsed());

    let t4 = Instant::now();
    let pdf = output.join(format!("{}_bench.pdf", decknaming.prefix));
    report.save(&pdf, &Thumbnail::new(150))?;
    eprintln!("[bench] report.save (pdf): {:.2?}", t4.elapsed());

    eprintln!("[bench] total: {:.2?}", t0.elapsed());
    eprintln!(
        "[bench] outputs: {} ({} bytes), {} ({} bytes)",
        apkg.display(),
        fs::metadata(&apkg).map(|m| m.len()).unwrap_or(0),
        pdf.display(),
        fs::metadata(&pdf).map(|m| m.len()).unwrap_or(0),
    );

    let _ = support_lang;
    Ok(())
}

fn pair_from_document(document: &VocabularyDocument) -> Result<LanguagePair> {
    let first = document
        .entries
        .first()
        .ok_or_else(|| anyhow!("vocabulary document contains no entries"))?;
    Ok(LanguagePair::new(
        first.target.lang.as_str(),
        first.source.lang.as_str(),
    ))
}

fn md5_hex(text: &str) -> String {
    format!("{:x}", md5::compute(text.as_bytes()))
}

fn home_cache() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME not set"))?;
    Ok(Path::new(&home).join("Library").join("Caches"))
}
