use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Codepoint, FontId, Line, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, RawImage, RawImageData, RawImageFormat, TextItem, TextMatrix,
    XObjectTransform,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::languages::TextDirection;
use crate::vocabulary::VocabularyEntry;

use super::FontPalette;
use super::font::{carries, font_arc, leading, rgb, shaping_font_arc, target};
use super::shaping::{
    inferred_direction, shape, shape_with_context, supplemental_slot, visual_runs,
};
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
        let scaled = scale_thumbnails(self.rows.as_slice(), thumbnail)?;
        let mut pages = vec![Vec::new()];
        let mut y = 10.0f32;
        for ((entry, _), image) in self.rows.iter().zip(scaled) {
            if y > LIMIT {
                pages.push(Vec::new());
                y = 10.0;
            }
            let ops = pages.last_mut().expect("report must keep one active page");
            self.row(&mut doc, ops, &prepared, &registered, entry, image, &mut y)?;
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
        let parsed = parse_palette_parallel(&self.palette)?;
        let mut buckets = CharBuckets::new(parsed.supplemental_regular.len());
        let view = ClassifierView::from(&parsed);
        for (entry, _) in &self.rows {
            for (index, (line, _)) in self.layout.row(entry).into_iter().enumerate() {
                let bold = index == 0;
                for ch in line.chars() {
                    let track = view.track(ch, bold);
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
        let shaping_active = buckets.shaping;
        Ok(PaletteFonts {
            primary_regular: embedded(&parsed.primary_regular, &buckets.primary_regular, false),
            primary_bold: embedded(&parsed.primary_bold, &buckets.primary_bold, false),
            cjk_regular: (!buckets.cjk_regular.is_empty())
                .then(|| embedded(&parsed.cjk_regular, &buckets.cjk_regular, false)),
            cjk_bold: (!buckets.cjk_bold.is_empty())
                .then(|| embedded(&parsed.cjk_bold, &buckets.cjk_bold, false)),
            supplemental_regular: parsed
                .supplemental_regular
                .iter()
                .zip(&buckets.supplemental_regular)
                .map(|(font, bucket)| (!bucket.is_empty()).then(|| font.clone()))
                .collect(),
            supplemental_bold: parsed
                .supplemental_bold
                .iter()
                .zip(&buckets.supplemental_bold)
                .map(|(font, bucket)| (!bucket.is_empty()).then(|| font.clone()))
                .collect(),
            fallback: (!buckets.fallback.is_empty())
                .then(|| embedded(&parsed.fallback, &buckets.fallback, false)),
            classifier_primary_regular: parsed.primary_regular,
            classifier_primary_bold: parsed.primary_bold,
            classifier_cjk_regular: parsed.cjk_regular,
            classifier_cjk_bold: parsed.cjk_bold,
            classifier_supplemental_regular: parsed.supplemental_regular,
            classifier_supplemental_bold: parsed.supplemental_bold,
            classifier_fallback: parsed.fallback,
            shaping_active,
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
        scaled: Option<DynamicImage>,
        y: &mut f32,
    ) -> Result<()> {
        let top = *y;
        if let Some(scaled) = scaled {
            let image = raw(scaled);
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
            let view = ClassifierView::from(fonts);
            for part in wrap_line(line.as_str(), size, WIDTH, bold, view) {
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

    /// Write one wrapped text line in bidi visual order with OpenType shaping
    /// whenever a complex-script face is active.
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
        let view = ClassifierView::from(fonts);
        let direction = inferred_direction(line);
        let spans = visual_spans(line, direction, bold, view);
        let line_width = spans
            .iter()
            .map(|span| {
                if fonts.shaping_active && matches!(span.track, Track::Supplemental(_)) {
                    return shaped_span_width(view.font(span.track, bold), span, size);
                }
                view.measure(span.text.as_str(), bold, size)
            })
            .sum::<f32>();
        let mut x = if direction == TextDirection::Rtl {
            INDENT + (WIDTH - line_width).max(0.0)
        } else {
            INDENT
        };
        for span in spans {
            let id = ids.id(span.track, bold);
            let font = view.font(span.track, bold);
            let advance = if fonts.shaping_active && matches!(span.track, Track::Supplemental(_)) {
                emit_shaped(ops, font, id, &span, size, x, HEIGHT - y, color.clone())
            } else {
                emit_plain(
                    ops,
                    id,
                    span.text.as_str(),
                    size,
                    x,
                    HEIGHT - y,
                    color.clone(),
                );
                view.measure(span.text.as_str(), bold, size)
            };
            x += advance;
        }
    }
}

fn emit_plain(
    ops: &mut Vec<Op>,
    id: FontId,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
    color: printpdf::Color,
) {
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(x), Mm(y)),
    });
    ops.push(Op::SetFillColor { col: color });
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(id),
        size: Pt(size),
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(String::from(text))],
    });
    ops.push(Op::EndTextSection);
}

#[allow(clippy::too_many_arguments)]
fn emit_shaped(
    ops: &mut Vec<Op>,
    font: &ParsedFont,
    id: FontId,
    span: &VisualSpan,
    size: f32,
    x: f32,
    y: f32,
    color: printpdf::Color,
) -> f32 {
    let glyphs = shape_with_context(
        font,
        span.text.as_str(),
        span.rtl,
        span.pre_context.as_str(),
        span.post_context.as_str(),
    )
    .expect("invariant: a parsed PDF font must be shapeable");
    let units = f32::from(font.font_metrics.units_per_em).max(1.0);
    let scale = size / units;
    let mut cursor = 0.0_f32;
    ops.push(Op::StartTextSection);
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(id),
        size: Pt(size),
    });
    ops.push(Op::SetFillColor { col: color });
    for glyph in glyphs {
        ops.push(Op::SetTextMatrix {
            matrix: TextMatrix::Raw([
                1.0,
                0.0,
                0.0,
                1.0,
                x * 72.0 / 25.4 + (cursor + f32::from(glyph.x_offset)) * scale,
                y * 72.0 / 25.4 + f32::from(glyph.y_offset) * scale,
            ]),
        });
        ops.push(Op::ShowText {
            items: vec![TextItem::GlyphIds(vec![Codepoint {
                gid: glyph.gid,
                offset: 0.0,
                cid: glyph.cid,
            }])],
        });
        cursor += f32::from(glyph.x_advance);
    }
    ops.push(Op::EndTextSection);
    cursor * scale * 25.4 / 72.0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Track {
    Primary,
    Cjk,
    Supplemental(usize),
    Fallback,
}

struct CharBuckets {
    primary_regular: HashSet<char>,
    primary_bold: HashSet<char>,
    cjk_regular: HashSet<char>,
    cjk_bold: HashSet<char>,
    supplemental_regular: Vec<HashSet<char>>,
    supplemental_bold: Vec<HashSet<char>>,
    fallback: HashSet<char>,
    shaping: bool,
}

impl CharBuckets {
    fn new(supplemental: usize) -> Self {
        Self {
            primary_regular: HashSet::new(),
            primary_bold: HashSet::new(),
            cjk_regular: HashSet::new(),
            cjk_bold: HashSet::new(),
            supplemental_regular: vec![HashSet::new(); supplemental],
            supplemental_bold: vec![HashSet::new(); supplemental],
            fallback: HashSet::new(),
            shaping: false,
        }
    }

    fn insert(&mut self, track: Track, bold: bool, ch: char) {
        self.shaping |= supplemental_slot(ch).is_some();
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
            (Track::Supplemental(index), true) => {
                self.supplemental_bold[index].insert(ch);
            }
            (Track::Supplemental(index), false) => {
                self.supplemental_regular[index].insert(ch);
            }
            (Track::Fallback, _) => {
                self.fallback.insert(ch);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ClassifierView<'a> {
    primary_regular: &'a ParsedFont,
    primary_bold: &'a ParsedFont,
    cjk_regular: &'a ParsedFont,
    cjk_bold: &'a ParsedFont,
    supplemental_regular: &'a [Arc<ParsedFont>],
    supplemental_bold: &'a [Arc<ParsedFont>],
    fallback: &'a ParsedFont,
}

impl<'a> From<&'a ParsedPalette> for ClassifierView<'a> {
    fn from(value: &'a ParsedPalette) -> Self {
        Self {
            primary_regular: value.primary_regular.as_ref(),
            primary_bold: value.primary_bold.as_ref(),
            cjk_regular: value.cjk_regular.as_ref(),
            cjk_bold: value.cjk_bold.as_ref(),
            supplemental_regular: value.supplemental_regular.as_slice(),
            supplemental_bold: value.supplemental_bold.as_slice(),
            fallback: value.fallback.as_ref(),
        }
    }
}

impl<'a> From<&'a PaletteFonts> for ClassifierView<'a> {
    fn from(value: &'a PaletteFonts) -> Self {
        Self {
            primary_regular: value.classifier_primary_regular.as_ref(),
            primary_bold: value.classifier_primary_bold.as_ref(),
            cjk_regular: value.classifier_cjk_regular.as_ref(),
            cjk_bold: value.classifier_cjk_bold.as_ref(),
            supplemental_regular: value.classifier_supplemental_regular.as_slice(),
            supplemental_bold: value.classifier_supplemental_bold.as_slice(),
            fallback: value.classifier_fallback.as_ref(),
        }
    }
}

impl ClassifierView<'_> {
    fn track(&self, ch: char, bold: bool) -> Track {
        let supplemental = if bold {
            self.supplemental_bold
        } else {
            self.supplemental_regular
        };
        if let Some(index) = supplemental_slot(ch).filter(|index| {
            supplemental
                .get(*index)
                .is_some_and(|font| carries(font.as_ref(), ch))
        }) {
            return Track::Supplemental(index);
        }
        let primary = if bold {
            self.primary_bold
        } else {
            self.primary_regular
        };
        if carries(primary, ch) {
            return Track::Primary;
        }
        let cjk = if bold {
            self.cjk_bold
        } else {
            self.cjk_regular
        };
        if carries(cjk, ch) {
            return Track::Cjk;
        }
        if let Some(index) = supplemental
            .iter()
            .position(|font| carries(font.as_ref(), ch))
        {
            return Track::Supplemental(index);
        }
        if carries(self.fallback, ch) {
            return Track::Fallback;
        }
        Track::Primary
    }

    fn font(&self, track: Track, bold: bool) -> &ParsedFont {
        match (track, bold) {
            (Track::Primary, true) => self.primary_bold,
            (Track::Primary, false) => self.primary_regular,
            (Track::Cjk, true) => self.cjk_bold,
            (Track::Cjk, false) => self.cjk_regular,
            (Track::Supplemental(index), true) => self.supplemental_bold[index].as_ref(),
            (Track::Supplemental(index), false) => self.supplemental_regular[index].as_ref(),
            (Track::Fallback, _) => self.fallback,
        }
    }

    fn measure(&self, text: &str, bold: bool, size: f32) -> f32 {
        self.font_spans(text, bold)
            .iter()
            .map(|span| {
                let rtl = inferred_direction(span.text.as_str()) == TextDirection::Rtl;
                shaped_width(self.font(span.track, bold), span.text.as_str(), rtl, size)
            })
            .sum()
    }

    fn font_spans(&self, text: &str, bold: bool) -> Vec<FontSpan> {
        let mut spans = Vec::new();
        let mut current = String::new();
        let mut current_track = Track::Primary;
        let mut start = 0usize;
        for (index, ch) in text.char_indices() {
            let track = self.track(ch, bold);
            if !current.is_empty() && track != current_track {
                spans.push(FontSpan {
                    text: std::mem::take(&mut current),
                    track: current_track,
                    range: start..index,
                });
                start = index;
            }
            current.push(ch);
            current_track = track;
        }
        if !current.is_empty() {
            spans.push(FontSpan {
                text: current,
                track: current_track,
                range: start..text.len(),
            });
        }
        spans
    }
}

#[derive(Clone, Debug)]
struct FontSpan {
    text: String,
    track: Track,
    range: std::ops::Range<usize>,
}

#[derive(Clone, Debug)]
struct VisualSpan {
    text: String,
    pre_context: String,
    post_context: String,
    track: Track,
    rtl: bool,
}

fn visual_spans(
    text: &str,
    direction: TextDirection,
    bold: bool,
    view: ClassifierView<'_>,
) -> Vec<VisualSpan> {
    let mut spans = Vec::new();
    for run in visual_runs(text, direction) {
        let run_start = run.range.start;
        let mut directional = view
            .font_spans(&text[run.range.clone()], bold)
            .into_iter()
            .map(|span| {
                let start = run_start + span.range.start;
                let end = run_start + span.range.end;
                VisualSpan {
                    text: span.text,
                    pre_context: text[..start].to_string(),
                    post_context: text[end..].to_string(),
                    track: span.track,
                    rtl: run.rtl,
                }
            })
            .collect::<Vec<_>>();
        if run.rtl {
            directional.reverse();
        }
        spans.extend(directional);
    }
    spans
}

fn shaped_width(font: &ParsedFont, text: &str, rtl: bool, size: f32) -> f32 {
    let units = f32::from(font.font_metrics.units_per_em).max(1.0);
    let advance = shape(font, text, rtl)
        .expect("invariant: a parsed PDF font must be shapeable")
        .iter()
        .map(|glyph| f32::from(glyph.x_advance))
        .sum::<f32>();
    advance * size / units * 25.4 / 72.0
}

fn shaped_span_width(font: &ParsedFont, span: &VisualSpan, size: f32) -> f32 {
    let units = f32::from(font.font_metrics.units_per_em).max(1.0);
    let advance = shape_with_context(
        font,
        span.text.as_str(),
        span.rtl,
        span.pre_context.as_str(),
        span.post_context.as_str(),
    )
    .expect("invariant: a parsed PDF font must be shapeable")
    .iter()
    .map(|glyph| f32::from(glyph.x_advance))
    .sum::<f32>();
    advance * size / units * 25.4 / 72.0
}

fn wrap_line(
    text: &str,
    size: f32,
    width: f32,
    bold: bool,
    view: ClassifierView<'_>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for (word_index, word) in text.split_whitespace().enumerate() {
        for (piece_index, piece) in wrap_pieces(word).into_iter().enumerate() {
            let joiner = if !current.is_empty() && word_index > 0 && piece_index == 0 {
                " "
            } else {
                ""
            };
            let candidate = format!("{current}{joiner}{piece}");
            if view.measure(candidate.as_str(), bold, size) <= width {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current = piece;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_pieces(word: &str) -> Vec<String> {
    if !word.chars().any(is_unspaced) {
        return vec![word.to_string()];
    }
    word.graphemes(true).map(String::from).collect()
}

fn is_unspaced(ch: char) -> bool {
    super::font::is_cjk(ch) || matches!(u32::from(ch), 0x0E00..=0x0E7F)
}

#[derive(Clone, Debug)]
struct PaletteFonts {
    primary_regular: Arc<ParsedFont>,
    primary_bold: Arc<ParsedFont>,
    cjk_regular: Option<Arc<ParsedFont>>,
    cjk_bold: Option<Arc<ParsedFont>>,
    supplemental_regular: Vec<Option<Arc<ParsedFont>>>,
    supplemental_bold: Vec<Option<Arc<ParsedFont>>>,
    fallback: Option<Arc<ParsedFont>>,
    classifier_primary_regular: Arc<ParsedFont>,
    classifier_primary_bold: Arc<ParsedFont>,
    classifier_cjk_regular: Arc<ParsedFont>,
    classifier_cjk_bold: Arc<ParsedFont>,
    classifier_supplemental_regular: Vec<Arc<ParsedFont>>,
    classifier_supplemental_bold: Vec<Arc<ParsedFont>>,
    classifier_fallback: Arc<ParsedFont>,
    shaping_active: bool,
}

impl PaletteFonts {
    /// Register every embedded track with the document and return the font
    /// id table. CJK and fallback are registered only if at least one glyph
    /// routed to them.
    fn register(&self, doc: &mut PdfDocument) -> PageFonts {
        let supplemental_regular = self
            .supplemental_regular
            .iter()
            .map(|font| font.as_ref().map(|value| doc.add_font(value.as_ref())))
            .collect::<Vec<_>>();
        let supplemental_bold = self
            .supplemental_bold
            .iter()
            .enumerate()
            .map(|(index, font)| match font.as_ref() {
                Some(value)
                    if self.supplemental_regular[index]
                        .as_ref()
                        .is_some_and(|regular| Arc::ptr_eq(regular, value)) =>
                {
                    supplemental_regular[index].clone()
                }
                Some(value) => Some(doc.add_font(value.as_ref())),
                None => None,
            })
            .collect();
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
            supplemental_regular,
            supplemental_bold,
            fallback: self
                .fallback
                .as_ref()
                .map(|font| doc.add_font(font.as_ref())),
        }
    }
}

#[derive(Clone, Debug)]
struct PageFonts {
    primary_regular: FontId,
    primary_bold: FontId,
    cjk_regular: Option<FontId>,
    cjk_bold: Option<FontId>,
    supplemental_regular: Vec<Option<FontId>>,
    supplemental_bold: Vec<Option<FontId>>,
    fallback: Option<FontId>,
}

impl PageFonts {
    fn id(&self, track: Track, bold: bool) -> FontId {
        match (track, bold) {
            (Track::Primary, true) => self.primary_bold.clone(),
            (Track::Primary, false) => self.primary_regular.clone(),
            (Track::Cjk, true) => self
                .cjk_bold
                .clone()
                .unwrap_or_else(|| self.primary_bold.clone()),
            (Track::Cjk, false) => self
                .cjk_regular
                .clone()
                .unwrap_or_else(|| self.primary_regular.clone()),
            (Track::Supplemental(index), true) => self.supplemental_bold[index]
                .clone()
                .unwrap_or_else(|| self.primary_bold.clone()),
            (Track::Supplemental(index), false) => self.supplemental_regular[index]
                .clone()
                .unwrap_or_else(|| self.primary_regular.clone()),
            (Track::Fallback, _) => self
                .fallback
                .clone()
                .unwrap_or_else(|| self.primary_regular.clone()),
        }
    }
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

#[derive(Clone, Debug)]
struct ParsedPalette {
    primary_regular: Arc<ParsedFont>,
    primary_bold: Arc<ParsedFont>,
    cjk_regular: Arc<ParsedFont>,
    cjk_bold: Arc<ParsedFont>,
    supplemental_regular: Vec<Arc<ParsedFont>>,
    supplemental_bold: Vec<Arc<ParsedFont>>,
    fallback: Arc<ParsedFont>,
}

/// Resolve and parse all configured font tracks concurrently.
fn parse_palette_parallel(palette: &FontPalette) -> Result<ParsedPalette> {
    thread::scope(|scope| {
        let primary = palette.primary();
        let cjk = palette.cjk();
        let fallback = palette.fallback();
        let pr = scope.spawn(|| font_arc(primary, false));
        let pb = scope.spawn(|| font_arc(primary, true));
        let cr = scope.spawn(|| font_arc(cjk, false));
        let cb = scope.spawn(|| font_arc(cjk, true));
        let sr = palette
            .supplemental()
            .iter()
            .map(|family| scope.spawn(|| font_arc(family, false)))
            .collect::<Vec<_>>();
        let sb = palette
            .supplemental()
            .iter()
            .map(|family| scope.spawn(|| shaping_font_arc(family, true)))
            .collect::<Vec<_>>();
        let fb = scope.spawn(|| font_arc(fallback, false));
        Ok(ParsedPalette {
            primary_regular: pr
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))??,
            primary_bold: pb
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))??,
            cjk_regular: cr
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))??,
            cjk_bold: cb
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))??,
            supplemental_regular: joined_fonts(sr)?,
            supplemental_bold: joined_fonts(sb)?,
            fallback: fb
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))??,
        })
    })
}

fn joined_fonts(
    handles: Vec<std::thread::ScopedJoinHandle<'_, Result<Arc<ParsedFont>>>>,
) -> Result<Vec<Arc<ParsedFont>>> {
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("font parse panicked"))?
        })
        .collect()
}

fn embedded(font: &Arc<ParsedFont>, chars: &HashSet<char>, full: bool) -> Arc<ParsedFont> {
    if full && font.original_index == 0 {
        return font.clone();
    }
    Arc::new(subset_or_full(font, chars))
}

/// Decode and resize every row's thumbnail in parallel. Image decoding +
/// Lanczos resize is CPU-heavy — at ~4 ms per JPEG, 33 rows in serial cost
/// ~130 ms. Running across the available cores collapses that to roughly the
/// time of a single decode.
fn scale_thumbnails(
    rows: &[(VocabularyEntry, Option<PathBuf>)],
    thumbnail: &Thumbnail,
) -> Result<Vec<Option<DynamicImage>>> {
    let paths: Vec<Option<PathBuf>> = rows
        .iter()
        .map(|(_, image)| {
            image
                .as_ref()
                .filter(|path| path.is_file())
                .map(|path| path.to_path_buf())
        })
        .collect();
    thread::scope(|scope| {
        let handles: Vec<_> = paths
            .iter()
            .map(|path| {
                let thumbnail = thumbnail.clone();
                let path = path.clone();
                scope.spawn(move || -> Result<Option<DynamicImage>> {
                    match path {
                        Some(path) => Ok(Some(thumbnail.scaled(path.as_path())?)),
                        None => Ok(None),
                    }
                })
            })
            .collect();
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            out.push(
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("thumbnail decode panicked"))??,
            );
        }
        Ok(out)
    })
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

#[cfg(test)]
mod tests {
    use super::{ClassifierView, FontPalette, parse_palette_parallel, wrap_line};

    #[test]
    fn row_wrap_preserves_korean_and_mixed_thai_word_spaces() {
        let parsed = parse_palette_parallel(&FontPalette::default()).expect("fonts must resolve");
        let inputs = ["한국어 문장", "use ช่วย instead"];
        let output = inputs.map(|input| {
            wrap_line(input, 8.0, 200.0, false, ClassifierView::from(&parsed)).join("|")
        });
        assert_eq!(
            output, inputs,
            "row wrapping removed explicit Korean or Thai word spaces"
        );
    }
}
