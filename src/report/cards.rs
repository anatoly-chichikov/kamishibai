//! Printable A4 sheets with four fixed-size foldcards and measured text bounds.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Result, anyhow};
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Codepoint, Color, CurTransMat, FontId, ImageCompression, ImageOptimizationOptions, Line,
    LineDashPattern, LinePoint, Mm, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Polygon, PolygonRing, Pt, Rgb, TextItem, TextMatrix, XObjectTransform,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::languages::{TextDirection, language};
use crate::markdown::{Block, TextChunk, parse_card_context};
use crate::vocabulary::VocabularyEntry;

use super::Thumbnail;
use super::font::{carries, font_arc, leading, shaping_font_arc};
use super::shaping::{
    inferred_direction, shape, shape_with_context, supplemental_slot, visual_runs,
};
use super::{FontFamily, FontPalette};

const SHEET_W: f32 = 210.0;
const SHEET_H: f32 = 297.0;
const CARD_W: f32 = 105.0;
const HALF_H: f32 = 74.25;
const PAD: f32 = 5.0;
const IMAGE_X: f32 = 5.0;
const IMAGE_SIDE: f32 = 55.0;
const COL_GAP: f32 = 5.0;
const TEXT_PAD_RIGHT: f32 = 5.0;
const PANEL_BORDER_PT: f32 = 0.6;
const IMPORTANCE_GLYPHS: &str = "Importance ";
const BULLET_MARKER: &str = "•  ";
/// The same marker for a right-to-left column: the dot moves to the outer edge
/// and the gap keeps it off the text. Same glyphs, so the same width.
const BULLET_MARKER_RTL: &str = "  •";
const BULLET_INDENT_STEP: f32 = 3.5;
const BLOCK_GAP_RATIO: f32 = 0.35;
const HAIR: f32 = 0.4;
const BEZIER_CIRCLE_K: f32 = 0.552_284_8;

const PHRASE_SIZE: f32 = 7.65;
const GLOSS_SIZE: f32 = 5.0;
const EN_SIZE: f32 = 10.6;
const IPA_SIZE: f32 = 6.8;
const LEX_SIZE: f32 = 8.6;
const MEANING_SIZE: f32 = 8.6;
const EXPLAIN_SIZE: f32 = 7.4;
const EXPLAIN_SIZE_MIN: f32 = 5.5;
const EXPLAIN_SIZE_STEP: f32 = 0.4;
const IMP_SIZE: f32 = 6.8;
const COMPACT_PAD: f32 = 4.0;
const EXPLAIN_SIZE_FLOOR: f32 = 5.0;
const ITALIC_SLANT: f32 = 0.21;

const INK: (u8, u8, u8) = (0, 0, 0);
const MUTED: (u8, u8, u8) = (110, 108, 100);
const GLOSS_INK: (u8, u8, u8) = (150, 148, 142);
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
        for fragments in self.pages(&prepared)? {
            let mut ops = Vec::new();
            self.draw_sheet(&mut doc, &mut ops, &prepared, &ids, &scaled, &fragments);
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
        fs::write(output, super::font_embedding::normalized(pdf)?)?;
        Ok(())
    }

    /// Pre-subset every font track to the actual characters its cards assign
    /// to it. Same per-glyph dispatch as the row report: primary → CJK →
    /// fallback for body text, plus a separate mono track for IPA.
    fn prepare_fonts(&self) -> Result<SheetFonts> {
        let parsed = parse_palette_parallel(&self.palette, &self.mono)?;
        let mut buckets = CharBuckets::new(parsed.supplemental_regular.len());
        for (entry, _) in &self.cards {
            let plan = CardPlan::build(entry);
            plan.collect(&mut buckets, ClassifierView::from(&parsed));
        }
        buckets.primary_regular.insert(' ');
        if buckets.primary_bold.is_empty() {
            buckets.primary_bold.insert(' ');
        }
        if buckets.mono.is_empty() {
            buckets.mono.insert(' ');
        }
        let shaping_active = buckets.shaping;
        Ok(SheetFonts {
            primary_regular: Arc::new(subset_or_full(
                &parsed.primary_regular,
                &buckets.primary_regular,
            )),
            primary_bold: Arc::new(subset_or_full(&parsed.primary_bold, &buckets.primary_bold)),
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
            mono: Arc::new(subset_or_full(&parsed.mono, &buckets.mono)),
            classifier_primary_regular: parsed.primary_regular,
            classifier_primary_bold: parsed.primary_bold,
            classifier_cjk_regular: parsed.cjk_regular,
            classifier_cjk_bold: parsed.cjk_bold,
            classifier_supplemental_regular: parsed.supplemental_regular,
            classifier_supplemental_bold: parsed.supplemental_bold,
            classifier_fallback: parsed.fallback,
            classifier_mono: parsed.mono,
            shaping_active,
        })
    }

    /// Fit every card before drawing and keep four fixed rows on each page.
    fn pages(&self, fonts: &SheetFonts) -> Result<Vec<Vec<FixedCard>>> {
        let mut pages = Vec::new();
        let mut page = Vec::new();
        for (index, (entry, _)) in self.cards.iter().enumerate() {
            let plan = CardPlan::build(entry);
            let faces = CardLayout::build(&plan, fonts).map_err(|error| {
                anyhow!(
                    "card {} ({}) cannot fit its fixed printable size: {}",
                    index + 1,
                    entry.term.as_str(),
                    error,
                )
            })?;
            page.push(FixedCard { card: index, faces });
            if page.len() == 4 {
                pages.push(std::mem::take(&mut page));
            }
        }
        if !page.is_empty() || pages.is_empty() {
            pages.push(page);
        }
        Ok(pages)
    }

    /// Draw cut and fold guides at the actual boundaries of measured cards.
    fn draw_sheet(
        &self,
        doc: &mut PdfDocument,
        ops: &mut Vec<Op>,
        fonts: &SheetFonts,
        ids: &SheetIds,
        scaled: &[Option<DynamicImage>],
        fragments: &[FixedCard],
    ) {
        draw_cut_lines(ops, fragments);
        draw_fold_lines(ops, fragments);
        let mut top = SHEET_H;
        for fragment in fragments {
            top -= HALF_H;
            let image = scaled.get(fragment.card).and_then(Option::as_ref).cloned();
            push_save_translate(ops, 0.0, top);
            let image_y = (HALF_H - IMAGE_SIDE) / 2.0;
            if let Some(decoded) = image {
                draw_image(doc, ops, decoded, IMAGE_X, image_y, IMAGE_SIDE);
            }
            draw_panel_border(ops, IMAGE_X, image_y, IMAGE_SIDE);
            self.draw_face(ops, fonts, ids, fragment, true);
            ops.push(Op::RestoreGraphicsState);
            push_save_translate(ops, CARD_W, top);
            self.draw_face(ops, fonts, ids, fragment, false);
            ops.push(Op::RestoreGraphicsState);
        }
    }

    /// Render one bounded face using the same rows that determined its height.
    fn draw_face(
        &self,
        ops: &mut Vec<Op>,
        fonts: &SheetFonts,
        ids: &SheetIds,
        fragment: &FixedCard,
        front: bool,
    ) {
        let rows = if front {
            &fragment.faces.front
        } else {
            &fragment.faces.back
        };
        let (x, width, padding) = if front {
            let x = IMAGE_X + IMAGE_SIDE + COL_GAP;
            (x, CARD_W - x - TEXT_PAD_RIGHT, PAD)
        } else {
            (
                fragment.faces.padding,
                CARD_W - fragment.faces.padding * 2.0,
                fragment.faces.padding,
            )
        };
        let mut cursor = if front {
            (HALF_H * 0.70).max(rows_height(rows) + padding)
        } else {
            HALF_H - padding
        };
        for row in rows {
            cursor -= row.height();
            draw_face_row(ops, fonts, ids, row, x, cursor, width);
        }
        debug_assert!(
            cursor >= padding - 0.001,
            "planned card text crossed its bottom inset"
        );
    }
}

/// One text baseline, rule, or importance row with its measured spacing.
#[derive(Clone, Debug)]
struct FaceRow {
    content: RowContent,
    size: f32,
    gap: f32,
    direction: TextDirection,
}

impl FaceRow {
    /// Return the complete vertical advance consumed by this row.
    fn height(&self) -> f32 {
        self.gap
            + if matches!(self.content, RowContent::Rule) {
                self.size
            } else {
                leading(self.size)
            }
    }
}

/// The rendering role and preserved styled content of one face row.
#[derive(Clone, Debug)]
enum RowContent {
    Text {
        chunks: Vec<TextChunk>,
        color: (u8, u8, u8),
        bullet: Option<BulletLead>,
    },
    Mono(String),
    Rule,
    Importance(u8),
}

/// The independently flowing front and back of one vocabulary card.
#[derive(Clone, Debug)]
struct CardLayout {
    front: Vec<FaceRow>,
    back: Vec<FaceRow>,
    padding: f32,
}

impl CardLayout {
    /// Fit every field inside one fixed face, using compact spacing only when needed.
    fn build(plan: &CardPlan, fonts: &SheetFonts) -> Result<Self> {
        let view = ClassifierView::from(fonts);
        let front_width = CARD_W - IMAGE_X - IMAGE_SIDE - COL_GAP - TEXT_PAD_RIGHT;
        let mut front = text_rows(
            &plan.front_phrase,
            front_width,
            PHRASE_SIZE,
            plan.source_direction,
            INK,
            view,
        );
        let mut hint = text_rows(
            &[italic_chunk(&plan.gloss)],
            front_width,
            GLOSS_SIZE,
            plan.source_direction,
            GLOSS_INK,
            view,
        );
        if let Some(row) = hint.first_mut() {
            row.gap = leading(PHRASE_SIZE) * 0.4;
        }
        front.extend(hint);
        if rows_height(&front) > HALF_H - PAD * 2.0 {
            return Err(anyhow!(
                "source sentence and hint need {:.2}mm but only {:.2}mm is available",
                rows_height(&front),
                HALF_H - PAD * 2.0
            ));
        }
        for padding in [PAD, COMPACT_PAD] {
            let mut size = EXPLAIN_SIZE;
            loop {
                let back = back_rows(plan, size, fonts, padding);
                if rows_height(&back) <= HALF_H - padding * 2.0 {
                    return Ok(Self {
                        front,
                        back,
                        padding,
                    });
                }
                if size <= EXPLAIN_SIZE_MIN {
                    break;
                }
                size = (size - EXPLAIN_SIZE_STEP).max(EXPLAIN_SIZE_MIN);
            }
        }
        for step in 1_u8..=5 {
            let size = (EXPLAIN_SIZE_MIN - f32::from(step) * 0.1).max(EXPLAIN_SIZE_FLOOR);
            let back = back_rows(plan, size, fonts, COMPACT_PAD);
            if rows_height(&back) <= HALF_H - COMPACT_PAD * 2.0 {
                return Ok(Self {
                    front,
                    back,
                    padding: COMPACT_PAD,
                });
            }
        }
        let back = back_rows(plan, EXPLAIN_SIZE_FLOOR, fonts, COMPACT_PAD);
        Err(anyhow!(
            "back text needs {:.2}mm but only {:.2}mm is available at the {:.1}pt readable floor; shorten the explanation before publishing",
            rows_height(&back),
            HALF_H - COMPACT_PAD * 2.0,
            EXPLAIN_SIZE_FLOOR
        ))
    }
}

/// One vocabulary card whose two faces have passed the fixed-size layout check.
#[derive(Clone, Debug)]
struct FixedCard {
    card: usize,
    faces: CardLayout,
}

/// Sum exactly the advances the renderer consumes for these rows.
fn rows_height(rows: &[FaceRow]) -> f32 {
    rows.iter().map(FaceRow::height).sum()
}

/// Wrap one text field with its styling, color, and reading direction intact.
fn text_rows(
    chunks: &[TextChunk],
    width: f32,
    size: f32,
    direction: TextDirection,
    color: (u8, u8, u8),
    view: ClassifierView<'_>,
) -> Vec<FaceRow> {
    wrap_runs(chunks, width, size, view)
        .into_iter()
        .map(|chunks| FaceRow {
            content: RowContent::Text {
                chunks,
                color,
                bullet: None,
            },
            size,
            gap: 0.0,
            direction,
        })
        .collect()
}

/// Wrap the entire back face so long headings and pronunciations can continue.
fn back_rows(plan: &CardPlan, size: f32, fonts: &SheetFonts, padding: f32) -> Vec<FaceRow> {
    let view = ClassifierView::from(fonts);
    let width = CARD_W - padding * 2.0;
    let mut rows = text_rows(
        &plan.back_phrase,
        width,
        EN_SIZE,
        plan.target_direction,
        INK,
        view,
    );
    rows.push(FaceRow {
        content: RowContent::Rule,
        size: 3.0,
        gap: 0.0,
        direction: plan.target_direction,
    });
    rows.extend(text_rows(
        &[bold_chunk(&plan.lemma)],
        width,
        LEX_SIZE,
        plan.target_direction,
        INK,
        view,
    ));
    rows.extend(
        wrap_mono(
            &plan.lemma_ipa,
            width,
            IPA_SIZE,
            fonts.classifier_mono.as_ref(),
        )
        .into_iter()
        .map(|text| FaceRow {
            content: RowContent::Mono(text),
            size: IPA_SIZE,
            gap: -leading(IPA_SIZE) * 0.05,
            direction: TextDirection::Ltr,
        }),
    );
    let mut meaning = text_rows(
        &[plain_chunk(&plan.meaning)],
        width,
        MEANING_SIZE,
        plan.source_direction,
        INK,
        view,
    );
    if let Some(row) = meaning.first_mut() {
        row.gap = leading(MEANING_SIZE) * 0.4;
    }
    rows.extend(meaning);
    rows.push(FaceRow {
        content: RowContent::Importance(plan.importance),
        size: IMP_SIZE,
        gap: leading(IMP_SIZE) * 0.4,
        direction: plan.source_direction,
    });
    let explanation = explanation_layout(&plan.explanation, width, size, view);
    rows.extend(
        explanation
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| FaceRow {
                content: RowContent::Text {
                    chunks: row.chunks,
                    color: INK,
                    bullet: row.bullet,
                },
                size,
                gap: row.gap_before
                    + if index == 0 {
                        leading(EXPLAIN_SIZE) * 0.6
                    } else {
                        0.0
                    },
                direction: plan.source_direction,
            }),
    );
    if padding < PAD {
        for row in &mut rows {
            if row.gap > 0.0 {
                row.gap *= 0.35;
            }
            if matches!(row.content, RowContent::Rule) {
                row.size = 1.8;
            }
        }
    }
    rows
}

/// Wrap pronunciation by graphemes using the same monospace font as drawing.
fn wrap_mono(text: &str, width: f32, size: f32, font: &ParsedFont) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut advance = 0.0;
    let units = f32::from(font.font_metrics.units_per_em).max(1.0);
    for grapheme in text.graphemes(true) {
        let measured = shape(font, grapheme, false)
            .map(|glyphs| {
                glyphs
                    .iter()
                    .map(|glyph| f32::from(glyph.x_advance))
                    .sum::<f32>()
            })
            .unwrap_or(units * 0.5)
            / units
            * size
            * 25.4
            / 72.0;
        if !current.is_empty() && advance + measured > width {
            lines.push(std::mem::take(&mut current));
            advance = 0.0;
        }
        current.push_str(grapheme);
        advance += measured;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Render exactly one previously measured face row.
#[allow(clippy::too_many_arguments)]
fn draw_face_row(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    row: &FaceRow,
    x: f32,
    cursor: f32,
    width: f32,
) {
    match &row.content {
        RowContent::Rule => draw_hairline(
            ops,
            x,
            cursor + row.size * (1.6 / 3.0),
            x + width,
            cursor + row.size * (1.6 / 3.0),
        ),
        RowContent::Importance(value) => {
            draw_importance(ops, fonts, ids, *value, x, cursor, row.direction)
        }
        RowContent::Mono(text) => draw_mono(ops, fonts, ids, text, row.size, x, cursor, MUTED),
        RowContent::Text {
            chunks,
            color,
            bullet,
        } => {
            let (text_x, inner) = if let Some(bullet) = bullet {
                let marker_width =
                    ClassifierView::from(fonts).measure(BULLET_MARKER, false, row.size);
                let indent = bullet_indent(bullet.indent, width, marker_width);
                let (marker_x, text_x) = bullet_gutter(row.direction, indent, marker_width, width);
                let marker_x = marker_x + x - PAD;
                let text_x = text_x + x - PAD;
                if bullet.marker {
                    draw_runs(
                        ops,
                        fonts,
                        ids,
                        &[plain_chunk(bullet_marker(row.direction))],
                        row.size,
                        marker_x,
                        cursor,
                        *color,
                        TextDirection::Ltr,
                        marker_width,
                    );
                }
                (text_x, width - indent - marker_width)
            } else {
                (x, width)
            };
            draw_runs(
                ops,
                fonts,
                ids,
                chunks,
                row.size,
                text_x,
                cursor,
                *color,
                row.direction,
                inner,
            );
        }
    }
}

/// Return the marker glyphs for one reading direction.
fn bullet_marker(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Ltr => BULLET_MARKER,
        TextDirection::Rtl => BULLET_MARKER_RTL,
    }
}

/// Keep even imported deep indentation inside the printable text frame.
fn bullet_indent(indent: u8, width: f32, marker: f32) -> f32 {
    (f32::from(indent) * BULLET_INDENT_STEP).min((width - marker - 10.0).max(0.0))
}

/// Return where one bullet's marker and its text frame start.
///
/// The indent eats the margin the reader starts from — the left one going
/// left-to-right, the right one going right-to-left — and the marker sits in
/// the gutter that indent opens. Either way the marker's inner edge meets the
/// text frame's outer edge, so a right-aligned right-to-left line ends up
/// against its own dot instead of across the card from it.
///
/// The frame's width is `text_w - indent - marker_w` in both directions, which
/// is why explanation wrapping needs no direction of its own.
fn bullet_gutter(
    direction: TextDirection,
    indent_mm: f32,
    marker_w: f32,
    text_w: f32,
) -> (f32, f32) {
    match direction {
        TextDirection::Ltr => (PAD + indent_mm, PAD + indent_mm + marker_w),
        TextDirection::Rtl => (PAD + text_w - indent_mm - marker_w, PAD),
    }
}

/// Wrap every explanation block into immutable rows before rendering begins.
fn explanation_layout(
    blocks: &[Block],
    width: f32,
    size: f32,
    view: ClassifierView<'_>,
) -> ExplanationLayout {
    let marker_width = view.measure(BULLET_MARKER, false, size);
    let mut rows = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let gap_before = explanation_gap(blocks.get(index.wrapping_sub(1)), block, size);
        let (chunks, inner_w, indent) = match block {
            Block::Paragraph(chunks) => (chunks.as_slice(), width, None),
            Block::Bullet { indent, chunks } => {
                let indent_mm = bullet_indent(*indent, width, marker_width);
                (
                    chunks.as_slice(),
                    (width - indent_mm - marker_width).max(10.0),
                    Some(*indent),
                )
            }
        };
        for (line_index, chunks) in wrap_runs(chunks, inner_w, size, view)
            .into_iter()
            .enumerate()
        {
            let gap = if line_index == 0 { gap_before } else { 0.0 };
            rows.push(ExplanationRow {
                chunks,
                gap_before: gap,
                bullet: indent.map(|value| BulletLead {
                    indent: value,
                    marker: line_index == 0,
                }),
            });
        }
    }
    ExplanationLayout { rows }
}

/// Return the gap between unlike blocks while keeping one bullet list tight.
fn explanation_gap(previous: Option<&Block>, current: &Block, size: f32) -> f32 {
    match (previous, current) {
        (None, _) | (Some(Block::Bullet { .. }), Block::Bullet { .. }) => 0.0,
        (Some(_), _) => leading(size) * BLOCK_GAP_RATIO,
    }
}

/// One complete explanation plan selected before any back-face text is drawn.
#[derive(Clone, Debug)]
struct ExplanationLayout {
    rows: Vec<ExplanationRow>,
}

/// One wrapped explanation baseline with its optional bullet gutter.
#[derive(Clone, Debug)]
struct ExplanationRow {
    chunks: Vec<TextChunk>,
    gap_before: f32,
    bullet: Option<BulletLead>,
}

/// One bullet marker placement attached only to its first wrapped row.
#[derive(Clone, Copy, Debug)]
struct BulletLead {
    indent: u8,
    marker: bool,
}

/// One card's pre-computed text content with bold-highlighted runs.
#[derive(Clone, Debug)]
struct CardPlan {
    front_phrase: Vec<TextChunk>,
    source_direction: TextDirection,
    gloss: String,
    back_phrase: Vec<TextChunk>,
    target_direction: TextDirection,
    lemma: String,
    lemma_ipa: String,
    meaning: String,
    importance: u8,
    explanation: Vec<Block>,
}

impl CardPlan {
    /// Build one card plan from one vocabulary entry. The source sentence is
    /// split into [before, highlight, after] runs so the idiom prints bold
    /// inside a regular-weight phrase; the target sentence reuses the term as
    /// its highlight when present, otherwise stays plain. `source_context`
    /// is parsed as light markdown so V14-style bold headers and bulleted
    /// senses render as structured blocks on the card back.
    fn build(entry: &VocabularyEntry) -> Self {
        let highlight = entry.source.highlight.as_str();
        let sentence = entry.source.sentence.as_str();
        let front = bold_split(sentence, highlight);
        let target = entry.target.sentence.as_str();
        let term = entry.term.as_str();
        let back = bold_split(target, term);
        Self {
            front_phrase: front,
            source_direction: language(entry.source.lang.as_str())
                .expect("invariant: vocabulary source language must be supported")
                .direction,
            gloss: entry.source.hint.as_str().to_string(),
            back_phrase: back,
            target_direction: language(entry.target.lang.as_str())
                .expect("invariant: vocabulary target language must be supported")
                .direction,
            lemma: entry.term.as_str().to_string(),
            lemma_ipa: pronounce(entry.pronunciation.as_str()),
            meaning: entry.meaning.as_str().to_string(),
            importance: entry.importance.value(),
            explanation: parse_card_context(entry.source.context.as_str()),
        }
    }

    /// Accumulate every character into its routed bucket so the prepared
    /// fonts subset to exactly what this card writes.
    fn collect(&self, buckets: &mut CharBuckets, view: ClassifierView<'_>) {
        let mut push_runs = |runs: &[TextChunk]| {
            for chunk in runs {
                for ch in chunk.text.chars() {
                    let track = view.track(ch, chunk.bold);
                    buckets.insert(track, chunk.bold, ch);
                }
            }
        };
        push_runs(self.front_phrase.as_slice());
        push_runs(&[italic_chunk(self.gloss.as_str())]);
        push_runs(self.back_phrase.as_slice());
        push_runs(&[bold_chunk(self.lemma.as_str())]);
        push_runs(&[plain_chunk(self.meaning.as_str())]);
        for block in &self.explanation {
            match block {
                Block::Paragraph(chunks) => push_runs(chunks.as_slice()),
                Block::Bullet { chunks, .. } => {
                    push_runs(&[plain_chunk(BULLET_MARKER)]);
                    push_runs(chunks.as_slice());
                }
            }
        }
        push_runs(&[plain_chunk(IMPORTANCE_GLYPHS)]);
        for ch in self.lemma_ipa.chars() {
            buckets.mono.insert(ch);
        }
    }
}

/// Return the source sentence split into regular and bold runs around the
/// highlighted span. Falls back to a single regular run when the highlight is
/// not a verbatim substring.
fn bold_split(sentence: &str, highlight: &str) -> Vec<TextChunk> {
    if highlight.is_empty() {
        return vec![plain_chunk(sentence)];
    }
    if let Some(start) = sentence.find(highlight) {
        let before = &sentence[..start];
        let after = &sentence[start + highlight.len()..];
        let mut runs = Vec::new();
        if !before.is_empty() {
            runs.push(plain_chunk(before));
        }
        runs.push(TextChunk {
            text: highlight.to_string(),
            bold: true,
            italic: false,
        });
        if !after.is_empty() {
            runs.push(plain_chunk(after));
        }
        return runs;
    }
    vec![plain_chunk(sentence)]
}

/// Build one neutral text chunk — no bold, no italic.
fn plain_chunk(text: &str) -> TextChunk {
    TextChunk {
        text: text.to_string(),
        bold: false,
        italic: false,
    }
}

/// Build one bold-only text chunk — used for lemma-style runs.
fn bold_chunk(text: &str) -> TextChunk {
    TextChunk {
        text: text.to_string(),
        bold: true,
        italic: false,
    }
}

/// Build one italic-only text chunk — used for the front-face gloss.
fn italic_chunk(text: &str) -> TextChunk {
    TextChunk {
        text: text.to_string(),
        bold: false,
        italic: true,
    }
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

/// Push a save-state + translate matrix so the following operations draw in a
/// local frame whose origin sits at the given page-space coordinate (mm).
fn push_save_translate(ops: &mut Vec<Op>, tx: f32, ty: f32) {
    ops.push(Op::SaveGraphicsState);
    ops.push(Op::SetTransformationMatrix {
        matrix: CurTransMat::Translate(Pt(tx * 72.0 / 25.4), Pt(ty * 72.0 / 25.4)),
    });
}

/// Draw dashed cuts only at the actual lower boundaries of occupied rows.
fn draw_cut_lines(ops: &mut Vec<Op>, fragments: &[FixedCard]) {
    if fragments.is_empty() {
        return;
    }
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(HAIRLINE),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.5) });
    ops.push(Op::SetLineDashPattern {
        dash: LineDashPattern {
            offset: 0,
            dash_1: Some(3),
            gap_1: Some(2),
            ..Default::default()
        },
    });
    let mut top = SHEET_H;
    for _ in fragments {
        top -= HALF_H;
        if top > 0.0 {
            draw_line(ops, 0.0, top, SHEET_W, top);
        }
    }
    ops.push(Op::SetLineDashPattern {
        dash: LineDashPattern::default(),
    });
}

/// Give each occupied foldcard one guide spanning its complete measured height.
fn draw_fold_lines(ops: &mut Vec<Op>, fragments: &[FixedCard]) {
    if fragments.is_empty() {
        return;
    }
    ops.push(Op::SetOutlineColor {
        col: rgb_tuple(HAIRLINE),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.4) });
    let mut top = SHEET_H;
    for _ in fragments {
        let bottom = top - HALF_H;
        draw_line(ops, CARD_W, bottom, CARD_W, top);
        top = bottom;
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
/// Draw the importance meter: the label, then ten dots reading away from it.
///
/// The label stays English and stays readable left to right; only the row's
/// anchor follows the card. On a right-to-left card the row hangs off the right
/// margin instead of stranding itself under text that has moved away.
fn draw_importance(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    score: u8,
    x: f32,
    y: f32,
    direction: TextDirection,
) {
    let label_w = measure_runs(
        ClassifierView::from(fonts),
        &[plain_chunk(IMPORTANCE_GLYPHS)],
        IMP_SIZE,
    );
    let radius = 0.6_f32;
    let pitch = 1.8_f32;
    let label_x = match direction {
        TextDirection::Ltr => x,
        TextDirection::Rtl => (CARD_W - PAD - label_w).max(x),
    };
    draw_runs(
        ops,
        fonts,
        ids,
        &[plain_chunk(IMPORTANCE_GLYPHS)],
        IMP_SIZE,
        label_x,
        y,
        MUTED,
        TextDirection::Ltr,
        label_w,
    );
    let cap_height = IMP_SIZE * 25.4 / 72.0 * 0.62;
    let dot_cy = y + cap_height / 2.0;
    for index in 0..10 {
        let step = radius + index as f32 * pitch;
        let dot_cx = match direction {
            TextDirection::Ltr => label_x + label_w + step,
            TextDirection::Rtl => label_x - step,
        };
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

/// Draw one logical line of styled chunks starting at the given baseline.
/// Each chunk runs in its own text section so per-chunk italic slants stay
/// independent and per-character font dispatch routes mixed scripts. Italic
/// is rendered as a synthetic oblique through the text matrix — there is no
/// italic font track.
#[allow(clippy::too_many_arguments)]
fn draw_runs(
    ops: &mut Vec<Op>,
    fonts: &SheetFonts,
    ids: &SheetIds,
    chunks: &[TextChunk],
    size: f32,
    x_start: f32,
    y: f32,
    color: (u8, u8, u8),
    direction: TextDirection,
    width: f32,
) {
    let view = ClassifierView::from(fonts);
    let spans = render_spans(chunks, direction, view);
    let line_width = spans
        .iter()
        .map(|span| {
            if fonts.shaping_active && matches!(span.track, Track::Supplemental(_)) {
                return shaped_span_width(view.font(span.track, span.bold), span, size);
            }
            view.measure(span.text.as_str(), span.bold, size)
        })
        .sum::<f32>();
    let mut x = if direction == TextDirection::Rtl {
        x_start + (width - line_width).max(0.0)
    } else {
        x_start
    };
    for span in spans {
        let font = view.font(span.track, span.bold);
        let id = ids.id(span.track, span.bold);
        let advance = if fonts.shaping_active && matches!(span.track, Track::Supplemental(_)) {
            emit_shaped(ops, font, id, &span, size, x, y, color)
        } else {
            emit_plain(ops, id, &span, size, x, y, color);
            view.measure(span.text.as_str(), span.bold, size)
        };
        x += advance;
    }
}

/// One styled, font-homogeneous visual span ready for shaping.
#[derive(Clone, Debug)]
struct RenderSpan {
    text: String,
    pre_context: String,
    post_context: String,
    bold: bool,
    italic: bool,
    track: Track,
    rtl: bool,
}

/// Resolve styled logical chunks into visual-order spans while preserving
/// their original font weight and emphasis.
fn render_spans(
    chunks: &[TextChunk],
    direction: TextDirection,
    view: ClassifierView<'_>,
) -> Vec<RenderSpan> {
    let mut text = String::new();
    let mut styles = Vec::new();
    for chunk in chunks {
        let start = text.len();
        text.push_str(chunk.text.as_str());
        styles.push((start..text.len(), chunk.bold, chunk.italic));
    }
    let mut spans = Vec::new();
    for run in visual_runs(text.as_str(), direction) {
        let mut directional = Vec::new();
        for (style, bold, italic) in &styles {
            let start = style.start.max(run.range.start);
            let end = style.end.min(run.range.end);
            if start >= end {
                continue;
            }
            for span in view.font_spans(&text[start..end], *bold) {
                let global_start = start + span.range.start;
                let global_end = start + span.range.end;
                directional.push(RenderSpan {
                    text: span.text,
                    pre_context: text[..global_start].to_string(),
                    post_context: text[global_end..].to_string(),
                    bold: *bold,
                    italic: *italic,
                    track: span.track,
                    rtl: run.rtl,
                });
            }
        }
        if run.rtl {
            directional.reverse();
        }
        spans.extend(directional);
    }
    spans
}

/// Emit one shaped run as individually positioned glyph ids.
#[allow(clippy::too_many_arguments)]
fn emit_shaped(
    ops: &mut Vec<Op>,
    font: &ParsedFont,
    id: FontId,
    span: &RenderSpan,
    size: f32,
    x: f32,
    y: f32,
    color: (u8, u8, u8),
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
    ops.push(Op::SetFillColor {
        col: rgb_tuple(color),
    });
    for glyph in glyphs {
        let offset_x = cursor + f32::from(glyph.x_offset);
        let offset_y = f32::from(glyph.y_offset);
        ops.push(Op::SetTextMatrix {
            matrix: TextMatrix::Raw([
                1.0,
                0.0,
                if span.italic { ITALIC_SLANT } else { 0.0 },
                1.0,
                x * 72.0 / 25.4 + offset_x * scale,
                y * 72.0 / 25.4 + offset_y * scale,
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

/// Emit one ordinary LTR span through printpdf's Unicode text path.
#[allow(clippy::too_many_arguments)]
fn emit_plain(
    ops: &mut Vec<Op>,
    id: FontId,
    span: &RenderSpan,
    size: f32,
    x: f32,
    y: f32,
    color: (u8, u8, u8),
) {
    ops.push(Op::StartTextSection);
    if span.italic {
        ops.push(Op::SetTextMatrix {
            matrix: TextMatrix::Raw([
                1.0,
                0.0,
                ITALIC_SLANT,
                1.0,
                x * 72.0 / 25.4,
                y * 72.0 / 25.4,
            ]),
        });
    } else {
        ops.push(Op::SetTextCursor {
            pos: Point::new(Mm(x), Mm(y)),
        });
    }
    ops.push(Op::SetLineHeight { lh: Pt(size) });
    ops.push(Op::SetFillColor {
        col: rgb_tuple(color),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(id),
        size: Pt(size),
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(span.text.clone())],
    });
    ops.push(Op::EndTextSection);
}

fn shaped_span_width(font: &ParsedFont, span: &RenderSpan, size: f32) -> f32 {
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

/// Wrap a sequence of styled chunks to fit the given width in mm. Each
/// output line preserves the per-chunk weight and italic flags so the
/// renderer can switch fonts and matrices inside one visual line. Glued
/// punctuation — a comma or period the regular run carries straight after
/// the bold highlight — wraps together with the word it abuts instead of
/// drifting onto its own line.
fn wrap_runs(
    runs: &[TextChunk],
    width: f32,
    size: f32,
    view: ClassifierView<'_>,
) -> Vec<Vec<TextChunk>> {
    let groups = group_tokens(tokenize(runs));
    let mut lines: Vec<Vec<TextChunk>> = Vec::new();
    let mut current: Vec<TextChunk> = Vec::new();
    let mut current_width = 0.0_f32;
    let space_width = view.measure(" ", false, size);
    for (space_before, group) in groups
        .into_iter()
        .flat_map(|group| split_wide_group(group, width, size, view))
    {
        let group_width: f32 = group
            .iter()
            .map(|chunk| view.measure(chunk.text.as_str(), chunk.bold, size))
            .sum();
        let needs_space = !current.is_empty() && space_before;
        let extra = if needs_space { space_width } else { 0.0 };
        if !current.is_empty() && current_width + extra + group_width > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0.0;
        }
        let needs_space = !current.is_empty() && space_before;
        if needs_space {
            current.push(TextChunk {
                text: String::from(" "),
                bold: false,
                italic: false,
            });
            current_width += space_width;
        }
        current_width += group_width;
        current.extend(group);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Break only an overwide token into grapheme-safe groups without adding spaces.
fn split_wide_group(
    group: (bool, Vec<TextChunk>),
    width: f32,
    size: f32,
    view: ClassifierView<'_>,
) -> Vec<(bool, Vec<TextChunk>)> {
    if measure_runs(view, &group.1, size) <= width {
        return vec![group];
    }
    let mut groups = Vec::new();
    for chunk in group.1 {
        for grapheme in chunk.text.graphemes(true) {
            groups.push((
                groups.is_empty() && group.0,
                vec![TextChunk {
                    text: grapheme.to_string(),
                    bold: chunk.bold,
                    italic: chunk.italic,
                }],
            ));
        }
    }
    groups
}

/// One wrap token: its text, weight, italic flag, and whether whitespace
/// precedes it in the source. A `space_before` of false marks punctuation
/// glued to the previous token — carried into the regular run straight after
/// the bold highlight — so it never gains a leading space or wraps away from it.
#[derive(Clone, Debug)]
struct WrapToken {
    text: String,
    bold: bool,
    italic: bool,
    space_before: bool,
}

/// Split a chunk sequence into wrap tokens. `split_whitespace` marks the gaps
/// inside one chunk; at a chunk boundary a space exists unless the previous
/// chunk ends and the next chunk starts on non-whitespace, which is exactly
/// how `bold_split` leaves a trailing comma or period attached to the
/// highlight. CJK-bearing tokens are then expanded character-by-character so
/// wrap has real break points inside scripts that do not use spaces between
/// words.
fn tokenize(runs: &[TextChunk]) -> Vec<WrapToken> {
    let mut tokens = Vec::new();
    let mut prev_open = false;
    for chunk in runs {
        if chunk.text.is_empty() {
            continue;
        }
        let starts_ws = chunk.text.starts_with(char::is_whitespace);
        for (index, word) in chunk.text.split_whitespace().enumerate() {
            let space_before = if tokens.is_empty() {
                false
            } else if index == 0 {
                starts_ws || !prev_open
            } else {
                true
            };
            tokens.extend(expand_unspaced(WrapToken {
                text: word.to_string(),
                bold: chunk.bold,
                italic: chunk.italic,
                space_before,
            }));
        }
        prev_open = !chunk.text.ends_with(char::is_whitespace);
    }
    tokens
}

/// Expand one whitespace-delimited token into per-character sub-tokens when
/// it carries CJK code points. Latin / Cyrillic runs inside the same token
/// stay glued together as a single sub-token. CJK sub-tokens keep
/// `space_before=false` so the renderer never inserts a literal space; the
/// fresh wrap-group decision lives in `group_tokens`.
fn expand_unspaced(token: WrapToken) -> Vec<WrapToken> {
    if !token.text.chars().any(is_unspaced) {
        return vec![token];
    }
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut first = true;
    for grapheme in token.text.graphemes(true) {
        if grapheme.chars().any(is_unspaced) {
            if !buffer.is_empty() {
                out.push(WrapToken {
                    text: std::mem::take(&mut buffer),
                    bold: token.bold,
                    italic: token.italic,
                    space_before: first && token.space_before,
                });
                first = false;
            }
            out.push(WrapToken {
                text: grapheme.to_string(),
                bold: token.bold,
                italic: token.italic,
                space_before: first && token.space_before,
            });
            first = false;
        } else {
            buffer.push_str(grapheme);
        }
    }
    if !buffer.is_empty() {
        out.push(WrapToken {
            text: buffer,
            bold: token.bold,
            italic: token.italic,
            space_before: first && token.space_before,
        });
    }
    out
}

/// Return whether the character belongs to a script that does not separate
/// words with spaces — Hiragana, Katakana, Hangul, and the CJK ideographic
/// ranges. These need to be treated as line-break candidates per character.
fn is_cjk(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x3000..=0x303F
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2A6DF
            | 0xAC00..=0xD7AF
    )
}

/// Return whether one character belongs to a script whose words can require
/// line breaks without ASCII whitespace.
fn is_unspaced(ch: char) -> bool {
    is_cjk(ch) || matches!(u32::from(ch), 0x0E00..=0x0E7F)
}

/// Coalesce wrap tokens into groups: each group is one space-preceded token
/// plus any glued punctuation that follows it. CJK characters always start a
/// fresh group on either side so wrap can pick any of them as a break point.
fn group_tokens(tokens: Vec<WrapToken>) -> Vec<(bool, Vec<TextChunk>)> {
    let mut groups: Vec<(bool, Vec<TextChunk>)> = Vec::new();
    for token in tokens {
        let starts_with_cjk = token.text.chars().any(is_unspaced);
        let prev_ends_with_cjk = groups
            .last()
            .and_then(|(_, group)| group.last())
            .is_some_and(|chunk| chunk.text.chars().any(is_unspaced));
        let break_here = token.space_before || starts_with_cjk || prev_ends_with_cjk;
        let space_before = token.space_before;
        let chunk = TextChunk {
            text: token.text,
            bold: token.bold,
            italic: token.italic,
        };
        if break_here || groups.is_empty() {
            groups.push((space_before, vec![chunk]));
        } else {
            groups
                .last_mut()
                .expect("invariant: a glued token always follows an existing group")
                .1
                .push(chunk);
        }
    }
    groups
}

/// Return the measured width (mm) of styled chunks at the given size.
fn measure_runs(view: ClassifierView<'_>, chunks: &[TextChunk], size: f32) -> f32 {
    chunks
        .iter()
        .map(|chunk| view.measure(chunk.text.as_str(), chunk.bold, size))
        .sum()
}

/// Active routing destination for one character at the current weight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Track {
    Primary,
    Cjk,
    Supplemental(usize),
    Fallback,
}

/// Mutable character collector across every font track used by the sheet.
struct CharBuckets {
    primary_regular: HashSet<char>,
    primary_bold: HashSet<char>,
    cjk_regular: HashSet<char>,
    cjk_bold: HashSet<char>,
    supplemental_regular: Vec<HashSet<char>>,
    supplemental_bold: Vec<HashSet<char>>,
    fallback: HashSet<char>,
    mono: HashSet<char>,
    shaping: bool,
}

impl CharBuckets {
    /// Create empty buckets for every configured supplemental slot.
    fn new(supplemental: usize) -> Self {
        Self {
            primary_regular: HashSet::new(),
            primary_bold: HashSet::new(),
            cjk_regular: HashSet::new(),
            cjk_bold: HashSet::new(),
            supplemental_regular: vec![HashSet::new(); supplemental],
            supplemental_bold: vec![HashSet::new(); supplemental],
            fallback: HashSet::new(),
            mono: HashSet::new(),
            shaping: false,
        }
    }

    /// Record one character into the bucket selected by track and weight.
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

/// Subsetted and classifier copies of every track the sheet embeds.
#[derive(Clone, Debug)]
struct SheetFonts {
    primary_regular: Arc<ParsedFont>,
    primary_bold: Arc<ParsedFont>,
    cjk_regular: Option<Arc<ParsedFont>>,
    cjk_bold: Option<Arc<ParsedFont>>,
    supplemental_regular: Vec<Option<Arc<ParsedFont>>>,
    supplemental_bold: Vec<Option<Arc<ParsedFont>>>,
    fallback: Option<Arc<ParsedFont>>,
    mono: Arc<ParsedFont>,
    classifier_primary_regular: Arc<ParsedFont>,
    classifier_primary_bold: Arc<ParsedFont>,
    classifier_cjk_regular: Arc<ParsedFont>,
    classifier_cjk_bold: Arc<ParsedFont>,
    classifier_supplemental_regular: Vec<Arc<ParsedFont>>,
    classifier_supplemental_bold: Vec<Arc<ParsedFont>>,
    classifier_fallback: Arc<ParsedFont>,
    classifier_mono: Arc<ParsedFont>,
    shaping_active: bool,
}

impl SheetFonts {
    /// Register every embedded track with the document and return the font
    /// id table.
    fn register(&self, doc: &mut PdfDocument) -> SheetIds {
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
            supplemental_regular,
            supplemental_bold,
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
    supplemental_regular: Vec<Option<FontId>>,
    supplemental_bold: Vec<Option<FontId>>,
    fallback: Option<FontId>,
    mono: FontId,
}

impl SheetIds {
    /// Return the registered font id for one rendering track and weight.
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

/// Lightweight read-only view onto the classifier fonts used by wrap/measure.
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

impl<'a> From<&'a SheetFonts> for ClassifierView<'a> {
    /// Borrow one classifier view from the prepared font set.
    fn from(value: &'a SheetFonts) -> Self {
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

impl<'a> From<&'a ParsedPalette> for ClassifierView<'a> {
    /// Borrow one classifier view from an unprepared palette.
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

impl ClassifierView<'_> {
    /// Measure one string at the given size and weight in millimeters using
    /// the same dispatch chain the renderer follows.
    fn measure(&self, text: &str, bold: bool, size: f32) -> f32 {
        let mut total = 0.0_f32;
        for span in self.font_spans(text, bold) {
            let font = self.font(span.track, bold);
            let units = f32::from(font.font_metrics.units_per_em).max(1.0);
            let rtl = inferred_direction(span.text.as_str()) == TextDirection::Rtl;
            let advance = shape(font, span.text.as_str(), rtl)
                .map(|glyphs| {
                    glyphs
                        .iter()
                        .map(|glyph| f32::from(glyph.x_advance))
                        .sum::<f32>()
                })
                .unwrap_or(units * 0.5);
            total += advance / units;
        }
        total * size * 25.4 / 72.0
    }

    /// Return the routing track that carries a character at the given weight.
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

    /// Return the parsed font bound to a rendering track and weight.
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

    /// Split one string whenever actual glyph coverage selects a new font.
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

/// Fully parsed classifier fonts before usage-based embedding decisions.
#[derive(Clone, Debug)]
struct ParsedPalette {
    primary_regular: Arc<ParsedFont>,
    primary_bold: Arc<ParsedFont>,
    cjk_regular: Arc<ParsedFont>,
    cjk_bold: Arc<ParsedFont>,
    supplemental_regular: Vec<Arc<ParsedFont>>,
    supplemental_bold: Vec<Arc<ParsedFont>>,
    fallback: Arc<ParsedFont>,
    mono: Arc<ParsedFont>,
}

/// Resolve and parse all configured font tracks concurrently.
fn parse_palette_parallel(palette: &FontPalette, mono: &FontFamily) -> Result<ParsedPalette> {
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
        let mn = scope.spawn(|| font_arc(mono, false));
        Ok(ParsedPalette {
            primary_regular: pr.join().map_err(|_| anyhow!("font parse panicked"))??,
            primary_bold: pb.join().map_err(|_| anyhow!("font parse panicked"))??,
            cjk_regular: cr.join().map_err(|_| anyhow!("font parse panicked"))??,
            cjk_bold: cb.join().map_err(|_| anyhow!("font parse panicked"))??,
            supplemental_regular: joined_fonts(sr)?,
            supplemental_bold: joined_fonts(sb)?,
            fallback: fb.join().map_err(|_| anyhow!("font parse panicked"))??,
            mono: mn.join().map_err(|_| anyhow!("font parse panicked"))??,
        })
    })
}

fn joined_fonts(
    handles: Vec<std::thread::ScopedJoinHandle<'_, Result<Arc<ParsedFont>>>>,
) -> Result<Vec<Arc<ParsedFont>>> {
    handles
        .into_iter()
        .map(|handle| handle.join().map_err(|_| anyhow!("font parse panicked"))?)
        .collect()
}

fn embedded(font: &Arc<ParsedFont>, chars: &HashSet<char>, full: bool) -> Arc<ParsedFont> {
    if full && font.original_index == 0 {
        return font.clone();
    }
    Arc::new(subset_or_full(font, chars))
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

#[cfg(test)]
mod tests {
    use super::{
        BULLET_MARKER, BULLET_MARKER_RTL, CardPlan, CardSheet, ClassifierView, EXPLAIN_SIZE_MIN,
        FontPalette, PAD, ParsedPalette, bold_split, bullet_gutter, bullet_marker,
        explanation_layout, group_tokens, plain_chunk, tokenize, wrap_runs,
    };
    use crate::languages::TextDirection;
    use crate::markdown::{Block, parse_markdown};
    use crate::report::font::font_arc;
    use crate::vocabulary::{
        Importance, LanguageCode, NonEmptyText, VocabularyEntry, VocabularySource, VocabularyTarget,
    };
    use printpdf::{Op, PdfDocument, TextItem};
    use std::sync::Arc;

    /// Return one realistic dense card whose variable-height rows each wrap
    /// twice before its six reviewed senses and four context sections.
    fn dense_six_sense_entry() -> VocabularyEntry {
        VocabularyEntry {
            term: NonEmptyText::new("bank").expect("term must be valid"),
            meaning: NonEmptyText::new(
                "to tilt an aircraft sideways during a turn by lifting one wing above the other meaning-anchor",
            )
            .expect("meaning must be valid"),
            pronunciation: NonEmptyText::new("bank").expect("pronunciation must be valid"),
            transcription: NonEmptyText::new("bank").expect("transcription must be valid"),
            importance: Importance::new(5).expect("importance must be valid"),
            source: VocabularySource {
                sentence: NonEmptyText::new("The aircraft made a smooth bank")
                    .expect("source sentence must be valid"),
                lang: LanguageCode::new("en").expect("source language must be valid"),
                highlight: NonEmptyText::new("bank").expect("highlight must be valid"),
                hint: NonEmptyText::new("a controlled turn").expect("hint must be valid"),
                context: NonEmptyText::new("**Meaning.**\n- **a financial institution that safeguards deposits and lends money to people and companies [finance]**\n- the sloping ground immediately beside a river, lake, canal, or similar body of water [landform]\n- a stored reserve of blood, data, food, or other resources for future use [reserve]\n- a long row or tier of matching objects arranged closely beside one another [row]\n- to rely confidently on a person, promise, event, or expected future result [rely]\n- to tilt an aircraft sideways while turning by raising one wing above another [aviation]\nThe noun senses grew from ideas of an edge or accumulated mass; the verbs developed separately.\n\n**Where you'll hear it.** Common in finance, geography, computing, medicine, aviation, and dependent plans.\n\n**Where it's out of place.** Do not use the finance sense for a wallet, safe, or ordinary container.\n\n**Subtlety.** Bank on takes a person or outcome; bank the aircraft names deliberate sideways tilt final-context-anchor.")
                    .expect("context must be valid"),
            },
            target: VocabularyTarget {
                sentence: NonEmptyText::new(
                    "The pilot banked the aircraft smoothly while approaching the distant runway target-anchor",
                )
                .expect("target sentence must be valid"),
                lang: LanguageCode::new("ru").expect("target language must be valid"),
            },
        }
    }

    /// A bullet's text must end up against its own dot, whichever way the card
    /// reads. This is the geometry the Hebrew card got wrong: the marker sat on
    /// the left margin while the right-aligned text hugged the right one.
    #[test]
    fn a_bullet_marker_always_touches_the_text_frame_it_belongs_to() {
        let text_w = 95.0_f32;
        let marker_w = 3.0_f32;
        let apart = [0.0_f32, 3.5, 7.0]
            .into_iter()
            .flat_map(|indent| {
                [TextDirection::Ltr, TextDirection::Rtl].map(move |direction| (direction, indent))
            })
            .filter(|(direction, indent)| {
                let (marker_x, text_x) = bullet_gutter(*direction, *indent, marker_w, text_w);
                let inner_w = text_w - indent - marker_w;
                match direction {
                    TextDirection::Ltr => (marker_x + marker_w - text_x).abs() > f32::EPSILON,
                    TextDirection::Rtl => (text_x + inner_w - marker_x).abs() > f32::EPSILON,
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            apart,
            Vec::<(TextDirection, f32)>::new(),
            "a bullet marker drifted away from the text frame it marks"
        );
    }

    /// However deep the indent and whichever way the card reads, the marker
    /// stays inside the panel it was drawn for.
    #[test]
    fn a_bullet_marker_never_leaves_the_panel() {
        let text_w = 95.0_f32;
        let marker_w = 3.0_f32;
        let escaped = [0.0_f32, 3.5, 7.0, 10.5]
            .into_iter()
            .flat_map(|indent| {
                [TextDirection::Ltr, TextDirection::Rtl].map(move |direction| (direction, indent))
            })
            .filter(|(direction, indent)| {
                let (marker_x, _) = bullet_gutter(*direction, *indent, marker_w, text_w);
                marker_x < PAD || marker_x + marker_w > PAD + text_w
            })
            .collect::<Vec<_>>();
        assert_eq!(
            escaped,
            Vec::<(TextDirection, f32)>::new(),
            "a bullet marker was placed outside the panel"
        );
    }

    /// The indent eats the margin the reader starts from, so a nested bullet
    /// moves inward from opposite sides in the two directions.
    #[test]
    fn indenting_a_bullet_moves_it_away_from_the_margin_the_reader_starts_from() {
        let text_w = 95.0_f32;
        let marker_w = 3.0_f32;
        let (flush_ltr, _) = bullet_gutter(TextDirection::Ltr, 0.0, marker_w, text_w);
        let (nested_ltr, _) = bullet_gutter(TextDirection::Ltr, 7.0, marker_w, text_w);
        let (flush_rtl, _) = bullet_gutter(TextDirection::Rtl, 0.0, marker_w, text_w);
        let (nested_rtl, _) = bullet_gutter(TextDirection::Rtl, 7.0, marker_w, text_w);
        assert_eq!(
            (nested_ltr > flush_ltr, nested_rtl < flush_rtl),
            (true, true),
            "a nested bullet did not move inward from the margin its reader starts at"
        );
    }

    /// Both markers carry the same glyphs, so the width the layout measured
    /// once holds for either direction.
    #[test]
    fn the_two_bullet_markers_are_the_same_glyphs_mirrored() {
        assert_eq!(
            (
                bullet_marker(TextDirection::Rtl),
                BULLET_MARKER_RTL.chars().rev().collect::<String>().as_str(),
            ),
            (BULLET_MARKER_RTL, BULLET_MARKER),
            "the right-to-left bullet marker is not the mirror of the left-to-right one"
        );
    }

    /// A comma carried into the regular run right after the bold highlight
    /// stays in the same wrap group as the word it abuts.
    #[test]
    fn punctuation_after_the_highlight_stays_glued_to_its_word() {
        let runs = bold_split("Сегодня холоднее, чем было вчера.", "холоднее");
        let groups = group_tokens(tokenize(runs.as_slice()));
        let glued = groups
            .iter()
            .find(|(_, group)| group.iter().any(|chunk| chunk.text == "холоднее"))
            .is_some_and(|(_, group)| group.iter().any(|chunk| chunk.text == ","));
        assert!(
            glued,
            "a comma abutting the bold highlight drifted out of its word group"
        );
    }

    /// A sentence-final period after the highlight does not become a lonely,
    /// space-prefixed token of its own.
    #[test]
    fn a_sentence_final_period_does_not_become_its_own_token() {
        let runs = bold_split("Они посмотрели военное шествие.", "шествие");
        let lonely_period = tokenize(runs.as_slice())
            .iter()
            .any(|token| token.text == "." && token.space_before);
        assert!(
            !lonely_period,
            "a sentence-final period gained a leading space instead of hugging its word"
        );
    }

    /// Ordinary whitespace-separated words keep the spaces between them.
    #[test]
    fn ordinary_words_keep_their_separating_spaces() {
        let tokens = tokenize(&[plain_chunk("one two three")]);
        let spaced = tokens.iter().skip(1).all(|token| token.space_before);
        assert!(
            spaced,
            "ordinary whitespace-separated words lost the spaces between them"
        );
    }

    /// A dense reviewed-sense list keeps every content token inside the
    /// available height and spends no vertical gap between adjacent bullets.
    #[test]
    fn a_six_sense_explanation_plans_every_token_inside_the_card() {
        let palette = FontPalette::default();
        let parsed = ParsedPalette {
            primary_regular: font_arc(palette.primary(), false).expect("primary must resolve"),
            primary_bold: font_arc(palette.primary(), true).expect("primary bold must resolve"),
            cjk_regular: font_arc(palette.cjk(), false).expect("CJK must resolve"),
            cjk_bold: font_arc(palette.cjk(), true).expect("CJK bold must resolve"),
            supplemental_regular: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, false).expect("supplemental must resolve"))
                .collect(),
            supplemental_bold: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, true).expect("supplemental bold must resolve"))
                .collect(),
            fallback: font_arc(palette.fallback(), false).expect("fallback must resolve"),
            mono: Arc::clone(
                &font_arc(palette.primary(), false).expect("test mono substitute must resolve"),
            ),
        };
        let blocks = parse_markdown(
            "**Meaning.**\n- **a financial institution that safeguards deposits and lends money to people and companies [finance]**\n- the sloping ground immediately beside a river, lake, canal, or similar body of water [landform]\n- a stored reserve of blood, data, food, or other resources for future use [reserve]\n- a long row or tier of matching objects arranged closely beside one another [row]\n- to rely confidently on a person, promise, event, or expected future result [rely]\n- to tilt an aircraft sideways while turning by raising one wing above another [aviation]\nThe noun senses grew from ideas of an edge or accumulated mass; the verbs developed separately.\n\n**Where you'll hear it.** Common in finance, geography, computing, medicine, aviation, and dependent plans.\n\n**Where it's out of place.** Do not use the finance sense for a wallet, safe, or ordinary container.\n\n**Subtlety.** Bank on takes a person or outcome; bank the aircraft names deliberate sideways tilt.",
        );
        let view = ClassifierView::from(&parsed);
        let layout = explanation_layout(&blocks, 95.0, EXPLAIN_SIZE_MIN, view);
        let expected = blocks
            .iter()
            .flat_map(|block| match block {
                Block::Paragraph(chunks) | Block::Bullet { chunks, .. } => chunks.as_slice(),
            })
            .flat_map(|chunk| chunk.text.split_whitespace())
            .map(String::from)
            .collect::<Vec<_>>();
        let actual = layout
            .rows
            .iter()
            .flat_map(|row| row.chunks.as_slice())
            .flat_map(|chunk| chunk.text.split_whitespace())
            .map(String::from)
            .collect::<Vec<_>>();
        let spaced_bullet_rows = layout
            .rows
            .iter()
            .filter(|row| row.bullet.is_some() && row.gap_before > 0.0)
            .count();
        assert_eq!(
            (actual, spaced_bullet_rows),
            (expected, 1),
            "the six-sense plan omitted content or spaced adjacent bullets"
        );
    }

    /// Rendering the dense two-line card emits an identifying tail token from
    /// every reviewed sense and context section instead of stopping at PAD.
    #[test]
    fn a_dense_six_sense_back_emits_every_planned_content_tail() {
        let item = dense_six_sense_entry();
        let mut sheet = CardSheet::new();
        sheet.append(&item, None);
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let mut document = PdfDocument::new("dense six-sense back");
        let ids = fonts.register(&mut document);
        let plan = CardPlan::build(&item);
        let view = ClassifierView::from(&fonts);
        let target_lines = wrap_runs(plan.back_phrase.as_slice(), 95.0, super::EN_SIZE, view).len();
        let meaning_lines = wrap_runs(
            &[plain_chunk(plan.meaning.as_str())],
            95.0,
            super::MEANING_SIZE,
            view,
        )
        .len();
        let mut ops = Vec::new();
        for page in sheet.pages(&fonts).expect("card must fit") {
            for fragment in page {
                sheet.draw_face(&mut ops, &fonts, &ids, &fragment, false);
            }
        }
        let emitted = ops
            .iter()
            .filter_map(|op| match op {
                Op::ShowText { items } => Some(items.as_slice()),
                _ => None,
            })
            .flatten()
            .filter_map(|item| match item {
                TextItem::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let anchors = [
            "target-anchor",
            "meaning-anchor",
            "[finance]",
            "[landform]",
            "[reserve]",
            "[row]",
            "[rely]",
            "[aviation]",
            "plans.",
            "container.",
            "final-context-anchor.",
        ]
        .map(|anchor| emitted.contains(anchor));
        assert_eq!(
            (target_lines, meaning_lines, anchors),
            (2, 2, [true; 11]),
            "the dense card did not wrap its fixed rows twice and emit every planned context tail"
        );
    }

    /// An unbroken term or URL stays within a narrow face without losing letters.
    #[test]
    fn long_unbroken_tokens_wrap_without_escaping_the_text_frame() {
        let item = dense_six_sense_entry();
        let mut sheet = CardSheet::new();
        sheet.append(&item, None);
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let view = ClassifierView::from(&fonts);
        let input = "unbrokenword".repeat(43);
        let lines = wrap_runs(&[super::bold_chunk(&input)], 35.0, super::PHRASE_SIZE, view);
        let output = lines
            .iter()
            .flatten()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        assert_eq!(
            (
                output,
                lines
                    .iter()
                    .all(|line| super::measure_runs(view, line, super::PHRASE_SIZE) <= 35.001)
            ),
            (input, true),
            "an unbroken token lost letters or crossed the narrow face width"
        );
    }

    /// Dense reviewed meanings fit one original card without reducing their content.
    #[test]
    fn dense_cards_fit_the_original_size_above_the_readable_floor() {
        let item = dense_six_sense_entry();
        let mut sheet = CardSheet::new();
        sheet.append(&item, None);
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let pages = sheet.pages(&fonts).expect("dense card must fit");
        let card = &pages[0][0];
        let sizes = card
            .faces
            .back
            .iter()
            .filter_map(|row| match &row.content {
                super::RowContent::Text {
                    bullet: Some(_), ..
                } => Some(row.size),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            pages.len() == 1
                && pages[0].len() == 1
                && sizes.iter().all(|size| *size >= super::EXPLAIN_SIZE_MIN)
                && super::rows_height(&card.faces.back) <= super::HALF_H - card.faces.padding * 2.0,
            "a dense card changed size, crossed its inset, or fell below the ordinary type floor"
        );
    }

    /// Dense and short cards share four original rows and the original guides.
    #[test]
    fn dense_cards_keep_four_fixed_rows_and_the_original_cut_guides() {
        let dense = dense_six_sense_entry();
        let mut short = dense.clone();
        short.source.context =
            NonEmptyText::new("A short explanation").expect("context must be valid");
        let mut sheet = CardSheet::new();
        for item in [&short, &dense, &short, &short] {
            sheet.append(item, None);
        }
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let pages = sheet.pages(&fonts).expect("cards must fit");
        let geometry = pages
            .iter()
            .map(|page| page.iter().map(|card| card.card).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut cuts = Vec::new();
        super::draw_cut_lines(&mut cuts, &pages[0]);
        let positions = cuts
            .iter()
            .filter_map(|op| match op {
                Op::DrawLine { line } => Some(line.points[0].p.y.0),
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected = [super::HALF_H * 3.0, super::HALF_H * 2.0, super::HALF_H]
            .map(|y| printpdf::Point::new(printpdf::Mm(0.0), printpdf::Mm(y)).y.0);
        assert_eq!(
            (geometry, positions),
            (vec![vec![0, 1, 2, 3]], expected.to_vec()),
            "dense content changed the four-card row allocation or cut positions"
        );
    }

    /// The fixed front remains the source prompt without answer-bearing labels.
    #[test]
    fn fixed_fronts_dont_disclose_the_target_term() {
        let mut item = dense_six_sense_entry();
        item.term = NonEmptyText::new("secretanswer").expect("term must be valid");
        let mut sheet = CardSheet::new();
        sheet.append(&item, None);
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let mut document = PdfDocument::new("fixed fronts");
        let ids = fonts.register(&mut document);
        let pages = sheet.pages(&fonts).expect("card must fit");
        let mut ops = Vec::new();
        for card in pages.iter().flatten() {
            sheet.draw_face(&mut ops, &fonts, &ids, card, true);
        }
        let emitted = ops
            .iter()
            .filter_map(|op| match op {
                Op::ShowText { items } => Some(items),
                _ => None,
            })
            .flatten()
            .filter_map(|item| match item {
                TextItem::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(
            !emitted.contains("secretanswer") && !emitted.contains("part"),
            "a fixed front disclosed the answer or gained a continuation label"
        );
    }

    /// Each unbounded imported field fails before rendering outside a fixed face.
    #[test]
    fn oversized_fields_are_refused_instead_of_leaving_their_fixed_faces() {
        let rejected = (0..7)
            .filter(|index| {
                let mut item = dense_six_sense_entry();
                let long = NonEmptyText::new("unboundedword ".repeat(900))
                    .expect("long text must be valid");
                match index {
                    0 => item.source.sentence = long,
                    1 => item.source.hint = long,
                    2 => item.target.sentence = long,
                    3 => item.term = long,
                    4 => item.pronunciation = long,
                    5 => item.meaning = long,
                    _ => item.source.context = long,
                }
                let mut sheet = CardSheet::new();
                sheet.append(&item, None);
                let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
                sheet.pages(&fonts).is_err()
            })
            .count();
        assert_eq!(
            rejected, 7,
            "an unbounded field escaped its fixed face or was silently truncated"
        );
    }

    /// Fitting preserves every surviving context character and its emphasis.
    #[test]
    fn fixed_fitting_keeps_every_reviewed_sense_and_usage_character() {
        let item = dense_six_sense_entry();
        let mut sheet = CardSheet::new();
        sheet.append(&item, None);
        let fonts = sheet.prepare_fonts().expect("sheet fonts must resolve");
        let plan = CardPlan::build(&item);
        let layout = super::CardLayout::build(&plan, &fonts).expect("card must fit");
        let expected = plan
            .explanation
            .iter()
            .flat_map(|block| match block {
                Block::Paragraph(chunks) | Block::Bullet { chunks, .. } => chunks,
            })
            .flat_map(|chunk| {
                chunk
                    .text
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .map(|character| (character, chunk.bold, chunk.italic))
            })
            .collect::<Vec<_>>();
        let start = layout
            .back
            .iter()
            .position(|row| matches!(row.content, super::RowContent::Importance(_)))
            .expect("importance must precede context")
            + 1;
        let actual = layout.back[start..]
            .iter()
            .flat_map(|row| match &row.content {
                super::RowContent::Text { chunks, .. } => chunks.as_slice(),
                _ => &[],
            })
            .flat_map(|chunk| {
                chunk
                    .text
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .map(|character| (character, chunk.bold, chunk.italic))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "fixed fitting dropped, reordered, or restyled a reviewed sense or usage explanation"
        );
    }

    #[test]
    fn korean_wrap_preserves_explicit_word_spaces() {
        let palette = FontPalette::default();
        let parsed = ParsedPalette {
            primary_regular: font_arc(palette.primary(), false).expect("primary must resolve"),
            primary_bold: font_arc(palette.primary(), true).expect("primary bold must resolve"),
            cjk_regular: font_arc(palette.cjk(), false).expect("CJK must resolve"),
            cjk_bold: font_arc(palette.cjk(), true).expect("CJK bold must resolve"),
            supplemental_regular: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, false).expect("supplemental must resolve"))
                .collect(),
            supplemental_bold: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, true).expect("supplemental bold must resolve"))
                .collect(),
            fallback: font_arc(palette.fallback(), false).expect("fallback must resolve"),
            mono: Arc::clone(
                &font_arc(palette.primary(), false).expect("test mono substitute must resolve"),
            ),
        };
        let input = "오늘 날씨가 정말 좋아요";
        let output = wrap_runs(
            &[plain_chunk(input)],
            200.0,
            8.0,
            ClassifierView::from(&parsed),
        )
        .into_iter()
        .flatten()
        .map(|chunk| chunk.text)
        .collect::<String>();
        assert_eq!(
            output, input,
            "Korean wrapping removed explicit word spaces"
        );
    }

    /// Thai grapheme break opportunities never manufacture spaces absent from
    /// the original no-space sentence.
    #[test]
    fn thai_wrap_never_inserts_spaces_between_graphemes() {
        let palette = FontPalette::default();
        let parsed = ParsedPalette {
            primary_regular: font_arc(palette.primary(), false).expect("primary must resolve"),
            primary_bold: font_arc(palette.primary(), true).expect("primary bold must resolve"),
            cjk_regular: font_arc(palette.cjk(), false).expect("CJK must resolve"),
            cjk_bold: font_arc(palette.cjk(), true).expect("CJK bold must resolve"),
            supplemental_regular: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, false).expect("supplemental must resolve"))
                .collect(),
            supplemental_bold: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, true).expect("supplemental bold must resolve"))
                .collect(),
            fallback: font_arc(palette.fallback(), false).expect("fallback must resolve"),
            mono: Arc::clone(
                &font_arc(palette.primary(), false).expect("test mono substitute must resolve"),
            ),
        };
        let input = "ภาษาไทยมีสระและวรรณยุกต์";
        let output = wrap_runs(
            &[plain_chunk(input)],
            12.0,
            8.0,
            ClassifierView::from(&parsed),
        )
        .into_iter()
        .flatten()
        .map(|chunk| chunk.text)
        .collect::<String>();
        assert_eq!(
            output, input,
            "Thai wrapping inserted spaces between grapheme clusters"
        );
    }

    #[test]
    fn mixed_thai_wrap_preserves_explicit_script_boundaries() {
        let palette = FontPalette::default();
        let parsed = ParsedPalette {
            primary_regular: font_arc(palette.primary(), false).expect("primary must resolve"),
            primary_bold: font_arc(palette.primary(), true).expect("primary bold must resolve"),
            cjk_regular: font_arc(palette.cjk(), false).expect("CJK must resolve"),
            cjk_bold: font_arc(palette.cjk(), true).expect("CJK bold must resolve"),
            supplemental_regular: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, false).expect("supplemental must resolve"))
                .collect(),
            supplemental_bold: palette
                .supplemental()
                .iter()
                .map(|family| font_arc(family, true).expect("supplemental bold must resolve"))
                .collect(),
            fallback: font_arc(palette.fallback(), false).expect("fallback must resolve"),
            mono: Arc::clone(
                &font_arc(palette.primary(), false).expect("test mono substitute must resolve"),
            ),
        };
        let input = "use ช่วย instead";
        let output = wrap_runs(
            &[plain_chunk(input)],
            200.0,
            8.0,
            ClassifierView::from(&parsed),
        )
        .into_iter()
        .flatten()
        .map(|chunk| chunk.text)
        .collect::<String>();
        assert_eq!(
            output, input,
            "mixed Thai wrapping removed explicit script-boundary spaces"
        );
    }
}
