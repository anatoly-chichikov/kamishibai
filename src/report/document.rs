use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Line, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt,
    RawImage, RawImageData, RawImageFormat, TextItem, XObjectTransform,
};

use crate::vocabulary::VocabularyEntry;

use super::FontFamily;
use super::font::{leading, parsed, rgb, target, wrap};
use super::{FontSelector, ReportLayout, Thumbnail};

const GAP: f32 = 1.0;
const HEIGHT: f32 = 297.0;
const IMAGE: f32 = 25.0;
const INDENT: f32 = 40.0;
const LIMIT: f32 = 240.0;
const MARGIN: f32 = 10.0;
const WIDTH: f32 = 210.0 - INDENT - MARGIN;

/// Accumulate report rows and render them into one PDF.
#[derive(Clone, Debug)]
pub struct Report<L, F> {
    font: F,
    layout: L,
    rows: Vec<(VocabularyEntry, Option<PathBuf>)>,
}

struct PageState<'a> {
    doc: &'a mut PdfDocument,
    ops: &'a mut Vec<Op>,
    fonts: &'a mut BTreeMap<FontFamily, Pair>,
    subsetted: &'a BTreeMap<FontFamily, SubsetFonts>,
}

#[derive(Clone, Debug)]
struct SubsetFonts {
    regular: Rc<ParsedFont>,
    bold: Rc<ParsedFont>,
}

impl<L, F> Report<L, F> {
    /// Create one empty report.
    pub fn new(layout: L, font: F) -> Self {
        Self {
            font,
            layout,
            rows: Vec::new(),
        }
    }

    /// Append one entry and optional image path to the report.
    pub fn append(&mut self, entry: &VocabularyEntry, image: Option<PathBuf>) {
        self.rows.push((entry.clone(), image));
    }
}

impl<L, F> Report<L, F>
where
    L: ReportLayout,
    F: FontSelector,
{
    /// Save the accumulated report to one PDF file.
    pub fn save(&self, output: impl AsRef<Path>, thumbnail: &Thumbnail) -> Result<()> {
        if let Some(parent) = output.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let subsetted = self.prepared_fonts()?;
        let mut doc = PdfDocument::new("Kamishibai Report");
        let mut fonts = BTreeMap::new();
        let mut pages = vec![Vec::new()];
        let mut y = 10.0f32;
        for (entry, image) in &self.rows {
            if y > LIMIT {
                pages.push(Vec::new());
                y = 10.0;
            }
            let mut page = PageState {
                doc: &mut doc,
                ops: pages.last_mut().expect("report must keep one active page"),
                fonts: &mut fonts,
                subsetted: &subsetted,
            };
            self.row(&mut page, entry, image.as_deref(), thumbnail, &mut y)?;
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

    /// Pre-subset every font family to the actual characters its rows use.
    /// Without this, printpdf 0.9.1 embeds the full font (subsetting in that
    /// version is hard-disabled by an `if false &&` guard in serialize.rs:1162),
    /// which on macOS picks Arial Unicode MS (~23 MB per weight) and inflates
    /// PDFs to tens of megabytes.
    fn prepared_fonts(&self) -> Result<BTreeMap<FontFamily, SubsetFonts>> {
        let mut chars_per_family: BTreeMap<FontFamily, HashSet<char>> = BTreeMap::new();
        for (entry, _) in &self.rows {
            let family = self.font.selected(entry);
            let entry_chars = self
                .layout
                .row(entry)
                .into_iter()
                .flat_map(|(line, _)| line.chars().collect::<Vec<_>>())
                .collect::<Vec<_>>();
            chars_per_family
                .entry(family)
                .or_default()
                .extend(entry_chars);
        }
        let mut prepared = BTreeMap::new();
        for (family, chars) in chars_per_family {
            let regular_path = family.regular()?;
            let bold_path = family.bold()?;
            let regular = parsed(&regular_path)?;
            let bold = parsed(&bold_path)?;
            let regular_subset = subset_or_full(&regular, &chars);
            let bold_subset = subset_or_full(&bold, &chars);
            prepared.insert(
                family,
                SubsetFonts {
                    regular: Rc::new(regular_subset),
                    bold: Rc::new(bold_subset),
                },
            );
        }
        Ok(prepared)
    }

    /// Render one entry onto the active page.
    fn row(
        &self,
        page: &mut PageState<'_>,
        entry: &VocabularyEntry,
        image: Option<&Path>,
        thumbnail: &Thumbnail,
        y: &mut f32,
    ) -> Result<()> {
        let top = *y;
        let pair = self.fonts(page, entry)?;
        if let Some(path) = image.filter(|path| path.is_file()) {
            let image = raw(thumbnail.scaled(path)?);
            let scale_x = target(IMAGE, image.width as f32);
            let scale_y = target(IMAGE, image.height as f32);
            let id = page.doc.add_image(&image);
            page.ops.push(Op::UseXobject {
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
            let glyphs = if index == 0 {
                pair.bold_font.as_ref()
            } else {
                pair.regular_font.as_ref()
            };
            for part in wrap(line.as_str(), size, WIDTH, glyphs) {
                let (font, color) = if index == 0 {
                    (pair.bold.clone(), rgb(0, 0, 0))
                } else if size <= 8.0 {
                    (pair.regular.clone(), rgb(120, 120, 120))
                } else {
                    (pair.regular.clone(), rgb(0, 0, 0))
                };
                self.text(page, part.as_str(), size, font, color, text);
                text += leading(size);
            }
        }
        *y = text.max(top + IMAGE) + 7.0;
        page.ops.push(Op::SetOutlineColor {
            col: rgb(200, 200, 200),
        });
        page.ops.push(Op::DrawLine {
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

    /// Write one text line at the target baseline.
    fn text(
        &self,
        page: &mut PageState<'_>,
        line: &str,
        size: f32,
        font: printpdf::FontId,
        color: printpdf::Color,
        y: f32,
    ) {
        page.ops.push(Op::StartTextSection);
        page.ops.push(Op::SetTextCursor {
            pos: Point::new(Mm(INDENT), Mm(HEIGHT - y)),
        });
        page.ops.push(Op::SetFont {
            font: PdfFontHandle::External(font),
            size: Pt(size),
        });
        page.ops.push(Op::SetLineHeight { lh: Pt(size) });
        page.ops.push(Op::SetFillColor { col: color });
        page.ops.push(Op::ShowText {
            items: vec![TextItem::Text(String::from(line))],
        });
        page.ops.push(Op::EndTextSection);
    }

    /// Return cached or newly registered fonts for one entry.
    fn fonts(&self, page: &mut PageState<'_>, entry: &VocabularyEntry) -> Result<Pair> {
        let family = self.font.selected(entry);
        if let Some(pair) = page.fonts.get(&family) {
            return Ok(pair.clone());
        }
        let prepared = page
            .subsetted
            .get(&family)
            .ok_or_else(|| anyhow!("subsetted fonts missing for family {}", family.name()))?;
        let pair = Pair {
            regular: page.doc.add_font(&prepared.regular),
            bold: page.doc.add_font(&prepared.bold),
            regular_font: prepared.regular.clone(),
            bold_font: prepared.bold.clone(),
        };
        page.fonts.insert(family, pair.clone());
        Ok(pair)
    }
}

#[derive(Clone, Debug)]
struct Pair {
    bold: printpdf::FontId,
    bold_font: Rc<ParsedFont>,
    regular: printpdf::FontId,
    regular_font: Rc<ParsedFont>,
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

/// Subset one font down to the supplied character set; falls back to the full
/// font when allsorts cannot subset it (rare but happens for some CFF fonts).
fn subset_or_full(font: &ParsedFont, chars: &HashSet<char>) -> ParsedFont {
    let mut glyph_ids: BTreeMap<u16, char> = BTreeMap::new();
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
