use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Line, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt,
    RawImage, RawImageData, RawImageFormat, TextItem, XObjectTransform,
};

use crate::vocabulary::VocabularyEntry;

use super::FontPalette;
use super::font::{carries, leading, parsed, rgb, target, wrap};
use super::{ReportLayout, Thumbnail};

const GAP: f32 = 1.0;
const HEIGHT: f32 = 297.0;
const IMAGE: f32 = 25.0;
const INDENT: f32 = 40.0;
const LIMIT: f32 = 240.0;
const MARGIN: f32 = 10.0;
const WIDTH: f32 = 210.0 - INDENT - MARGIN;

/// Accumulate report rows and render them into one PDF.
#[derive(Clone, Debug)]
pub struct Report<L> {
    layout: L,
    palette: FontPalette,
    rows: Vec<(VocabularyEntry, Option<PathBuf>)>,
}

impl<L> Report<L> {
    /// Create one empty report using the default font palette.
    pub fn new(layout: L) -> Self {
        Self {
            layout,
            palette: FontPalette::default(),
            rows: Vec::new(),
        }
    }

    /// Override the font palette — used by tests that pin a specific family.
    #[must_use]
    pub fn with_palette(mut self, palette: FontPalette) -> Self {
        self.palette = palette;
        self
    }

    /// Append one entry and optional image path to the report.
    pub fn append(&mut self, entry: &VocabularyEntry, image: Option<PathBuf>) {
        self.rows.push((entry.clone(), image));
    }
}

impl<L> Report<L>
where
    L: ReportLayout,
{
    /// Save the accumulated report to one PDF file.
    pub fn save(&self, output: impl AsRef<Path>, thumbnail: &Thumbnail) -> Result<()> {
        if let Some(parent) = output.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let prepared = self.prepared_fonts()?;
        let mut doc = PdfDocument::new("Kamishibai Report");
        let registered = prepared.register(&mut doc);
        let mut pages = vec![Vec::new()];
        let mut y = 10.0f32;
        for (entry, image) in &self.rows {
            if y > LIMIT {
                pages.push(Vec::new());
                y = 10.0;
            }
            let ops = pages.last_mut().expect("report must keep one active page");
            self.row(
                &mut doc,
                ops,
                &prepared,
                &registered,
                entry,
                image.as_deref(),
                thumbnail,
                &mut y,
            )?;
        }
        let pdf = doc
            .with_pages(
                pages
                    .into_iter()
                    .map(|ops| PdfPage::new(Mm(210.0), Mm(297.0), ops))
                    .collect(),
            )
            .save(&PdfSaveOptions::default(), &mut Vec::new());
        fs::write(output, pdf)?;
        Ok(())
    }

    /// Pre-subset every font track to the actual characters its rows assign
    /// to it. Without subsetting, printpdf 0.9.1 embeds the full font (its
    /// own subsetting is hard-disabled by an `if false &&` guard in
    /// serialize.rs:1162), which on macOS would inflate the PDF since one of
    /// the tracks is the 23 MB Arial Unicode MS file.
    ///
    /// Each character is routed at the current weight: primary current-weight
    /// → CJK current-weight → fallback (Arial Unicode MS regular). The
    /// fallback catches the holes in primary bold (IPA stress marks, etc.)
    /// the prepare and render passes use the same dispatch so subsets are
    /// always sufficient.
    fn prepared_fonts(&self) -> Result<PaletteFonts> {
        let primary_regular_full = Rc::new(parsed(&self.palette.primary().regular()?)?);
        let primary_bold_full = Rc::new(parsed(&self.palette.primary().bold()?)?);
        let cjk_regular_full = Rc::new(parsed(&self.palette.cjk().regular()?)?);
        let cjk_bold_full = Rc::new(parsed(&self.palette.cjk().bold()?)?);
        let fallback_full = Rc::new(parsed(&self.palette.fallback().regular()?)?);
        let mut buckets = CharBuckets::default();
        for (entry, _) in &self.rows {
            for (index, (line, _)) in self.layout.row(entry).into_iter().enumerate() {
                let bold = index == 0;
                for ch in line.chars() {
                    let track = dispatch(
                        ch,
                        bold,
                        &primary_regular_full,
                        &primary_bold_full,
                        &cjk_regular_full,
                        &cjk_bold_full,
                        &fallback_full,
                    );
                    buckets.insert(track, bold, ch);
                }
            }
        }
        if buckets.primary_regular.is_empty() {
            buckets.primary_regular.insert(' ');
        }
        if buckets.primary_bold.is_empty() {
            buckets.primary_bold.insert(' ');
        }
        Ok(PaletteFonts {
            primary_regular: Rc::new(subset_or_full(
                &primary_regular_full,
                &buckets.primary_regular,
            )),
            primary_bold: Rc::new(subset_or_full(&primary_bold_full, &buckets.primary_bold)),
            cjk_regular: (!buckets.cjk_regular.is_empty())
                .then(|| Rc::new(subset_or_full(&cjk_regular_full, &buckets.cjk_regular))),
            cjk_bold: (!buckets.cjk_bold.is_empty())
                .then(|| Rc::new(subset_or_full(&cjk_bold_full, &buckets.cjk_bold))),
            fallback: (!buckets.fallback.is_empty())
                .then(|| Rc::new(subset_or_full(&fallback_full, &buckets.fallback))),
            classifier_primary_regular: primary_regular_full,
            classifier_primary_bold: primary_bold_full,
            classifier_cjk_regular: cjk_regular_full,
            classifier_cjk_bold: cjk_bold_full,
            classifier_fallback: fallback_full,
        })
    }

    /// Render one entry onto the active page.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        doc: &mut PdfDocument,
        ops: &mut Vec<Op>,
        fonts: &PaletteFonts,
        ids: &PageFonts,
        entry: &VocabularyEntry,
        image: Option<&Path>,
        thumbnail: &Thumbnail,
        y: &mut f32,
    ) -> Result<()> {
        let top = *y;
        if let Some(path) = image.filter(|path| path.is_file()) {
            let image = raw(thumbnail.scaled(path)?);
            let scale_x = target(IMAGE, image.width as f32);
            let scale_y = target(IMAGE, image.height as f32);
            let id = doc.add_image(&image);
            ops.push(Op::UseXobject {
                id,
                transform: XObjectTransform {
                    translate_x: Some(Mm(10.0).into()),
                    translate_y: Some(Mm(HEIGHT - top - IMAGE).into()),
                    rotate: None,
                    scale_x: Some(scale_x),
                    scale_y: Some(scale_y),
                    dpi: Some(300.0),
                },
            });
        }
        let mut text = top + leading(11.0) * 0.75;
        for (index, (line, size)) in self.layout.row(entry).into_iter().enumerate() {
            if index > 0 {
                text += GAP;
            }
            let bold = index == 0;
            let primary_for_wrap = if bold {
                fonts.classifier_primary_bold.as_ref()
            } else {
                fonts.classifier_primary_regular.as_ref()
            };
            let cjk_for_wrap = if bold {
                fonts.classifier_cjk_bold.as_ref()
            } else {
                fonts.classifier_cjk_regular.as_ref()
            };
            let fallback_for_wrap = fonts.classifier_fallback.as_ref();
            for part in wrap(
                line.as_str(),
                size,
                WIDTH,
                primary_for_wrap,
                cjk_for_wrap,
                fallback_for_wrap,
            ) {
                let color = if bold {
                    rgb(0, 0, 0)
                } else if size <= 8.0 {
                    rgb(120, 120, 120)
                } else {
                    rgb(0, 0, 0)
                };
                self.line(ops, fonts, ids, part.as_str(), size, bold, color, text);
                text += leading(size);
            }
        }
        *y = text.max(top + IMAGE) + 7.0;
        ops.push(Op::SetOutlineColor {
            col: rgb(200, 200, 200),
        });
        ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    printpdf::LinePoint {
                        p: Point::new(Mm(10.0), Mm(HEIGHT - *y + 4.0)),
                        bezier: false,
                    },
                    printpdf::LinePoint {
                        p: Point::new(Mm(200.0), Mm(HEIGHT - *y + 4.0)),
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
        Ok(())
    }

    /// Write one wrapped text line, splitting into runs whenever the active
    /// track changes (primary → CJK → fallback) so every glyph reaches the
    /// font that actually carries it.
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        ops: &mut Vec<Op>,
        fonts: &PaletteFonts,
        ids: &PageFonts,
        line: &str,
        size: f32,
        bold: bool,
        color: printpdf::Color,
        y: f32,
    ) {
        if line.is_empty() {
            return;
        }
        ops.push(Op::StartTextSection);
        ops.push(Op::SetTextCursor {
            pos: Point::new(Mm(INDENT), Mm(HEIGHT - y)),
        });
        ops.push(Op::SetLineHeight { lh: Pt(size) });
        ops.push(Op::SetFillColor { col: color });
        let mut current = String::new();
        let mut current_track = Track::Primary;
        let mut started = false;
        for ch in line.chars() {
            let track = dispatch(
                ch,
                bold,
                fonts.classifier_primary_regular.as_ref(),
                fonts.classifier_primary_bold.as_ref(),
                fonts.classifier_cjk_regular.as_ref(),
                fonts.classifier_cjk_bold.as_ref(),
                fonts.classifier_fallback.as_ref(),
            );
            if started && track != current_track {
                emit(ops, ids, &current, current_track, bold, size);
                current.clear();
            }
            current.push(ch);
            current_track = track;
            started = true;
        }
        if !current.is_empty() {
            emit(ops, ids, &current, current_track, bold, size);
        }
        ops.push(Op::EndTextSection);
    }
}

fn emit(ops: &mut Vec<Op>, ids: &PageFonts, text: &str, track: Track, bold: bool, size: f32) {
    let id = match (track, bold) {
        (Track::Primary, true) => ids.primary_bold.clone(),
        (Track::Primary, false) => ids.primary_regular.clone(),
        (Track::Cjk, true) => ids
            .cjk_bold
            .clone()
            .unwrap_or_else(|| ids.primary_bold.clone()),
        (Track::Cjk, false) => ids
            .cjk_regular
            .clone()
            .unwrap_or_else(|| ids.primary_regular.clone()),
        (Track::Fallback, _) => ids
            .fallback
            .clone()
            .unwrap_or_else(|| ids.primary_regular.clone()),
    };
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(id),
        size: Pt(size),
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(String::from(text))],
    });
}

/// Pick the track that carries the glyph at the current weight: primary →
/// CJK → fallback. Identical logic for prepare-time char bucketing and
/// render-time dispatch, so every emitted glyph lives in the matching subset.
#[allow(clippy::too_many_arguments)]
fn dispatch(
    ch: char,
    bold: bool,
    primary_regular: &ParsedFont,
    primary_bold: &ParsedFont,
    cjk_regular: &ParsedFont,
    cjk_bold: &ParsedFont,
    fallback: &ParsedFont,
) -> Track {
    let primary = if bold { primary_bold } else { primary_regular };
    if carries(primary, ch) {
        return Track::Primary;
    }
    let cjk = if bold { cjk_bold } else { cjk_regular };
    if carries(cjk, ch) {
        return Track::Cjk;
    }
    if carries(fallback, ch) {
        return Track::Fallback;
    }
    Track::Primary
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Track {
    Primary,
    Cjk,
    Fallback,
}

#[derive(Default)]
struct CharBuckets {
    primary_regular: HashSet<char>,
    primary_bold: HashSet<char>,
    cjk_regular: HashSet<char>,
    cjk_bold: HashSet<char>,
    fallback: HashSet<char>,
}

impl CharBuckets {
    fn insert(&mut self, track: Track, bold: bool, ch: char) {
        match (track, bold) {
            (Track::Primary, true) => {
                self.primary_bold.insert(ch);
            }
            (Track::Primary, false) => {
                self.primary_regular.insert(ch);
            }
            (Track::Cjk, true) => {
                self.cjk_bold.insert(ch);
            }
            (Track::Cjk, false) => {
                self.cjk_regular.insert(ch);
            }
            (Track::Fallback, _) => {
                self.fallback.insert(ch);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct PaletteFonts {
    primary_regular: Rc<ParsedFont>,
    primary_bold: Rc<ParsedFont>,
    cjk_regular: Option<Rc<ParsedFont>>,
    cjk_bold: Option<Rc<ParsedFont>>,
    fallback: Option<Rc<ParsedFont>>,
    classifier_primary_regular: Rc<ParsedFont>,
    classifier_primary_bold: Rc<ParsedFont>,
    classifier_cjk_regular: Rc<ParsedFont>,
    classifier_cjk_bold: Rc<ParsedFont>,
    classifier_fallback: Rc<ParsedFont>,
}

impl PaletteFonts {
    /// Register every embedded track with the document and return the font
    /// id table. CJK and fallback are registered only if at least one glyph
    /// routed to them.
    fn register(&self, doc: &mut PdfDocument) -> PageFonts {
        PageFonts {
            primary_regular: doc.add_font(&self.primary_regular),
            primary_bold: doc.add_font(&self.primary_bold),
            cjk_regular: self
                .cjk_regular
                .as_ref()
                .map(|font| doc.add_font(font.as_ref())),
            cjk_bold: self
                .cjk_bold
                .as_ref()
                .map(|font| doc.add_font(font.as_ref())),
            fallback: self
                .fallback
                .as_ref()
                .map(|font| doc.add_font(font.as_ref())),
        }
    }
}

#[derive(Clone, Debug)]
struct PageFonts {
    primary_regular: printpdf::FontId,
    primary_bold: printpdf::FontId,
    cjk_regular: Option<printpdf::FontId>,
    cjk_bold: Option<printpdf::FontId>,
    fallback: Option<printpdf::FontId>,
}

fn raw(image: DynamicImage) -> RawImage {
    let (width, height) = image.dimensions();
    let data = image.to_rgb8();
    RawImage {
        pixels: RawImageData::U8(data.into_raw()),
        width: width as usize,
        height: height as usize,
        data_format: RawImageFormat::RGB8,
        tag: Vec::new(),
    }
}

/// Subset one font down to the supplied character set; falls back to the
/// full font when allsorts cannot subset it (rare but happens for some CFF
/// fonts).
fn subset_or_full(font: &ParsedFont, chars: &HashSet<char>) -> ParsedFont {
    let mut glyph_ids: std::collections::BTreeMap<u16, char> = std::collections::BTreeMap::new();
    glyph_ids.insert(0, '\0');
    for ch in chars {
        if let Some(gid) = font.lookup_glyph_index(*ch as u32) {
            glyph_ids.insert(gid, *ch);
        }
    }
    let Ok(subset) = printpdf::subset_font(font, &glyph_ids) else {
        return font.clone();
    };
    let mut warnings = Vec::new();
    ParsedFont::from_bytes(&subset.bytes, 0, &mut warnings).unwrap_or_else(|| font.clone())
}
