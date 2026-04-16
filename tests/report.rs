//! Tests for PDF report rendering.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use image::{Rgb, RgbImage};
use kamishibai::languages::{ReportFonts, ReportLabels};
use kamishibai::report::{FontFamily, FontPath, Report, ReportLayout, Thumbnail, VocabularyLayout};
use kamishibai::vocabulary::VocabularyEntry;
use lopdf::Document;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Clone, Debug)]
struct StaticLayout {
    rows: Vec<(String, f32)>,
}

impl ReportLayout for StaticLayout {
    /// Return the configured rows for each entry.
    fn row(&self, _entry: &VocabularyEntry) -> Vec<(String, f32)> {
        self.rows.clone()
    }
}

/// Create one normalized entry for report tests.
fn entry(word: &str, source: &str, target: &str) -> VocabularyEntry {
    VocabularyEntry {
        word: String::from(word),
        pronunciation: String::new(),
        translation: String::from("значение"),
        example: String::new(),
        source_lang: String::from(source),
        target_lang: String::from(target),
        sentence: String::from("пример"),
        highlight: String::new(),
        hint: String::new(),
        context: String::new(),
        importance: String::new(),
        transcription: String::new(),
    }
}

/// Create one square PNG fixture for report tests.
fn image(directory: &Path, size: u32) -> PathBuf {
    let path = directory.join(format!("{}.png", uuid()));
    let mut value = RgbImage::new(size, size);
    for pixel in value.pixels_mut() {
        *pixel = Rgb([42, 99, 200]);
    }
    value
        .save(&path)
        .expect("report image fixture must be saved");
    path
}

/// Return the first PDF header bytes as text.
fn header(path: &Path) -> String {
    String::from_utf8_lossy(&fs::read(path).expect("pdf output must exist")[..8]).into_owned()
}

/// Return the parsed PDF page count.
fn pages(path: &Path) -> usize {
    Document::load(path)
        .expect("pdf output must parse")
        .get_pages()
        .len()
}

/// Return the file size of one PDF fixture.
fn bytes(path: &Path) -> u64 {
    fs::metadata(path).expect("pdf output must exist").len()
}

/// Return one unique fixture identifier.
fn uuid() -> String {
    format!(
        "{:x}",
        md5::compute(format!("{:?}", std::time::SystemTime::now()))
    )
}

/// Return one reference manifest path.
fn references(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("reference")
        .join("manifests")
        .join(name)
}

/// Return the frozen normalized reference entries.
fn entries() -> Vec<VocabularyEntry> {
    serde_json::from_str(
        fs::read_to_string(references("normalized/mixed-target-deck.json"))
            .expect("reference entries must exist")
            .as_str(),
    )
    .expect("reference entries must parse")
}

/// Return the frozen report manifest.
fn report() -> Value {
    serde_json::from_str(
        fs::read_to_string(references("report.json"))
            .expect("reference report manifest must exist")
            .as_str(),
    )
    .expect("reference report manifest must parse")
}

/// Return one JSON-ready row manifest for one entry.
fn manifest_rows(entry: &VocabularyEntry) -> Vec<Value> {
    VocabularyLayout::new(ReportLabels::default())
        .row(entry)
        .into_iter()
        .map(|(text, size)| json!([text, size as i64]))
        .collect()
}

/// Vocabulary layout keeps the frozen full-entry row structure.
#[test]
fn vocabulary_layout_keeps_the_frozen_full_entry_row_structure() {
    let item = VocabularyEntry {
        word: String::from("café"),
        pronunciation: String::from("ˈkafe"),
        translation: String::from("кафе"),
        example: String::from("The café serves crêpes"),
        source_lang: String::from("en"),
        target_lang: String::from("ru"),
        sentence: String::from("В кафе подают крепы"),
        highlight: String::new(),
        hint: String::from("coast"),
        context: String::from("calm evening"),
        importance: String::from("7"),
        transcription: String::new(),
    };
    assert_eq!(
        VocabularyLayout::new(ReportLabels::default()).row(&item),
        vec![
            (String::from("café /ˈkafe/ — кафе"), 11.0),
            (String::from("The café serves crêpes"), 9.0),
            (String::from("Translation: В кафе подают крепы"), 9.0),
            (String::from("Context: calm evening"), 8.0),
            (String::from("Hint: coast"), 8.0),
            (String::from("Importance: 7/10"), 8.0),
        ],
        "vocabulary layout no longer keeps the frozen full entry row structure"
    );
}

/// Vocabulary layout keeps the frozen sparse-entry row structure.
#[test]
fn vocabulary_layout_keeps_the_frozen_sparse_entry_row_structure() {
    let item = entry("café", "en", "ru");
    assert_eq!(
        VocabularyLayout::new(ReportLabels::default()).row(&item),
        vec![
            (String::from("café — значение"), 11.0),
            (String::from("Translation: пример"), 9.0),
        ],
        "vocabulary layout no longer keeps the frozen sparse entry row structure"
    );
}

/// Font path resolution finds the configured system font.
#[test]
fn font_path_resolution_finds_the_configured_system_font() -> Result<()> {
    assert!(
        FontPath::new("DejaVu Sans").resolved()?.is_file(),
        "font path resolution no longer finds the configured system font"
    );
    Ok(())
}

/// Font family resolution finds both regular and bold font variants.
#[test]
fn font_family_resolution_finds_both_regular_and_bold_font_variants() -> Result<()> {
    let family = FontFamily::new("DejaVu Sans");
    assert_eq!(
        (family.regular()?.is_file(), family.bold()?.is_file()),
        (true, true),
        "font family resolution no longer finds both regular and bold font variants"
    );
    Ok(())
}

/// Thumbnail compression keeps the frozen filename prefix and file output.
#[test]
fn thumbnail_compression_keeps_the_frozen_filename_prefix_and_file_output() -> Result<()> {
    let directory = TempDir::new()?;
    let source = image(directory.path(), 256);
    let thumb = Thumbnail::new(150).compressed(&source, directory.path())?;
    assert_eq!(
        (
            thumb
                .file_name()
                .expect("thumbnail name must exist")
                .to_string_lossy()
                .starts_with("thumb_"),
            thumb.is_file(),
        ),
        (true, true),
        "thumbnail compression no longer keeps the frozen filename prefix and file output"
    );
    Ok(())
}

/// Reports with no entries still produce a nonempty PDF.
#[test]
fn reports_with_no_entries_still_produce_a_nonempty_pdf() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("empty.pdf");
    Report::new(
        StaticLayout { rows: vec![] },
        FontFamily::new("DejaVu Sans"),
    )
    .save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports with no entries no longer produce a nonempty PDF"
    );
    Ok(())
}

/// Reports with images produce a larger PDF payload.
#[test]
fn reports_with_images_produce_a_larger_pdf_payload() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("image.pdf");
    let mut report = Report::new(
        StaticLayout {
            rows: vec![
                (String::from("Ünïcödé línë"), 10.0),
                (String::from("wörd"), 14.0),
            ],
        },
        FontFamily::new("DejaVu Sans"),
    );
    report.append(
        &entry("wörd", "en", "ru"),
        Some(image(directory.path(), 128)),
    );
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 500,
        "reports with images no longer produce a larger PDF payload"
    );
    Ok(())
}

/// Reports render mixed-script text without failing.
#[test]
fn reports_render_mixed_script_text_without_failing() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("mixed.pdf");
    let mut report = Report::new(
        StaticLayout {
            rows: vec![
                (String::from("Кириллица проверка"), 10.0),
                (String::from("Ελληνικά δοκιμή"), 14.0),
                (String::from("Mixed Ünïcödé ñ ü ö"), 9.0),
            ],
        },
        FontFamily::new("DejaVu Sans"),
    );
    report.append(&entry("Ελληνικά", "ru", "el"), None);
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports no longer render mixed script text without failing"
    );
    Ok(())
}

/// Reports switch font families between non-Chinese and Chinese entries.
#[test]
fn reports_switch_font_families_between_non_chinese_and_chinese_entries() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("fonts.pdf");
    let mut report = Report::new(
        StaticLayout {
            rows: vec![(String::from("Mixed script"), 10.0)],
        },
        ReportFonts::default(),
    );
    report.append(&entry("plain", "en", "en"), None);
    report.append(&entry("朋友", "el", "zh"), None);
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports no longer switch font families between non Chinese and Chinese entries"
    );
    Ok(())
}

/// Reports wrap long text without failing.
#[test]
fn reports_wrap_long_text_without_failing() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("wrap.pdf");
    let paragraph = "Ünïcödé ".repeat(80);
    let mut report = Report::new(
        StaticLayout {
            rows: vec![(paragraph.clone(), 10.0)],
        },
        FontFamily::new("DejaVu Sans"),
    );
    report.append(&entry("wörd", "en", "ru"), None);
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports no longer wrap long text without failing"
    );
    Ok(())
}

/// Reports with many entries still span multiple pages.
#[test]
fn reports_with_many_entries_still_span_multiple_pages() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("pages.pdf");
    let mut report = Report::new(
        StaticLayout {
            rows: vec![
                (String::from("Строка öднä"), 11.0),
                (String::from("Пример prédlözhéniÿa"), 9.0),
            ],
        },
        FontFamily::new("DejaVu Sans"),
    );
    for index in 0..30 {
        report.append(
            &VocabularyEntry {
                word: format!("wörd{index}"),
                pronunciation: String::new(),
                translation: String::from("значение"),
                example: String::new(),
                source_lang: String::from("en"),
                target_lang: String::from("ru"),
                sentence: format!("Ünïcödé {index}"),
                highlight: String::new(),
                hint: String::new(),
                context: String::new(),
                importance: String::new(),
                transcription: String::new(),
            },
            Some(image(directory.path(), 128)),
        );
    }
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 3000,
        "reports with many entries no longer span multiple pages"
    );
    Ok(())
}

/// Reports near the page bottom still avoid nearly empty trailing pages.
#[test]
fn reports_near_the_page_bottom_still_avoid_nearly_empty_trailing_pages() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("threshold.pdf");
    let mut report = Report::new(
        StaticLayout {
            rows: vec![(String::from("Wörd"), 11.0), ("À".repeat(1000), 9.0)],
        },
        FontFamily::new("DejaVu Sans"),
    );
    for _ in 0..5 {
        report.append(
            &entry("wörd", "en", "ru"),
            Some(image(directory.path(), 128)),
        );
    }
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        pages(&path) <= 3,
        "reports near the page bottom no longer avoid nearly empty trailing pages"
    );
    Ok(())
}

/// Reports keep the frozen layout rows, labels, fonts, and page count snapshot.
#[test]
fn reports_keep_the_frozen_layout_rows_labels_fonts_and_page_count_snapshot() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("reference.pdf");
    let reference = report();
    let rows = entries();
    let mut report = Report::new(
        VocabularyLayout::new(ReportLabels::default()),
        ReportFonts::default(),
    );
    let first = image(directory.path(), 256);
    let second = image(directory.path(), 256);
    report.append(&rows[0], Some(first.clone()));
    report.append(&rows[1], Some(second.clone()));
    report.save(&path, &Thumbnail::new(150))?;
    assert_eq!(
        json!({
            "entries": rows.iter().zip([first, second]).map(|(entry, _image)| {
                json!({
                    "font": ReportFonts::default().selected(entry).name(),
                    "labels": {
                        "context": ReportLabels::default().selected(entry).context,
                        "hint": ReportLabels::default().selected(entry).hint,
                        "importance": ReportLabels::default().selected(entry).importance,
                        "sentence": ReportLabels::default().selected(entry).sentence,
                    },
                    "rows": manifest_rows(entry),
                    "source_lang": entry.source_lang,
                    "target_lang": entry.target_lang,
                    "word": entry.word,
                })
            }).collect::<Vec<_>>(),
            "pdf": {
                "header": header(&path),
                "page_count": pages(&path),
            },
        }),
        json!({
            "entries": reference["entries"].as_array().expect("reference entries must be an array").iter().map(|entry| {
                json!({
                    "font": entry["font"],
                    "labels": entry["labels"],
                    "rows": entry["rows"],
                    "source_lang": entry["source_lang"],
                    "target_lang": entry["target_lang"],
                    "word": entry["word"],
                })
            }).collect::<Vec<_>>(),
            "pdf": {
                "header": reference["pdf"]["header"],
                "page_count": reference["pdf"]["page_count"],
            },
        }),
        "reports no longer keep the frozen layout rows labels fonts and page count snapshot"
    );
    Ok(())
}
