//! PDF report rendering with system fonts and thumbnails.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow, bail};
use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView};
use printpdf::{
    Color, Line, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point,
    Pt, RawImage, RawImageData, RawImageFormat, Rgb, TextItem, XObjectTransform,
};
use tempfile::TempDir;

use crate::input::NormalizedEntry;
use crate::profile::{FontFamily as ProfileFontFamily, Fonts, Labels, UiLabels};

const HEIGHT: f32 = 297.0;
const IMAGE: f32 = 25.0;
const INDENT: f32 = 40.0;
const LIMIT: f32 = 240.0;
const MARGIN: f32 = 10.0;
const WIDTH: f32 = 210.0 - INDENT - MARGIN;

/// Select one label set for one report entry.
pub trait LabelSource {
    /// Return the label set for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> UiLabels;
}

impl LabelSource for Labels {
    /// Return the label set for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> UiLabels {
        Labels::selected(self, entry)
    }
}

impl LabelSource for UiLabels {
    /// Return the label set for the entry.
    fn selected(&self, _entry: &NormalizedEntry) -> UiLabels {
        self.clone()
    }
}

/// Select one font family for one report entry.
pub trait FontSelector {
    /// Return the font family for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> FontFamily;
}

impl FontSelector for Fonts {
    /// Return the font family for the entry.
    fn selected(&self, entry: &NormalizedEntry) -> FontFamily {
        FontFamily::new(Fonts::selected(self, entry).name())
    }
}

impl FontSelector for FontFamily {
    /// Return the font family for the entry.
    fn selected(&self, _entry: &NormalizedEntry) -> FontFamily {
        self.clone()
    }
}

/// Format one report entry into text rows.
pub trait ReportLayout {
    /// Return the text rows for one report entry.
    fn row(&self, entry: &NormalizedEntry) -> Vec<(String, f32)>;
}

/// Format one vocabulary entry into the frozen report row layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocabularyLayout<L> {
    labels: L,
}

impl Default for VocabularyLayout<Labels> {
    /// Return the default vocabulary layout.
    fn default() -> Self {
        Self {
            labels: Labels::default(),
        }
    }
}

impl<L> VocabularyLayout<L> {
    /// Create one vocabulary layout.
    pub fn new(labels: L) -> Self {
        Self { labels }
    }
}

impl<L> ReportLayout for VocabularyLayout<L>
where
    L: LabelSource,
{
    /// Return the text rows for one report entry.
    fn row(&self, entry: &NormalizedEntry) -> Vec<(String, f32)> {
        let labels = self.labels.selected(entry);
        let mut header = entry.word.clone();
        if !entry.pronunciation.is_empty() {
            header.push_str(format!(" /{}/", entry.pronunciation.trim_matches('/')).as_str());
        }
        header.push_str(format!(" — {}", entry.translation).as_str());
        let mut lines = vec![(header, 11.0)];
        if !entry.example.is_empty() {
            lines.push((entry.example.clone(), 9.0));
        }
        if !entry.sentence.is_empty() {
            lines.push((format!("{}: {}", labels.sentence(), entry.sentence), 9.0));
        }
        if !entry.context.is_empty() {
            lines.push((format!("{}: {}", labels.context(), entry.context), 8.0));
        }
        if !entry.hint.is_empty() {
            lines.push((format!("{}: {}", labels.hint(), entry.hint), 8.0));
        }
        if !entry.importance.is_empty() {
            lines.push((
                format!("{}: {}/10", labels.importance(), entry.importance),
                8.0,
            ));
        }
        lines
    }
}

/// Resolve one system font family to one filesystem path.
#[derive(Clone, Debug, PartialEq)]
pub struct FontPath {
    family: String,
}

impl FontPath {
    /// Create one regular-weight font path resolver.
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            family: family.into(),
        }
    }

    /// Create one bold-weight font path resolver.
    pub fn bold(family: impl Into<String>) -> Self {
        Self::new(format!("{}:Bold", family.into()))
    }

    /// Return the resolved filesystem path for the font.
    pub fn resolved(&self) -> Result<PathBuf> {
        let output = Command::new("fc-match")
            .args(["-f", "%{file}", self.family.as_str()])
            .output()?;
        let path = String::from_utf8(output.stdout)?.trim().to_string();
        if !output.status.success() || path.is_empty() {
            bail!("Font '{}' was not resolved by fc-match", self.family)
        }
        let result = PathBuf::from(path);
        if result.is_file() {
            return Ok(result);
        }
        bail!(
            "Font '{}' was not resolved to a filesystem path",
            self.family
        )
    }
}

/// Resolve regular and bold variants of one system font family.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFamily {
    regular: FontPath,
    bold: FontPath,
}

impl FontFamily {
    /// Create one regular and bold font resolver pair.
    pub fn new(family: impl Into<String>) -> Self {
        let family = family.into();
        Self {
            regular: FontPath::new(family.clone()),
            bold: FontPath::bold(family),
        }
    }

    /// Return the regular font path.
    pub fn regular(&self) -> Result<PathBuf> {
        self.regular.resolved()
    }

    /// Return the bold font path.
    pub fn bold(&self) -> Result<PathBuf> {
        self.bold.resolved()
    }
}

/// Resize one image to the target thumbnail size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumbnail {
    pixels: u32,
}

impl Thumbnail {
    /// Create one thumbnail compressor.
    pub fn new(pixels: u32) -> Self {
        Self { pixels }
    }

    /// Return the compressed JPEG thumbnail path.
    pub fn compressed(&self, source: &Path, directory: &Path) -> Result<PathBuf> {
        let image = image::open(source)?.thumbnail(self.pixels, self.pixels);
        let result = directory.join(format!(
            "thumb_{}",
            source
                .file_name()
                .ok_or_else(|| anyhow!("Image path '{}' has no filename", source.display()))?
                .to_string_lossy()
        ));
        let writer = fs::File::create(&result)?;
        let mut encoder = JpegEncoder::new_with_quality(writer, 60);
        encoder.encode_image(&image)?;
        Ok(result)
    }
}

/// Accumulate report rows and render them into one PDF.
#[derive(Clone, Debug)]
pub struct Report<L, F> {
    font: F,
    layout: L,
    rows: Vec<(NormalizedEntry, Option<PathBuf>)>,
}

/// Hold mutable PDF resources for the active page.
struct PageState<'a> {
    doc: &'a mut PdfDocument,
    ops: &'a mut Vec<Op>,
    fonts: &'a mut BTreeMap<(PathBuf, PathBuf), Pair>,
}

/// Hold thumbnail rendering dependencies for one row.
struct ImageState<'a> {
    directory: &'a Path,
    thumbnail: &'a Thumbnail,
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
    pub fn append(&mut self, entry: &NormalizedEntry, image: Option<PathBuf>) {
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
        let mut doc = PdfDocument::new("Kamishibai Report");
        let mut fonts = BTreeMap::new();
        let mut pages = vec![Vec::new()];
        let thumbs = TempDir::new()?;
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
            };
            let images = ImageState {
                directory: thumbs.path(),
                thumbnail,
            };
            self.row(&mut page, entry, image.as_deref(), &images, &mut y)?;
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

    /// Render one entry onto the active page.
    fn row(
        &self,
        page: &mut PageState<'_>,
        entry: &NormalizedEntry,
        image: Option<&Path>,
        images: &ImageState<'_>,
        y: &mut f32,
    ) -> Result<()> {
        let top = *y;
        let height = HEIGHT;
        let pair = self.fonts(page, entry)?;
        if let Some(path) = image.filter(|path| path.is_file()) {
            let thumb = images.thumbnail.compressed(path, images.directory)?;
            let image = raw(ImageReader::open(&thumb)?
                .with_guessed_format()?
                .decode()?
                .to_rgb8()
                .into())?;
            let scale_x = target(IMAGE, image.width as f32);
            let scale_y = target(IMAGE, image.height as f32);
            let id = page.doc.add_image(&image);
            page.ops.push(Op::UseXobject {
                id,
                transform: XObjectTransform {
                    translate_x: Some(Mm(10.0).into()),
                    translate_y: Some(Mm(height - top - IMAGE).into()),
                    rotate: None,
                    scale_x: Some(scale_x),
                    scale_y: Some(scale_y),
                    dpi: Some(300.0),
                },
            });
        }
        let mut text = top;
        for (index, (line, size)) in self.layout.row(entry).into_iter().enumerate() {
            for part in wrap(line.as_str(), size, WIDTH) {
                let (font, color) = if index == 0 {
                    (pair.bold.clone(), rgb(0, 0, 0))
                } else if size <= 8.0 {
                    (pair.regular.clone(), rgb(120, 120, 120))
                } else {
                    (pair.regular.clone(), rgb(0, 0, 0))
                };
                text += self.text(page, part.as_str(), size, font, color, text)?;
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
                        p: Point::new(Mm(10.0), Mm(height - *y + 4.0)),
                        bezier: false,
                    },
                    printpdf::LinePoint {
                        p: Point::new(Mm(200.0), Mm(height - *y + 4.0)),
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
        Ok(())
    }

    /// Write one text line and return its height increment.
    fn text(
        &self,
        page: &mut PageState<'_>,
        line: &str,
        size: f32,
        font: printpdf::FontId,
        color: Color,
        y: f32,
    ) -> Result<f32> {
        let line_height = size * 0.5;
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
        Ok(line_height)
    }

    /// Return cached or newly registered fonts for one entry.
    fn fonts(&self, page: &mut PageState<'_>, entry: &NormalizedEntry) -> Result<Pair> {
        let family = self.font.selected(entry);
        let regular = family.regular()?;
        let bold = family.bold()?;
        let key = (regular.clone(), bold.clone());
        if let Some(pair) = page.fonts.get(&key) {
            return Ok(pair.clone());
        }
        let regular = parsed(&regular)?;
        let bold = parsed(&bold)?;
        let pair = Pair {
            regular: page.doc.add_font(&regular),
            bold: page.doc.add_font(&bold),
        };
        page.fonts.insert(key, pair.clone());
        Ok(pair)
    }
}

/// Hold one regular and bold PDF font identifier pair.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Pair {
    bold: printpdf::FontId,
    regular: printpdf::FontId,
}

/// Parse one filesystem font file into one PDF font.
fn parsed(path: &Path) -> Result<ParsedFont> {
    let bytes = fs::read(path)?;
    ParsedFont::from_bytes(bytes.as_slice(), 0, &mut Vec::new())
        .ok_or_else(|| anyhow!("Font '{}' could not be parsed", path.display()))
}

/// Convert one decoded image into the raw PDF image representation.
fn raw(image: DynamicImage) -> Result<RawImage> {
    let (width, height) = image.dimensions();
    let data = image.to_rgb8();
    Ok(RawImage {
        pixels: RawImageData::U8(data.into_raw()),
        width: width as usize,
        height: height as usize,
        data_format: RawImageFormat::RGB8,
        tag: Vec::new(),
    })
}

/// Wrap one report line to the target text width.
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let limit = ((width / (size * 0.2)).floor() as usize).max(1);
    if text.chars().count() <= limit {
        return vec![String::from(text)];
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        if end < chars.len() {
            let cut = chars[start..end]
                .iter()
                .rposition(|char| char.is_whitespace())
                .map(|index| start + index)
                .filter(|index| *index > start);
            if let Some(index) = cut {
                end = index;
            }
        }
        let line = chars[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if !line.is_empty() {
            lines.push(line);
        }
        start = if end == start { end + 1 } else { end };
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
    }
    if lines.is_empty() {
        return vec![String::new()];
    }
    lines
}

/// Return one PDF RGB color value.
fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(Rgb::new(
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
        None,
    ))
}

/// Return the scaling factor that fits one image side into the target width.
fn target(mm: f32, pixels: f32) -> f32 {
    if pixels == 0.0 {
        return 1.0;
    }
    let points = mm * 72.0 / 25.4;
    points * 300.0 / (pixels * 72.0)
}

impl From<ProfileFontFamily> for FontFamily {
    /// Create one report font family from one profile font family name.
    fn from(value: ProfileFontFamily) -> Self {
        FontFamily::new(value.name())
    }
}
