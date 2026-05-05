//! Quick audit: render the report for each single-target fixture and report
//! output sizes. Used during the font-coverage investigation to surface
//! broken pairs (IPA tofu, missing CJK glyphs) and verify subsetting keeps
//! file sizes small.

use std::fs;
use std::path::PathBuf;

use kamishibai::languages::ReportLabels;
use kamishibai::report::{Report, Thumbnail, VocabularyLayout};
use kamishibai::vocabulary::VocabularyDocument;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inputs = manifest
        .join("tests")
        .join("fixtures")
        .join("reference")
        .join("inputs");
    let out = manifest.join("target").join("font-audit");
    fs::create_dir_all(&out).expect("audit output dir must exist");
    let mut paths: Vec<PathBuf> = fs::read_dir(&inputs)
        .expect("inputs dir must exist")
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("single-target-"))
                .unwrap_or(false)
        })
        .collect();
    paths.push(inputs.join("mixed-target-deck.json"));
    paths.sort();
    for path in &paths {
        let document = match VocabularyDocument::load(path) {
            Ok(value) => value,
            Err(error) => {
                println!("SKIP {}: {error}", path.display());
                continue;
            }
        };
        let mut report = Report::new(VocabularyLayout::new(ReportLabels::default()));
        for entry in &document.entries {
            report.append(entry, None);
        }
        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("entry")
            .to_string();
        let output = out.join(format!("{label}.pdf"));
        report
            .save(&output, &Thumbnail::new(150))
            .expect("audit pdf must save");
        let size = fs::metadata(&output)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        println!("{label} -> {} bytes", size);
    }
}
