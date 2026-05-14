//! Printable A4 card sheet — four fold-cards per page in monochrome layout.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Result, anyhow};
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Color, CurTransMat, FontId, ImageCompression, ImageOptimizationOptions, Line, LineDashPattern,
    LinePoint, Mm, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Polygon, PolygonRing, Pt, Rgb, TextItem, XObjectTransform,
};

use crate::vocabulary::VocabularyEntry;

use super::Thumbnail;
use super::font::{carries, font_arc, leading};
use super::{FontFamily, FontPalette};

const SHEET_W: f32 = 210.0;
const SHEET_H: f32 = 297.0;
const CARD_W: f32 = 105.0;
const HALF_H: f32 = 74.25;
const PAD: f32 = 5.0;
const IMAGE_X: f32 = 5.0;
const IMAGE_SIDE: f32 = 55.0;
const IMAGE_Y: f32 = (HALF_H - IMAGE_SIDE) / 2.0;
const COL_GAP: f32 = 5.0;
const TEXT_PAD_RIGHT: f32 = 5.0;
const PANEL_BORDER_PT: f32 = 0.6;
const IMPORTANCE_GLYPHS: &str = "Importance ";
const HAIR: f32 = 0.4;
const CUT_DASH: f32 = 1.4;
const CUT_GAP: f32 = 1.0;
const BEZIER_CIRCLE_K: f32 = 0.552_284_8;

const PHRASE_SIZE: f32 = 9.0;
const GLOSS_SIZE: f32 = 6.8;
const EN_SIZE: f32 = 10.6;
const IPA_SIZE: f32 = 6.8;
const LEX_SIZE: f32 = 8.6;
const MEANING_SIZE: f32 = 8.6;
const EXPLAIN_SIZE: f32 = 7.4;
const EXPLAIN_SIZE_MIN: f32 = 5.5;
const EXPLAIN_SIZE_STEP: f32 = 0.4;
const IMP_SIZE: f32 = 6.8;

const INK: (u8, u8, u8) = (0, 0, 0);
const MUTED: (u8, u8, u8) = (110, 108, 100);
const HAIRLINE: (u8, u8, u8) = (215, 213, 208);

/// Accumulate vocabulary cards and render them onto printable A4 sheets.
#[derive(Clone, Debug)]
pub struct CardSheet {
    palette: FontPalette,
    mono: FontFamily,
    cards: Vec<(VocabularyEntry, Option<PathBuf>)>,
}

impl Default for CardSheet {
    /// Return one empty card sheet with the default font palette.
    fn default() -> Self {
        Self::new()
    }
}

impl CardSheet {
    /// Create one empty card sheet.
    pub fn new() -> Self {
        Self {
            palette: FontPalette::default(),
            mono: FontFamily::new("Courier New"),
            cards: Vec::new(),
        }
    }

    /// Override the font palette — used by tests that pin a specific family.
    #[must_use]
    pub fn with_palette(mut self, palette: FontPalette) -> Self {
        self.palette = palette;
        self
    }

    /// Override the monospace family used for IPA — used by tests.
    #[must_use]
    pub fn with_mono(mut self, mono: FontFamily) -> Self {
        self.mono = mono;
        self
    }

    /// Append one card with its optional manga illustration path.
    pub fn append(&mut self, entry: &VocabularyEntry, image: Option<PathBuf>) {
        self.cards.push((entry.clone(), image));
    }

    /// Save the accumulated cards to one PDF file.
    pub fn save(&self, output: impl AsRef<Path>, thumbnail: &Thumbnail) -> Result<()> {
        if let Some(parent) = output.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let prepared = self.prepare_fonts()?;
        let mut doc = PdfDocument::new("Kamishibai Card Sheet");
        let ids = prepared.register(&mut doc);
        let scaled = scale_images(self.cards.as_slice(), thumbnail)?;
        let mut pages: Vec<Vec<Op>> = Vec::new();
        for chunk_start in (0..self.cards.len().max(1)).step_by(4) {
            let mut ops = Vec::new();
            self.draw_sheet(&mut doc, &mut ops, &prepared, &ids, &scaled, chunk_start);
            pages.push(ops);
        }
        let save = PdfSaveOptions {
            image_optimization: Some(ImageOptimizationOptions {
                quality: Some(0.85),
                max_image_size: None,
                dither_greyscale: None,
                convert_to_greyscale: Some(false),
                auto_optimize: Some(false),
                format: Some(ImageCompression::Jpeg),
            }),
            ..PdfSaveOptions::default()
        };
        let pdf = doc
            .with_pages(
                pages
                    .into_iter()
                    .map(|ops| PdfPage::new(Mm(SHEET_W), Mm(SHEET_H), ops))
                    .collect(),
            )
            .save(&save, &mut Vec::new());
        fs::write(output, pdf)?;
        Ok(())
    }

    /// Pre-subset every font track to the actual characters its cards assign
    /// to it. Same per-glyph dispatch as the row report: primary → CJK →
    /// fallback for body text, plus a separate mono track for IPA.
    fn prepare_fonts(&self) -> Result<SheetFonts> {
        let (primary_regular, primary_bold, cjk_regular, cjk_bold, fallback, mono_regular) =
            parse_palette_parallel(&self.palette, &self.mono)?;
        let mut buckets = CharBuckets::default();
        for (entry, _) in &self.cards {
            let plan = CardPlan::build(entry);
            plan.collect(
                &mut buckets,
                &primary_regular,
                &primary_bold,
                &cjk_regular,
                &cjk_bold,
                &fallback,
            );
        }
        if buckets.primary_regular.is_empty() {
            buckets.primary_regular.insert(' ');
        }
        if buckets.primary_bold.is_empty() {
            buckets.primary_bold.insert(' ');
        }
        if buckets.mono.is_empty() {
            buckets.mono.insert(' ');
        }
        Ok(SheetFonts {
            primary_regular: Arc::new(subset_or_full(&primary_regular, &buckets.primary_regular)),
            primary_bold: Arc::new(subset_or_full(&primary_bold, &buckets.primary_bold)),
            cjk_regular: (!buckets.cjk_regular.is_empty())
                .then(|| Arc::new(subset_or_full(&cjk_regular, &buckets.cjk_regular))),
            cjk_bold: (!buckets.cjk_bold.is_empty())
                .then(|| Arc::new(subset_or_full(&cjk_bold, &buckets.cjk_bold))),
            fallback: (!buckets.fallback.is_empty())
                .then(|| Arc::new(subset_or_full(&fallback, &buckets.fallback))),
            mono: Arc::new(subset_or_full(&mono_regular, &buckets.mono)),
            classifier_primary_regular: primary_regular,
            classifier_primary_bold: primary_bold,
            classifier_cjk_regular: cjk_regular,
            classifier_cjk_bold: cjk_bold,
            classifier_fallback: fallback,
            classifier_mono: mono_regular,
        })
    }

    /// Render one A4 sheet containing up to four cards starting at the given
    /// offset. Cut and fold marks track which rows are actually filled so a
    /// sparse last page only shows the cuts that release real cards.
    fn draw_sheet(
        &self,
        doc: &mut PdfDocument,
        ops: &mut Vec<Op>,
        fonts: &SheetFonts,
        ids: &SheetIds,
        scaled: &[Option<DynamicImage>],
        offset: usize,
    ) {
        let filled = [
            self.cards.get(offset).is_some(),
            self.cards.get(offset + 1).is_some(),
            self.cards.get(offset + 2).is_some(),
            self.cards.get(offset + 3).is_some(),
        ];
        draw_cut_lines(ops, filled);
        draw_fold_lines(ops, filled);
        for slot in 0..4 {
            let Some((entry, _)) = self.cards.get(offset + slot) else {
                continue;
            };
            let image = scaled.get(offset + slot).and_then(Option::as_ref).cloned();
            let (cell_x, cell_y) = cell_origin(slot);
            self.draw_card(doc, ops, fonts, ids, entry, image, cell_x, cell_y);
        }
    }

    /// Draw one card: the front face on the left half of the row and the back
    /// face on the right half, both upright. The printed row folds along its
    /// vertical centre line so the two faces meet back-to-back.
    #[allow(clippy::too_many_arguments)]
    fn draw_card(
        &self,
        doc: &mut PdfDocument,
        ops: &mut Vec<Op>,
        fonts: &SheetFonts,
        ids: &SheetIds,
        entry: &VocabularyEntry,
        image: Option<DynamicImage>,
        cell_x: f32,
        cell_y: f32,
    ) {
        let plan = CardPlan::build(entry);
        push_save_translate(ops, cell_x, cell_y);
        self.draw_front(doc, ops, fonts, ids, &plan, image);
        ops.push(Op::RestoreGraphicsState);
        push_save_translate(ops, cell_x + CARD_W, cell_y);
        self.draw_back(ops, fonts, ids, &plan);
        ops.push(Op::RestoreGraphicsState);
    }

    /// Render the front face inside the local (0,0)→(CARD_W, HALF_H) frame:
    /// manga panel on the left, source sentence and gloss on the right.
    fn draw_front(
        &self,
        doc: &mut PdfDocument,
        ops: &mut Vec<Op>,
        fonts: &SheetFonts,
        ids: &SheetIds,
        plan: &CardPlan,
        image: Option<DynamicImage>,
    ) {
        if let Some(decoded) = image {
            draw_image(doc, ops, decoded, IMAGE_X, IMAGE_Y, IMAGE_SIDE);
        }
        draw_panel_border(ops, IMAGE_X, IMAGE_Y, IMAGE_SIDE);
        let text_x = IMAGE_X + IMAGE_SIDE + COL_GAP;
        let text_w = CARD_W - text_x - TEXT_PAD_RIGHT;
        let mut cursor = IMAGE_Y + IMAGE_SIDE;
        let phrase_lines = wrap_runs(
            plan.front_phrase.as_slice(),
            text_w,
            PHRASE_SIZE,
            ClassifierView::from(fonts),
        );
        for line in phrase_lines {
            cursor -= leading(PHRASE_SIZE);
            draw_runs(ops, fonts, ids, &line, PHRASE_SIZE, text_x, cursor, INK);
        }
        cursor -= leading(PHRASE_SIZE) * 0.4;
        let gloss_lines = wrap_runs(
            &[(plan.gloss.clone(), false)],
            text_w,
            GLOSS_SIZE,
            ClassifierView::from(fonts),
        );
        for line in gloss_lines {
            cursor -= leading(GLOSS_SIZE);
            draw_runs(ops, fonts, ids, &line, GLOSS_SIZE, text_x, cursor, MUTED);
        }
    }

    /// Render the back face inside the local (0,0)→(CARD_W, HALF_H) frame:
    /// target sentence, lemma row, meaning, importance, and explanation.
    fn draw_back(&self, ops: &mut Vec<Op>, fonts: &SheetFonts, ids: &SheetIds, plan: &CardPlan) {
        let text_w = CARD_W - PAD * 2.0;
        let mut cursor = HALF_H - PAD;
        let en_lines = wrap_runs(
            plan.back_phrase.as_slice(),
            text_w,
            EN_SIZE,
            ClassifierView::from(fonts),
        );
        for line in en_lines {
            cursor -= leading(EN_SIZE);
            draw_runs(ops, fonts, ids, &line, EN_SIZE, PAD, cursor, INK);
        }
        cursor -= 1.4;
        draw_hairline(ops, PAD, cursor, CARD_W - PAD, cursor);
        cursor -= 1.6;
        cursor -= leading(LEX_SIZE);
        draw_runs(
            ops,
            fonts,
            ids,
            &[(plan.lemma.clone(), true)],
            LEX_SIZE,
            PAD,
            cursor,
            INK,
        );
        cursor -= leading(IPA_SIZE) * 0.95;
        draw_mono(
            ops,
            fonts,
            ids,
            plan.lemma_ipa.as_str(),
            IPA_SIZE,
            PAD,
            cursor,
            MUTED,
        );
        cursor -= leading(MEANING_SIZE) * 0.4;
        let meaning_lines = wrap_runs(
            &[(plan.meaning.clone(), false)],
            text_w,
            MEANING_SIZE,
            ClassifierView::from(fonts),
        );
        for line in meaning_lines {
            cursor -= leading(MEANING_SIZE);
            draw_runs(ops, fonts, ids, &line, MEANING_SIZE, PAD, cursor, INK);
        }
        cursor -= leading(IMP_SIZE) * 0.4;
        cursor -= leading(IMP_SIZE);
        draw_importance(ops, fonts, ids, plan.importance, PAD, cursor);
        cursor -= leading(EXPLAIN_SIZE) * 0.6;
        let view = ClassifierView::from(fonts);
        let explain_size = fit_explanation(&plan.explanation, text_w, cursor - PAD, view);
        let para_gap = leading(explain_size) * 0.35;
        'paragraphs: for (idx, paragraph) in plan.explanation.iter().enumerate() {
            if idx > 0 {
                cursor -= para_gap;
            }
            let lines = wrap_runs(&[(paragraph.clone(), false)], text_w, explain_size, view);
            for line in lines {
                cursor -= leading(explain_size);
                if cursor < PAD {
                    break 'paragraphs;
                }
                draw_runs(ops, fonts, ids, &line, explain_size, PAD, cursor, INK);
            }
        }
    }
}

/// Return the largest size in `[EXPLAIN_SIZE_MIN, EXPLAIN_SIZE]` (stepping by
/// `EXPLAIN_SIZE_STEP`) at which the wrapped, multi-paragraph explanation
/// fits inside `available` millimetres of vertical space. Accounts for the
/// inter-paragraph gap; falls back to the minimum size when even the
/// smallest pass overflows so the caller can still emit and let the cursor
/// guard clip the tail.
fn fit_explanation(
    paragraphs: &[String],
    width: f32,
    available: f32,
    view: ClassifierView<'_>,
) -> f32 {
    let mut size = EXPLAIN_SIZE;
    while size >= EXPLAIN_SIZE_MIN {
        let para_gap = leading(size) * 0.35;
        let mut total = 0.0_f32;
        for (idx, paragraph) in paragraphs.iter().enumerate() {
            if idx > 0 {
                total += para_gap;
            }
            let lines = wrap_runs(&[(paragraph.clone(), false)], width, size, view);
            total += lines.len() as f32 * leading(size);
        }
        if total <= available {
            return size;
        }
        size -= EXPLAIN_SIZE_STEP;
    }
    EXPLAIN_SIZE_MIN
}

/// One card's pre-computed text content with bold-highlighted runs.
#[derive(Clone, Debug)]
struct CardPlan {
    front_phrase: Vec<(String, bool)>,
    gloss: String,
    back_phrase: Vec<(String, bool)>,
    lemma: String,
    lemma_ipa: String,
    meaning: String,
    importance: u8,
    explanation: Vec<String>,
}

impl CardPlan {
    /// Build one card plan from one vocabulary entry. The source sentence is
    /// split into [before, highlight, after] runs so the idiom prints bold
    /// inside a regular-weight phrase; the target sentence reuses the term as
    /// its highlight when present, otherwise stays plain.
    fn build(entry: &VocabularyEntry) -> Self {
        let highlight = entry.source.highlight.as_str();
        let sentence = entry.source.sentence.as_str();
        let front = bold_split(sentence, highlight);
        let target = entry.target.sentence.as_str();
        let term = entry.term.as_str();
        let back = bold_split(target, term);
        Self {
            front_phrase: front,
            gloss: entry.source.hint.as_str().to_string(),
            back_phrase: back,
            lemma: entry.term.as_str().to_string(),
            lemma_ipa: pronounce(entry.pronunciation.as_str()),
            meaning: entry.meaning.as_str().to_string(),
            importance: entry.importance.value(),
            explanation: entry
                .source
                .context
                .as_str()
                .split('\n')
                .map(str::trim)
                .filter(|paragraph| !paragraph.is_empty())
                .map(String::from)
                .collect(),
        }
    }

    /// Accumulate every character into its routed bucket so the prepared
    /// fonts subset to exactly what this card writes.
    fn collect(
        &self,
        buckets: &mut CharBuckets,
        primary_regular: &ParsedFont,
        primary_bold: &ParsedFont,
        cjk_regular: &ParsedFont,
        cjk_bold: &ParsedFont,
        fallback: &ParsedFont,
    ) {
        let mut push_runs = |runs: &[(String, bool)]| {
            for (text, bold) in runs {
                for ch in text.chars() {
                    let track = dispatch(
                        ch,
                        *bold,
                        primary_regular,
                        primary_bold,
                        cjk_regular,
                        cjk_bold,
                        fallback,
                    );
                    buckets.insert(track, *bold, ch);
                }
            }
        };
        push_runs(self.front_phrase.as_slice());
        push_runs(&[(self.gloss.clone(), false)]);
        push_runs(self.back_phrase.as_slice());
        push_runs(&[(self.lemma.clone(), true)]);
        push_runs(&[(self.meaning.clone(), false)]);
        for paragraph in &self.explanation {
            push_runs(&[(paragraph.clone(), false)]);
        }
        push_runs(&[(String::from(IMPORTANCE_GLYPHS), false)]);
        for ch in self.lemma_ipa.chars() {
            buckets.mono.insert(ch);
        }
    }
}

/// Return the source sentence split into regular and bold runs around the
/// highlighted span. Falls back to a single regular run when the highlight is
/// not a verbatim substring.
fn bold_split(sentence: &str, highlight: &str) -> Vec<(String, bool)> {
    if highlight.is_empty() {
        return vec![(sentence.to_string(), false)];
    }
    if let Some(start) = sentence.find(highlight) {
        let before = &sentence[..start];
        let after = &sentence[start + highlight.len()..];
        let mut runs = Vec::new();
        if !before.is_empty() {
            runs.push((before.to_string(), false));
        }
        runs.push((highlight.to_string(), true));
        if !after.is_empty() {
            runs.push((after.to_string(), false));
        }
        return runs;
    }
    vec![(sentence.to_string(), false)]
}

/// Return one pronunciation string wrapped in IPA slashes when the source has
/// none, mirroring the design's `/lemma/` formatting.
fn pronounce(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('/') && trimmed.ends_with('/') {
        return trimmed.to_string();
    }
    format!("/{}/", trimmed.trim_matches('/'))
}

/// Return the bottom-left coordinate of the row for a slot index 0..4. Cards
/// stack top-to-bottom, each occupying one full-width `HALF_H`-tall row.
fn cell_origin(slot: usize) -> (f32, f32) {
    (0.0, SHEET_H - HALF_H * (slot as f32 + 1.0))
}

/// Push a save-state + translate matrix so the following operations draw in a
/// local frame whose origin sits at the given page-space coordinate (mm).
fn push_save_translate(ops: &mut Vec<Op>, tx: f32, ty: f32) {
    ops.push(Op::SaveGraphicsState);
    ops.push(Op::SetTransformationMatrix {
        matrix: CurTransMat::Translate(Pt(tx * 72.0 / 25.4), Pt(ty * 72.0 / 25.4)),
    });
}

/// Draw the dashed cut lines that separate the stacked cards. Cards stack
/// top-to-bottom, so the cuts are the horizontal boundaries between rows; a
/// boundary is emitted only when a real card sits on either side of it, so a
/// sparse last sheet does not drag empty rules across blank paper.
fn draw_cut_lines(ops: &mut Vec<Op>, filled: [bool; 4]) {
    let segments: [(bool, (f32, f32, f32, f32)); 3] = [
        (
            filled[0] || filled[1],
            (0.0, SHEET_H - HALF_H, SHEET_W, SHEET_H - HALF_H),
        ),
        (
            filled[1] || filled[2],
            (0.0, HALF_H * 2.0, SHEET_W, HALF_H * 2.0),
        ),
        (filled[2] || filled[3], (0.0, HALF_H, SHEET_W, HALF_H)),
    ];
    if !segments.iter().any(|(visible, _)| *visible) {
        return;
    }
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(HAIRLINE),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.5) });
    ops.push(Op::SetLineDashPattern {
        dash: LineDashPattern {
            offset: 0,
            dash_1: Some((CUT_DASH * 72.0 / 25.4) as i64),
            gap_1: Some((CUT_GAP * 72.0 / 25.4) as i64),
            ..Default::default()
        },
    });
    for (visible, (x1, y1, x2, y2)) in segments {
        if visible {
            draw_line(ops, x1, y1, x2, y2);
        }
    }
    ops.push(Op::SetLineDashPattern {
        dash: LineDashPattern::default(),
    });
}

/// Draw one hairline fold guide per filled card — the vertical centre line of
/// that card's row, where the front and back faces fold back-to-back. Empty
/// rows stay untouched so the cue only appears where there is something to
/// fold.
fn draw_fold_lines(ops: &mut Vec<Op>, filled: [bool; 4]) {
    if !filled.iter().any(|present| *present) {
        return;
    }
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(HAIRLINE),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.4) });
    for (slot, present) in filled.iter().enumerate() {
        if *present {
            let (_, cell_y) = cell_origin(slot);
            draw_line(ops, CARD_W, cell_y, CARD_W, cell_y + HALF_H);
        }
    }
}

/// Draw one straight stroke between two page-space points (mm).
fn draw_line(ops: &mut Vec<Op>, x1: f32, y1: f32, x2: f32, y2: f32) {
    ops.push(Op::DrawLine {
        line: Line {
            points: vec![
                LinePoint {
                    p: Point::new(Mm(x1), Mm(y1)),
                    bezier: false,
                },
                LinePoint {
                    p: Point::new(Mm(x2), Mm(y2)),
                    bezier: false,
                },
            ],
            is_closed: false,
        },
    });
}

/// Draw a hairline divider inside a local card frame.
fn draw_hairline(ops: &mut Vec<Op>, x1: f32, y1: f32, x2: f32, y2: f32) {
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(INK),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(HAIR) });
    draw_line(ops, x1, y1, x2, y2);
}

/// Draw the manga panel from an embedded raster. The image is scaled to fit
/// the panel square at 300 DPI so the printed sheet keeps the source's
/// pixel-perfect aesthetic.
fn draw_image(
    doc: &mut PdfDocument,
    ops: &mut Vec<Op>,
    image: DynamicImage,
    x: f32,
    y: f32,
    side: f32,
) {
    let (width, height) = image.dimensions();
    let raw = printpdf::RawImage {
        pixels: printpdf::RawImageData::U8(image.to_rgb8().into_raw()),
        width: width as usize,
        height: height as usize,
        data_format: printpdf::RawImageFormat::RGB8,
        tag: Vec::new(),
    };
    let scale = scale_to_side(side, width.max(height) as f32);
    let id = doc.add_image(&raw);
    ops.push(Op::UseXobject {
        id,
        transform: XObjectTransform {
            translate_x: Some(Mm(x).into()),
            translate_y: Some(Mm(y).into()),
            rotate: None,
            scale_x: Some(scale),
            scale_y: Some(scale),
            dpi: Some(300.0),
        },
    });
}

/// Stroke a thin black box around the manga panel.
fn draw_panel_border(ops: &mut Vec<Op>, x: f32, y: f32, side: f32) {
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(INK),
    });
    ops.push(Op::SetOutlineThickness {
        pt: Pt(PANEL_BORDER_PT),
    });
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing {
                points: rect_ring(x, y, side, side),
            }],
            mode: PaintMode::Stroke,
            winding_order: Default::default(),
        },
    });
}

/// Return the 4-vertex ring describing one axis-aligned rectangle in mm.
fn rect_ring(x: f32, y: f32, w: f32, h: f32) -> Vec<LinePoint> {
    vec![
        LinePoint {
            p: Point::new(Mm(x), Mm(y)),
            bezier: false,
        },
        LinePoint {
            p: Point::new(Mm(x + w), Mm(y)),
            bezier: false,
        },
        LinePoint {
            p: Point::new(Mm(x + w), Mm(y + h)),
            bezier: false,
        },
        LinePoint {
            p: Point::new(Mm(x), Mm(y + h)),
            bezier: false,
        },
    ]
}

/// Return the 12-point ring approximating a circle of radius `r` centred at
/// `(cx, cy)` with four cubic-bezier segments. Each segment encodes two
/// control handles followed by the next anchor, matching how printpdf
/// serialises bezier paths (see `serialize.rs::line_to_stream_ops`).
fn circle_ring(cx: f32, cy: f32, r: f32) -> Vec<LinePoint> {
    let k = r * BEZIER_CIRCLE_K;
    let anchor = |x: f32, y: f32| LinePoint {
        p: Point::new(Mm(x), Mm(y)),
        bezier: false,
    };
    let handle = |x: f32, y: f32| LinePoint {
        p: Point::new(Mm(x), Mm(y)),
        bezier: true,
    };
    vec![
        anchor(cx + r, cy),
        handle(cx + r, cy + k),
        handle(cx + k, cy + r),
        anchor(cx, cy + r),
        handle(cx - k, cy + r),
        handle(cx - r, cy + k),
        anchor(cx - r, cy),
        handle(cx - r, cy - k),
        handle(cx - k, cy - r),
        anchor(cx, cy - r),
        handle(cx + k, cy - r),
        handle(cx + r, cy - k),
    ]
}

/// Return the scale factor that fits one image side into the target side (mm)
/// at 300 DPI.
fn scale_to_side(side_mm: f32, pixels: f32) -> f32 {
    if pixels == 0.0 {
        return 1.0;
    }
    let points = side_mm * 72.0 / 25.4;
    points * 300.0 / (pixels * 72.0)
}

/// Draw the importance row: label and ten dots, no numeric trailer.
fn draw_importance(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    score: u8,
    x: f32,
    y: f32,
) {
    draw_runs(
        ops,
        fonts,
        ids,
        &[(String::from("Importance "), false)],
        IMP_SIZE,
        x,
        y,
        MUTED,
    );
    let label_w = measure_runs(
        ClassifierView::from(fonts),
        &[(String::from("Importance "), false)],
        IMP_SIZE,
    );
    let radius = 0.6_f32;
    let pitch = 1.8_f32;
    let cap_height = IMP_SIZE * 25.4 / 72.0 * 0.62;
    let dot_cy = y + cap_height / 2.0;
    for index in 0..10 {
        let dot_cx = x + label_w + radius + index as f32 * pitch;
        let filled = (index as u8) < score;
        draw_dot(ops, dot_cx, dot_cy, radius, filled);
    }
}

/// Draw one filled or outlined circular dot for the importance meter.
fn draw_dot(ops: &mut Vec<Op>, cx: f32, cy: f32, radius: f32, filled: bool) {
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(INK),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.5) });
    if filled {
        ops.push(Op::SetFillColor {
            col: rgb_tuple(INK),
        });
    }
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing {
                points: circle_ring(cx, cy, radius),
            }],
            mode: if filled {
                PaintMode::FillStroke
            } else {
                PaintMode::Stroke
            },
            winding_order: Default::default(),
        },
    });
}

/// Draw one IPA-style string in the monospace track at one baseline position.
#[allow(clippy::too_many_arguments)]
fn draw_mono(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
    color: (u8, u8, u8),
) {
    if text.is_empty() {
        return;
    }
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(x), Mm(y)),
    });
    ops.push(Op::SetLineHeight { lh: Pt(size) });
    ops.push(Op::SetFillColor {
        col: rgb_tuple(color),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(ids.mono.clone()),
        size: Pt(size),
    });
    let chars: String = text
        .chars()
        .filter(|ch| carries(fonts.classifier_mono.as_ref(), *ch))
        .collect();
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(chars)],
    });
    ops.push(Op::EndTextSection);
}

/// Emit one line composed of (text, bold) chunks at the given baseline. Each
/// chunk is split per character into primary / CJK / fallback runs at its
/// weight so mixed scripts share the same baseline.
#[allow(clippy::too_many_arguments)]
fn draw_runs(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    chunks: &[(String, bool)],
    size: f32,
    x: f32,
    y: f32,
    color: (u8, u8, u8),
) {
    if chunks.iter().all(|(text, _)| text.is_empty()) {
        return;
    }
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(x), Mm(y)),
    });
    ops.push(Op::SetLineHeight { lh: Pt(size) });
    ops.push(Op::SetFillColor {
        col: rgb_tuple(color),
    });
    for (text, bold) in chunks {
        if text.is_empty() {
            continue;
        }
        emit_chunk(ops, fonts, ids, text, *bold, size);
    }
    ops.push(Op::EndTextSection);
}

/// Emit one (text, bold) chunk inside an open text section, switching fonts
/// when the active track changes.
fn emit_chunk(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    text: &str,
    bold: bool,
    size: f32,
) {
    let mut buffer = String::new();
    let mut current = Track::Primary;
    let mut started = false;
    for ch in text.chars() {
        let track = dispatch(
            ch,
            bold,
            fonts.classifier_primary_regular.as_ref(),
            fonts.classifier_primary_bold.as_ref(),
            fonts.classifier_cjk_regular.as_ref(),
            fonts.classifier_cjk_bold.as_ref(),
            fonts.classifier_fallback.as_ref(),
        );
        if started && track != current {
            emit_run(ops, ids, &buffer, current, bold, size);
            buffer.clear();
        }
        buffer.push(ch);
        current = track;
        started = true;
    }
    if !buffer.is_empty() {
        emit_run(ops, ids, &buffer, current, bold, size);
    }
}

/// Emit one homogeneous run with its bound font.
fn emit_run(ops: &mut Vec<Op>, ids: &SheetIds, text: &str, track: Track, bold: bool, size: f32) {
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

/// Wrap a sequence of (text, bold) runs to fit the given width in mm. Each
/// output line preserves the per-run bold flag so the renderer can switch
/// fonts inside one visual line.
fn wrap_runs(
    runs: &[(String, bool)],
    width: f32,
    size: f32,
    view: ClassifierView<'_>,
) -> Vec<Vec<(String, bool)>> {
    let words = tokenize(runs);
    let mut lines: Vec<Vec<(String, bool)>> = Vec::new();
    let mut current: Vec<(String, bool)> = Vec::new();
    let mut current_width = 0.0_f32;
    let space_width = view.measure(" ", false, size);
    for (word, bold) in words {
        let word_width = view.measure(word.as_str(), bold, size);
        let extra = if current.is_empty() { 0.0 } else { space_width };
        if current_width + extra + word_width <= width || current.is_empty() {
            if extra > 0.0 {
                current.push((String::from(" "), bold));
            }
            current.push((word, bold));
            current_width += extra + word_width;
            continue;
        }
        lines.push(std::mem::take(&mut current));
        current.push((word, bold));
        current_width = word_width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Return the (word, bold) tokens taken from a run sequence.
fn tokenize(runs: &[(String, bool)]) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    for (text, bold) in runs {
        for word in text.split_whitespace() {
            tokens.push((word.to_string(), *bold));
        }
    }
    tokens
}

/// Return the measured width (mm) of (text, bold) chunks at the given size.
fn measure_runs(view: ClassifierView<'_>, chunks: &[(String, bool)], size: f32) -> f32 {
    chunks
        .iter()
        .map(|(text, bold)| view.measure(text.as_str(), *bold, size))
        .sum()
}

/// Pick the rendering track for a character at the current weight using the
/// primary → CJK → fallback dispatch chain.
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

/// Active routing destination for one character at the current weight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Track {
    Primary,
    Cjk,
    Fallback,
}

/// Mutable character collector across every font track used by the sheet.
#[derive(Default)]
struct CharBuckets {
    primary_regular: HashSet<char>,
    primary_bold: HashSet<char>,
    cjk_regular: HashSet<char>,
    cjk_bold: HashSet<char>,
    fallback: HashSet<char>,
    mono: HashSet<char>,
}

impl CharBuckets {
    /// Record one character into the bucket selected by track and weight.
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

/// Subsetted and classifier copies of every track the sheet embeds.
#[derive(Clone, Debug)]
struct SheetFonts {
    primary_regular: Arc<ParsedFont>,
    primary_bold: Arc<ParsedFont>,
    cjk_regular: Option<Arc<ParsedFont>>,
    cjk_bold: Option<Arc<ParsedFont>>,
    fallback: Option<Arc<ParsedFont>>,
    mono: Arc<ParsedFont>,
    classifier_primary_regular: Arc<ParsedFont>,
    classifier_primary_bold: Arc<ParsedFont>,
    classifier_cjk_regular: Arc<ParsedFont>,
    classifier_cjk_bold: Arc<ParsedFont>,
    classifier_fallback: Arc<ParsedFont>,
    classifier_mono: Arc<ParsedFont>,
}

impl SheetFonts {
    /// Register every embedded track with the document and return the font
    /// id table.
    fn register(&self, doc: &mut PdfDocument) -> SheetIds {
        SheetIds {
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
            mono: doc.add_font(&self.mono),
        }
    }
}

/// Font ids registered against one PDF document.
#[derive(Clone, Debug)]
struct SheetIds {
    primary_regular: FontId,
    primary_bold: FontId,
    cjk_regular: Option<FontId>,
    cjk_bold: Option<FontId>,
    fallback: Option<FontId>,
    mono: FontId,
}

/// Lightweight read-only view onto the classifier fonts used by wrap/measure.
#[derive(Clone, Copy)]
struct ClassifierView<'a> {
    primary_regular: &'a ParsedFont,
    primary_bold: &'a ParsedFont,
    cjk_regular: &'a ParsedFont,
    cjk_bold: &'a ParsedFont,
    fallback: &'a ParsedFont,
}

impl<'a> From<&'a SheetFonts> for ClassifierView<'a> {
    /// Borrow one classifier view from the prepared font set.
    fn from(value: &'a SheetFonts) -> Self {
        Self {
            primary_regular: value.classifier_primary_regular.as_ref(),
            primary_bold: value.classifier_primary_bold.as_ref(),
            cjk_regular: value.classifier_cjk_regular.as_ref(),
            cjk_bold: value.classifier_cjk_bold.as_ref(),
            fallback: value.classifier_fallback.as_ref(),
        }
    }
}

impl ClassifierView<'_> {
    /// Measure one string at the given size and weight in millimeters using
    /// the same dispatch chain the renderer follows.
    fn measure(&self, text: &str, bold: bool, size: f32) -> f32 {
        let mut total = 0.0_f32;
        for ch in text.chars() {
            let font = self.pick(ch, bold);
            let units = f32::from(font.font_metrics.units_per_em).max(1.0);
            let advance = font
                .lookup_glyph_index(ch as u32)
                .map(|gid| f32::from(font.get_horizontal_advance(gid)))
                .unwrap_or(units * 0.5);
            total += advance / units;
        }
        total * size * 25.4 / 72.0
    }

    /// Return the font that carries the character at the given weight.
    fn pick(&self, ch: char, bold: bool) -> &ParsedFont {
        let primary = if bold {
            self.primary_bold
        } else {
            self.primary_regular
        };
        if carries(primary, ch) {
            return primary;
        }
        let cjk = if bold {
            self.cjk_bold
        } else {
            self.cjk_regular
        };
        if carries(cjk, ch) {
            return cjk;
        }
        if carries(self.fallback, ch) {
            return self.fallback;
        }
        primary
    }
}

/// Decode and resize every card's thumbnail in parallel.
fn scale_images(
    cards: &[(VocabularyEntry, Option<PathBuf>)],
    thumbnail: &Thumbnail,
) -> Result<Vec<Option<DynamicImage>>> {
    let paths: Vec<Option<PathBuf>> = cards
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
                    .map_err(|_| anyhow!("card thumbnail decode panicked"))??,
            );
        }
        Ok(out)
    })
}

/// Resolve and parse all six embedded tracks concurrently.
#[allow(clippy::type_complexity)]
fn parse_palette_parallel(
    palette: &FontPalette,
    mono: &FontFamily,
) -> Result<(
    Arc<ParsedFont>,
    Arc<ParsedFont>,
    Arc<ParsedFont>,
    Arc<ParsedFont>,
    Arc<ParsedFont>,
    Arc<ParsedFont>,
)> {
    thread::scope(|scope| {
        let primary = palette.primary();
        let cjk = palette.cjk();
        let fallback = palette.fallback();
        let pr = scope.spawn(|| font_arc(primary, false));
        let pb = scope.spawn(|| font_arc(primary, true));
        let cr = scope.spawn(|| font_arc(cjk, false));
        let cb = scope.spawn(|| font_arc(cjk, true));
        let fb = scope.spawn(|| font_arc(fallback, false));
        let mn = scope.spawn(|| font_arc(mono, false));
        Ok((
            pr.join().map_err(|_| anyhow!("font parse panicked"))??,
            pb.join().map_err(|_| anyhow!("font parse panicked"))??,
            cr.join().map_err(|_| anyhow!("font parse panicked"))??,
            cb.join().map_err(|_| anyhow!("font parse panicked"))??,
            fb.join().map_err(|_| anyhow!("font parse panicked"))??,
            mn.join().map_err(|_| anyhow!("font parse panicked"))??,
        ))
    })
}

/// Subset one font down to the supplied character set; falls back to the full
/// font when allsorts cannot subset it.
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

/// Return one PDF RGB color value from a 0..=255 component tuple.
fn rgb_tuple(rgb: (u8, u8, u8)) -> Color {
    Color::Rgb(Rgb::new(
        f32::from(rgb.0) / 255.0,
        f32::from(rgb.1) / 255.0,
        f32::from(rgb.2) / 255.0,
        None,
    ))
}
