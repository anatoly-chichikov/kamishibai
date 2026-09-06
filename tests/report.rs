//! Tests for PDF report rendering.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use image::{Rgb, RgbImage};
use kamishibai::languages::ReportLabels;
use kamishibai::report::{
    CardSheet, FontFamily, FontPath, Report, ReportLayout, Thumbnail, VocabularyLayout,
};
use kamishibai::vocabulary::{
    Importance, LanguageCode, NonEmptyText, VocabularyDocument, VocabularyEntry, VocabularySource,
    VocabularyTarget,
};
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

/// Create one strict entry for report tests.
fn entry(word: &str, source: &str, target: &str) -> VocabularyEntry {
    VocabularyEntry {
        term: text(word),
        meaning: text("значение"),
        pronunciation: text("ˈprimer"),
        transcription: text(word),
        importance: score(5),
        source: VocabularySource {
            sentence: text("пример"),
            lang: code(source),
            highlight: text("пример"),
            hint: text("подсказка"),
            context: text("контекст"),
        },
        target: VocabularyTarget {
            sentence: text("translation example"),
            lang: code(target),
        },
    }
}

/// Create one strict multilingual entry whose fields exercise a script's
/// shaping, combining marks, or font coverage.
fn source_scripted_entry(fields: [&str; 8]) -> VocabularyEntry {
    let [
        highlight,
        term,
        source_sentence,
        source_hint,
        source_context,
        source,
        target_sentence,
        target,
    ] = fields;
    VocabularyEntry {
        term: text(term),
        meaning: text(highlight),
        pronunciation: text("/proof/"),
        transcription: text(term),
        importance: score(5),
        source: VocabularySource {
            sentence: text(source_sentence),
            lang: code(source),
            highlight: text(highlight),
            hint: text(source_hint),
            context: text(source_context),
        },
        target: VocabularyTarget {
            sentence: text(target_sentence),
            lang: code(target),
        },
    }
}

/// Create a role-correct reversed entry so each complex script is exercised
/// as target sentence and lemma rather than only as source context.
fn reversed_scripted_entry(fields: [&str; 8]) -> VocabularyEntry {
    let [
        term,
        meaning,
        source_sentence,
        source_hint,
        source_context,
        target,
        target_sentence,
        source,
    ] = fields;
    VocabularyEntry {
        term: text(term),
        meaning: text(meaning),
        pronunciation: text("/proof/"),
        transcription: text(term),
        importance: score(5),
        source: VocabularySource {
            sentence: text(source_sentence),
            lang: code(source),
            highlight: text(meaning),
            hint: text(source_hint),
            context: text(source_context),
        },
        target: VocabularyTarget {
            sentence: text(target_sentence),
            lang: code(target),
        },
    }
}

/// Return one validated text fixture.
fn text(value: &str) -> NonEmptyText {
    NonEmptyText::new(value).expect("test text must be valid")
}

/// Return one validated language fixture.
fn code(value: &str) -> LanguageCode {
    LanguageCode::new(value).expect("test language must be valid")
}

/// Return one validated importance fixture.
fn score(value: u8) -> Importance {
    Importance::new(value).expect("test importance must be valid")
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

/// Return the frozen reference entries.
fn entries() -> Vec<VocabularyEntry> {
    VocabularyDocument::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("reference")
            .join("inputs")
            .join("mixed-target-deck.json"),
    )
    .expect("reference entries must parse")
    .entries
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
        term: text("café"),
        meaning: text("кафе"),
        pronunciation: text("ˈkafe"),
        transcription: text("cafe"),
        importance: score(7),
        source: VocabularySource {
            sentence: text("В кафе подают крепы"),
            lang: code("en"),
            highlight: text("кафе"),
            hint: text("coast"),
            context: text("calm evening"),
        },
        target: VocabularyTarget {
            sentence: text("The café serves crêpes"),
            lang: code("ru"),
        },
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

/// Vocabulary layout keeps the strict row structure for placeholder entries.
#[test]
fn vocabulary_layout_keeps_the_strict_placeholder_row_structure() {
    let item = entry("café", "en", "ru");
    assert_eq!(
        VocabularyLayout::new(ReportLabels::default()).row(&item),
        vec![
            (String::from("café /ˈprimer/ — значение"), 11.0),
            (String::from("translation example"), 9.0),
            (String::from("Translation: пример"), 9.0),
            (String::from("Context: контекст"), 8.0),
            (String::from("Hint: подсказка"), 8.0),
            (String::from("Importance: 5/10"), 8.0),
        ],
        "vocabulary layout no longer keeps the strict placeholder row structure"
    );
}

/// Font path resolution finds the configured system font.
#[test]
fn font_path_resolution_finds_the_configured_system_font() -> Result<()> {
    assert!(
        FontPath::new("Arial").resolved()?.path.is_file(),
        "font path resolution no longer finds the configured system font"
    );
    Ok(())
}

/// Font family resolution gives regular and bold distinct file paths.
#[test]
fn font_family_resolution_gives_regular_and_bold_distinct_paths() -> Result<()> {
    let family = FontFamily::new("Arial");
    let regular = family.regular()?;
    let bold = family.bold()?;
    assert!(
        regular.path.is_file() && bold.path.is_file() && regular.path != bold.path,
        "font family resolution no longer differentiates regular and bold faces"
    );
    Ok(())
}

/// Thumbnail scaling clamps the longest side to the configured pixel budget.
#[test]
fn thumbnail_scaling_clamps_the_longest_side_to_the_configured_pixel_budget() -> Result<()> {
    let directory = TempDir::new()?;
    let source = image(directory.path(), 600);
    let scaled = Thumbnail::new(150).scaled(&source)?;
    assert_eq!(
        (scaled.width(), scaled.height()),
        (150, 150),
        "thumbnail scaling no longer clamps the longest side to the configured pixel budget"
    );
    Ok(())
}

/// Reports with no entries still produce a nonempty PDF.
#[test]
fn reports_with_no_entries_still_produce_a_nonempty_pdf() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("empty.pdf");
    Report::new(StaticLayout { rows: vec![] }).save(&path, &Thumbnail::new(150))?;
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
    let mut report = Report::new(StaticLayout {
        rows: vec![
            (String::from("Ünïcödé línë"), 10.0),
            (String::from("wörd"), 14.0),
        ],
    });
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
    let mut report = Report::new(StaticLayout {
        rows: vec![
            (String::from("Кириллица проверка"), 10.0),
            (String::from("Ελληνικά δοκιμή"), 14.0),
            (String::from("Mixed Ünïcödé ñ ü ö"), 9.0),
        ],
    });
    report.append(&entry("Ελληνικά", "ru", "el"), None);
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports no longer render mixed script text without failing"
    );
    Ok(())
}

/// Reports route CJK glyphs to the CJK fallback while non-CJK glyphs stay
/// on the embedded primary, on the same line and the same entry.
#[test]
fn reports_route_cjk_glyphs_to_the_cjk_fallback() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("fonts.pdf");
    let mut report = Report::new(StaticLayout {
        rows: vec![(String::from("hello 朋友 world"), 10.0)],
    });
    report.append(&entry("plain", "en", "en"), None);
    report.append(&entry("朋友", "el", "zh"), None);
    report.save(&path, &Thumbnail::new(150))?;
    assert!(
        bytes(&path) > 0,
        "reports no longer route CJK glyphs to the CJK fallback"
    );
    Ok(())
}

/// Reports wrap long text without failing.
#[test]
fn reports_wrap_long_text_without_failing() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("wrap.pdf");
    let paragraph = "Ünïcödé ".repeat(80);
    let mut report = Report::new(StaticLayout {
        rows: vec![(paragraph.clone(), 10.0)],
    });
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
    let mut report = Report::new(StaticLayout {
        rows: vec![
            (String::from("Строка öднä"), 11.0),
            (String::from("Пример prédlözhéniÿa"), 9.0),
        ],
    });
    for index in 0..30 {
        report.append(
            &VocabularyEntry {
                term: text(format!("wörd{index}").as_str()),
                meaning: text("значение"),
                pronunciation: text("vɜːd"),
                transcription: text("word"),
                importance: score(5),
                source: VocabularySource {
                    sentence: text(format!("Ünïcödé {index}").as_str()),
                    lang: code("en"),
                    highlight: text(format!("Ünïcödé {index}").as_str()),
                    hint: text("подсказка"),
                    context: text("контекст"),
                },
                target: VocabularyTarget {
                    sentence: text("Translation sample"),
                    lang: code("ru"),
                },
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
    let mut report = Report::new(StaticLayout {
        rows: vec![(String::from("Wörd"), 11.0), ("À".repeat(1000), 9.0)],
    });
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

/// Card sheets fit four duplex cards onto each printable A4 page.
#[test]
fn card_sheets_fit_four_duplex_cards_onto_each_printable_a4_page() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards.pdf");
    let mut sheet = CardSheet::new();
    for _ in 0..9 {
        sheet.append(
            &entry("idiom", "ru", "en"),
            Some(image(directory.path(), 256)),
        );
    }
    sheet.save(&path, &Thumbnail::new(256))?;
    assert_eq!(
        pages(&path),
        3,
        "card sheets no longer fit four duplex cards onto each printable A4 page"
    );
    Ok(())
}

/// Card sheets save a realistic dense multi-sense card after its complete
/// back-face layout has passed the lower-level content-preservation check.
#[test]
fn card_sheets_save_a_dense_multi_sense_context() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards-six-senses.pdf");
    let mut item = entry("bank", "en", "ru");
    item.target.sentence =
        text("The pilot banked the aircraft smoothly while approaching the distant runway marker");
    item.meaning =
        text("to tilt an aircraft sideways during a turn by lifting one wing above the other");
    item.source.context = text(
        "**Meaning.**\n- **a financial institution that safeguards deposits and lends money to people and companies [finance]**\n- the sloping ground immediately beside a river, lake, canal, or similar body of water [landform]\n- a stored reserve of blood, data, food, or other resources for future use [reserve]\n- a long row or tier of matching objects arranged closely beside one another [row]\n- to rely confidently on a person, promise, event, or expected future result [rely]\n- to tilt an aircraft sideways while turning by raising one wing above another [aviation]\nThe noun senses grew from ideas of an edge or accumulated mass; the verbs developed separately.\n\n**Where you'll hear it.** Common in finance, geography, computing, medicine, aviation, and dependent plans.\n\n**Where it's out of place.** Do not use the finance sense for a wallet, safe, or ordinary container.\n\n**Subtlety.** Bank on takes a person or outcome; bank the aircraft names deliberate sideways tilt - final context anchor.",
    );
    let mut sheet = CardSheet::new();
    sheet.append(&item, None);
    sheet.save(&path, &Thumbnail::new(256))?;
    assert_eq!(
        (path.exists(), pages(&path)),
        (true, 1),
        "the completely planned dense multi-sense card did not save as one printable page"
    );
    Ok(())
}

/// Fixed-size card sheets refuse unbounded imported prose without producing
/// oversized cards or a successful PDF with a clipped tail.
#[test]
fn card_sheets_refuse_unbounded_explanations_without_changing_card_size() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards-overflow.pdf");
    let mut item = entry("bank", "en", "ru");
    let context = format!(
        "**Meaning.**\n- {}\n\n**Subtlety.** final-overflow-anchor",
        "unbounded context ".repeat(1600)
    );
    item.source.context = text(context.as_str());
    let mut sheet = CardSheet::new();
    sheet.append(&item, None);
    let result = sheet.save(&path, &Thumbnail::new(256));
    assert_eq!(
        (result.is_err(), path.exists()),
        (true, false),
        "unbounded prose produced oversized cards or a clipped PDF instead of refusing"
    );
    Ok(())
}

/// Several dense cards retain the original four-card sheet geometry.
#[test]
fn card_sheets_pack_several_dense_cards_at_the_original_size() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards-dense.pdf");
    let mut item = entry("good for you", "ru", "en");
    item.source.context = text(&format!(
        "**Meaning.** A useful expression.\n\n**Subtlety.** {}",
        "Precise usage guidance belongs to this meaning. ".repeat(18)
    ));
    let mut sheet = CardSheet::new();
    for _ in 0..3 {
        sheet.append(&item, None);
    }
    sheet.save(&path, &Thumbnail::new(256))?;
    assert_eq!(
        pages(&path),
        1,
        "three dense foldcards no longer share one original-size sheet"
    );
    Ok(())
}

/// Card sheets render mixed-script content without panicking.
#[test]
fn card_sheets_render_mixed_script_content_without_panicking() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards-mixed.pdf");
    let mut sheet = CardSheet::new();
    sheet.append(&entry("光", "ru", "zh"), Some(image(directory.path(), 256)));
    sheet.append(&entry("Ελληνικά", "ru", "el"), None);
    sheet.save(&path, &Thumbnail::new(256))?;
    assert!(
        bytes(&path) > 1000,
        "card sheets no longer render mixed-script content without panicking"
    );
    Ok(())
}

/// Both PDF surfaces must expose CFF outlines through their actual character ids.
#[cfg(target_os = "macos")]
#[test]
fn cjk_bold_fonts_cannot_be_published_as_truetype_outlines() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = [
        directory.path().join("cjk-card.pdf"),
        directory.path().join("cjk-report.pdf"),
    ];
    let mut item = entry("cast", "ja", "en");
    item.source.context = text(
        "**意味**\n- **出演する俳優たち**\n- 投げること\n\n**使う場面**\n映画の話。\n\n**不適切な場面**\n制限はない。\n\n**注意点**\n文脈で区別する。",
    );
    let mut sheet = CardSheet::new();
    sheet.append(&item, None);
    sheet.save(&paths[0], &Thumbnail::new(150))?;
    let mut report = Report::new(StaticLayout {
        rows: vec![(String::from("意味 出演する俳優たち"), 10.0)],
    });
    report.append(&item, None);
    report.save(&paths[1], &Thumbnail::new(150))?;
    let compatible = paths
        .iter()
        .map(|path| cff_fonts_are_compatible(path))
        .collect::<Result<Vec<_>>>()?;
    assert!(
        compatible.into_iter().all(|valid| valid),
        "a bold CJK font has incorrect outline declarations, glyph addressing or widths"
    );
    Ok(())
}

/// Inspect saved font streams rather than trusting parser coverage or nonempty PDF bytes.
#[cfg(target_os = "macos")]
fn cff_fonts_are_compatible(path: &Path) -> Result<bool> {
    let document = Document::load(path)?;
    let mut inspected = 0;
    for object in document.objects.values() {
        let Ok(font) = object.as_dict() else {
            continue;
        };
        if !font
            .get(b"Subtype")
            .and_then(lopdf::Object::as_name)
            .is_ok_and(|name| name == b"Type0")
        {
            continue;
        }
        for child in font.get(b"DescendantFonts")?.as_array()? {
            let descendant = document.dereference(child)?.1.as_dict()?;
            let descriptor = descendant
                .get_deref(b"FontDescriptor", &document)?
                .as_dict()?;
            let stream = descriptor
                .get_deref(b"FontFile3", &document)
                .or_else(|_| descriptor.get_deref(b"FontFile2", &document))?
                .as_stream()?;
            let bytes = stream.get_plain_content()?;
            if bytes.starts_with(b"OTTO") {
                return Ok(false);
            }
            let Some(cff) = rustybuzz::ttf_parser::cff::Table::parse(&bytes) else {
                continue;
            };
            inspected += 1;
            if descendant.get(b"Subtype")?.as_name()? != b"CIDFontType0"
                || !descriptor.has(b"FontFile3")
                || !stream
                    .dict
                    .get(b"Subtype")
                    .and_then(lopdf::Object::as_name)
                    .is_ok_and(|name| name == b"CIDFontType0C")
            {
                return Ok(false);
            }
            let encoding = font.get_deref(b"Encoding", &document)?.as_stream()?;
            let mapping = String::from_utf8(encoding.get_plain_content()?)?;
            let collection = descendant.get(b"CIDSystemInfo")?.as_dict()?;
            if encoding.dict.get(b"CIDSystemInfo")?.as_dict()? != collection {
                return Ok(false);
            }
            let widths = descendant.get(b"W")?.as_array()?;
            for glyph in 0..cff.number_of_glyphs() {
                let cid = cff
                    .glyph_cid(rustybuzz::ttf_parser::GlyphId(glyph))
                    .expect("CFF glyph must have a CID");
                if !mapping.contains(&format!("<{glyph:04X}> {cid}\n"))
                    || glyph != 0
                        && !widths
                            .chunks_exact(2)
                            .any(|pair| pair[0].as_i64().is_ok_and(|value| value == i64::from(cid)))
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(inspected > 0)
}

/// The production card-sheet boundary generates one parseable PDF containing
/// every newly introduced complex-script font and both bidi directions.
#[test]
fn card_sheet_pdf_proves_rtl_and_complex_script_rendering() -> Result<()> {
    let directory = TempDir::new()?;
    let path = directory.path().join("cards-complex-scripts.pdf");
    let mut sheet = CardSheet::new();
    for item in [
        source_scripted_entry([
            "سلام",
            "hello",
            "سلام 2026، يا صديقي",
            "تحية ودية",
            "عبارة عربية قصيرة",
            "ar",
            "Hello, my friend",
            "en",
        ]),
        reversed_scripted_entry([
            "سلام",
            "hello",
            "I said hello to my friend",
            "a friendly greeting",
            "An Arabic greeting in the target sentence",
            "ar",
            "قلت سلام لصديقي",
            "en",
        ]),
        reversed_scripted_entry([
            "שלום",
            "hello",
            "I said hello to my friend",
            "a friendly greeting",
            "A Hebrew greeting in the target sentence",
            "he",
            "אמרתי שלום לחבר שלי",
            "en",
        ]),
        reversed_scripted_entry([
            "किताब",
            "book",
            "This book is very good",
            "something to read",
            "A Hindi noun in the target sentence",
            "hi",
            "यह किताब बहुत अच्छी है",
            "en",
        ]),
        reversed_scripted_entry([
            "เก่ง",
            "skilled",
            "They are very skilled",
            "having strong ability",
            "A Thai adjective in the target sentence",
            "th",
            "เขาเป็นคนเก่งมาก",
            "en",
        ]),
        reversed_scripted_entry([
            "친구",
            "friend",
            "I met a good friend",
            "a close person",
            "A Korean noun in the target sentence",
            "ko",
            "좋은 친구를 만났어요",
            "en",
        ]),
        source_scripted_entry([
            "שלום",
            "hello",
            "שלום 2026 חבר",
            "ברכה ידידותית",
            "משפט קצר בעברית",
            "he",
            "Hello, friend",
            "en",
        ]),
        source_scripted_entry([
            "किताब",
            "book",
            "यह एक अच्छी किताब है",
            "पढ़ने की चीज़",
            "देवनागरी मात्रा और संयुक्त अक्षर",
            "hi",
            "This is a good book",
            "en",
        ]),
        source_scripted_entry([
            "เก่ง",
            "skilled",
            "เขาเป็นคนเก่งมาก",
            "มีความสามารถ",
            "ภาษาไทยมีสระและวรรณยุกต์",
            "th",
            "They are very skilled",
            "en",
        ]),
        source_scripted_entry([
            "친구",
            "friend",
            "좋은 친구를 만났어요",
            "가까운 사람",
            "한국어 글꼴 적용 확인",
            "ko",
            "I met a good friend",
            "en",
        ]),
    ] {
        sheet.append(&item, None);
    }
    sheet.save(&path, &Thumbnail::new(256))?;
    assert!(
        pages(&path) == 3 && bytes(&path) > 50_000,
        "the complex-script card sheet was not emitted as a substantial parseable PDF"
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
    let mut report = Report::new(VocabularyLayout::new(ReportLabels::default()));
    let first = image(directory.path(), 256);
    let second = image(directory.path(), 256);
    report.append(&rows[0], Some(first.clone()));
    report.append(&rows[1], Some(second.clone()));
    report.save(&path, &Thumbnail::new(150))?;
    assert_eq!(
        json!({
            "entries": rows.iter().zip([first, second]).map(|(entry, _image)| {
                json!({
                    "labels": {
                        "context": ReportLabels::default().selected(entry).context,
                        "hint": ReportLabels::default().selected(entry).hint,
                        "importance": ReportLabels::default().selected(entry).importance,
                        "sentence": ReportLabels::default().selected(entry).sentence,
                    },
                    "rows": manifest_rows(entry),
                    "source_lang": entry.source.lang.as_str(),
                    "target_lang": entry.target.lang.as_str(),
                    "word": entry.term.as_str(),
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
