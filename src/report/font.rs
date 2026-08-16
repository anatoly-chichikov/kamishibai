use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use printpdf::{Color, ParsedFont, Rgb};
use rust_fontconfig::{FcFontCache, FcPattern, FcWeight, FontSource, OperatingSystem};

use anyhow::{Result, anyhow, bail};

/// Resolve one font family + weight to one filesystem path through the report
/// font cache.
/// The default palette combines platform Latin, CJK, complex-script, and
/// wide-coverage faces, so the binary stays fontless while glyph coverage is
/// verified from each resolved face before it is selected.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FontPath {
    bold: bool,
    family: String,
}

impl FontPath {
    /// Create one regular-weight font path resolver.
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            bold: false,
            family: family.into(),
        }
    }

    /// Create one bold-weight font path resolver.
    pub fn bold(family: impl Into<String>) -> Self {
        Self {
            bold: true,
            family: family.into(),
        }
    }

    /// Return the resolved filesystem path and face index for the font. The
    /// index points into the .ttc collection the resolver hands back (macOS ships
    /// every Helvetica Neue weight inside one .ttc), so the renderer reaches
    /// the correct subface instead of always reading face 0.
    pub fn resolved(&self) -> Result<ResolvedFont> {
        let resolved = self.matched()?;
        let path = self.materialized(resolved.bytes.as_slice())?;
        Ok(ResolvedFont {
            path,
            face_index: u32::try_from(resolved.face_index)
                .map_err(|_| anyhow!("Font '{}' face index does not fit into u32", self.label()))?,
        })
    }

    /// Return the requested font weight.
    fn weight(&self) -> FcWeight {
        if self.bold {
            return FcWeight::Bold;
        }
        FcWeight::Normal
    }

    /// Return the raw bytes and face index for the first matching font query.
    fn matched(&self) -> Result<MatchedFont> {
        for query in self.queries() {
            if let Ok(font) = self.bytes(query) {
                return Ok(font);
            }
        }
        Err(anyhow!(
            "Font '{}' was not resolved by the report font cache or platform fallbacks",
            self.label()
        ))
    }

    /// Return the family queries for the resolver.
    fn queries(&self) -> Vec<String> {
        let mut value = self
            .preferred()
            .iter()
            .map(|item| String::from(*item))
            .collect::<Vec<_>>();
        value.push(self.family.clone());
        for item in self.aliases() {
            if *item != self.family.as_str() && !value.iter().any(|query| query == item) {
                value.push(String::from(*item));
            }
        }
        value.extend(OperatingSystem::current().expand_generic_family("sans-serif", &[]));
        value
    }

    fn preferred(&self) -> &'static [&'static str] {
        match self.family.as_str() {
            "Geeza Pro" => &["گیزا پرو", "गीज़ा प्रो", "جيزة"],
            _ => &[],
        }
    }

    /// Return the platform fallback aliases for the requested family. macOS
    /// names come first, Linux/Windows names follow as a courtesy.
    fn aliases(&self) -> &'static [&'static str] {
        match self.family.as_str() {
            "Arial" => &[
                "Helvetica",
                "Helvetica Neue",
                "Liberation Sans",
                "DejaVu Sans",
            ],
            "Hiragino Sans" => &[
                "Hiragino Sans GB",
                "PingFang SC",
                "STHeiti",
                "Heiti SC",
                "Noto Sans CJK SC",
                "Source Han Sans SC",
                "Microsoft YaHei",
                "SimHei",
            ],
            "Arial Unicode MS" => &[
                "Arial Unicode",
                "Lucida Sans Unicode",
                "Noto Sans CJK SC",
                "Noto Sans CJK JP",
                "Noto Sans CJK KR",
                "Noto Sans",
                "DejaVu Sans",
                "Liberation Sans",
                "Segoe UI Symbol",
            ],
            "AppleGothic" => &[
                "Apple SD Gothic Neo",
                "Apple SD Gothic Neo Regular",
                "Noto Sans CJK JP",
                "Noto Sans CJK KR",
                "Noto Sans KR",
                "Malgun Gothic",
            ],
            "Geeza Pro" => &["Noto Sans Arabic", "Arial", "Tahoma"],
            "Arial Hebrew" => &[".SF Hebrew", "Noto Sans Hebrew", "Arial", "Tahoma"],
            "Kohinoor Devanagari" => &["Devanagari Sangam MN", "Noto Sans Devanagari", "Mangal"],
            "Thonburi" => &["Noto Sans Thai", "Leelawadee UI", "Tahoma"],
            "Courier New" => &[
                "CourierNewPSMT",
                "Courier",
                "Liberation Mono",
                "DejaVu Sans Mono",
                "Consolas",
            ],
            _ => &[],
        }
    }

    /// Return the raw bytes and face index for one family query.
    fn bytes(&self, query: String) -> Result<MatchedFont> {
        let pattern = FcPattern {
            family: Some(query),
            weight: self.weight(),
            ..Default::default()
        };
        let mut trace = Vec::new();
        let font = FONT_SOURCE_CACHE
            .query(&pattern, &mut trace)
            .ok_or_else(|| {
                anyhow!(
                    "Font '{}' was not resolved by the report font cache",
                    self.label()
                )
            })?;
        let source = FONT_SOURCE_CACHE
            .get_font_by_id(&font.id)
            .ok_or_else(|| anyhow!("Font '{}' resolved to a missing font id", self.label()))?;
        let face_index = match source {
            FontSource::Disk(path) => path.font_index,
            FontSource::Memory(font) => font.font_index,
        };
        let bytes = FONT_SOURCE_CACHE
            .get_font_bytes(&font.id)
            .ok_or_else(|| anyhow!("Font '{}' could not expose raw font data", self.label()))?;
        Ok(MatchedFont { bytes, face_index })
    }

    /// Persist the resolved font bytes to one reusable cache path.
    fn materialized(&self, bytes: &[u8]) -> Result<PathBuf> {
        let path = std::env::temp_dir()
            .join("kamishibai-fonts")
            .join(format!("{:x}.font", md5::compute(bytes)));
        if !path.exists() {
            fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| anyhow!("Font cache path '{}' has no parent", path.display()))?,
            )?;
            fs::write(&path, bytes)?;
        }
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "Font '{}' was not materialized into a filesystem path",
            self.label()
        )
    }

    /// Return the report label for the font variant.
    fn label(&self) -> String {
        if self.bold {
            return format!("{} bold", self.family);
        }
        self.family.clone()
    }
}

/// Bytes plus the .ttc face index for one resolved font query.
struct MatchedFont {
    bytes: Vec<u8>,
    face_index: usize,
}

/// One resolved font: a filesystem path the renderer can hand to printpdf,
/// plus the face index into a .ttc collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFont {
    pub path: PathBuf,
    pub face_index: u32,
}

/// Resolve regular and bold variants of one system font family.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

    /// Return the configured family name.
    pub fn name(&self) -> &str {
        self.regular.family.as_str()
    }

    /// Return the regular font path.
    pub fn regular(&self) -> Result<ResolvedFont> {
        self.regular.resolved()
    }

    /// Return the bold font path.
    pub fn bold(&self) -> Result<ResolvedFont> {
        self.bold.resolved()
    }
}

/// Glyph-coverage palette: primary Latin/Greek/Cyrillic, CJK, dedicated
/// complex-script faces, then one wide-coverage fallback. Dedicated faces are
/// tried for their own scripts before the general tracks because merely
/// carrying an Arabic, Devanagari, Thai, Hebrew, or Hangul codepoint does not
/// prove that a font has the shaping tables needed to render it readably.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontPalette {
    primary: FontFamily,
    cjk: FontFamily,
    supplemental: Vec<FontFamily>,
    fallback: FontFamily,
}

impl Default for FontPalette {
    /// Return the default palette: macOS-shipped Arial (separate Regular and
    /// Bold .ttf files, so the .ttc-subface issue does not apply), Hiragino
    /// Sans (W3 and W6 in separate .ttc subfaces) for CJK, and Arial Unicode
    /// MS as the wide-coverage fallback. Hiragino Sans GB used to be the CJK
    /// pick but font-kit resolves its bold to the same face as its regular
    /// on macOS, so bold CJK never actually rendered bold.
    fn default() -> Self {
        Self {
            primary: FontFamily::new("Arial"),
            cjk: FontFamily::new("Hiragino Sans"),
            supplemental: vec![
                FontFamily::new("AppleGothic"),
                FontFamily::new("Geeza Pro"),
                FontFamily::new("Arial Hebrew"),
                FontFamily::new("Kohinoor Devanagari"),
                FontFamily::new("Thonburi"),
            ],
            fallback: FontFamily::new("Arial Unicode MS"),
        }
    }
}

impl FontPalette {
    /// Create one explicit palette.
    pub fn new(primary: FontFamily, cjk: FontFamily, fallback: FontFamily) -> Self {
        Self {
            primary,
            cjk,
            supplemental: Vec::new(),
            fallback,
        }
    }

    /// Return the primary (Latin/Greek/Cyrillic) family.
    pub fn primary(&self) -> &FontFamily {
        &self.primary
    }

    /// Return the CJK family.
    pub fn cjk(&self) -> &FontFamily {
        &self.cjk
    }

    /// Return dedicated script families in their stable dispatch order.
    pub fn supplemental(&self) -> &[FontFamily] {
        self.supplemental.as_slice()
    }

    /// Return the wide-coverage fallback family.
    pub fn fallback(&self) -> &FontFamily {
        &self.fallback
    }
}

/// Cache key for one parsed font slot: family name plus weight.
type FontKey = (String, bool);

/// Process-wide cache of fully parsed PDF fonts. The system font files do not
/// change between invocations within a process, and parsing the largest face
/// (Arial Unicode MS, 23 MB) costs ~100 ms in release. The cache lets a second
/// report.save() in the same session pay zero parse cost.
static FONT_PARSE_CACHE: LazyLock<Mutex<HashMap<FontKey, Arc<ParsedFont>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Process-wide cache of system font metadata and paths.
static FONT_SOURCE_CACHE: LazyLock<FcFontCache> = LazyLock::new(FcFontCache::build);

/// Eagerly parse the default report palette on a background thread.
///
/// The configured system palette is parsed in the background. Calling this
/// when the TUI starts lets primary, CJK, complex-script, and fallback faces
/// warm while the user reviews cards, so `Report::save()` later finds them in
/// the process-wide cache and skips the parse phase entirely.
/// The thread is detached: failures land in the cache as a miss so the next
/// real call retries normally.
pub fn warm_fonts_async() {
    std::thread::spawn(|| {
        let palette = FontPalette::default();
        let _ = font_arc(palette.primary(), false);
        let _ = font_arc(palette.primary(), true);
        let _ = font_arc(palette.cjk(), false);
        let _ = font_arc(palette.cjk(), true);
        for family in palette.supplemental() {
            let _ = font_arc(family, false);
            let _ = font_arc(family, true);
        }
        let _ = font_arc(palette.fallback(), false);
    });
}

/// Return the parsed font for one family + weight, cached process-wide.
///
/// The first call asks the report font cache for the system match and parses the bytes
/// directly — bypassing the on-disk materialization step that the public
/// `FontPath::resolved()` round-tripped through. Later calls return the
/// cached `Arc` instantly.
pub(super) fn font_arc(family: &FontFamily, bold: bool) -> Result<Arc<ParsedFont>> {
    let key: FontKey = (family.name().to_string(), bold);
    if let Some(cached) = FONT_PARSE_CACHE
        .lock()
        .expect("font cache mutex must not be poisoned")
        .get(&key)
        .cloned()
    {
        return Ok(cached);
    }
    let path = if bold { &family.bold } else { &family.regular };
    let matched = path.matched()?;
    let font = ParsedFont::from_bytes(
        matched.bytes.as_slice(),
        matched.face_index,
        &mut Vec::new(),
    )
    .ok_or_else(|| anyhow!("Font '{}' could not be parsed", path.label()))?;
    let arc = Arc::new(font);
    let cached = FONT_PARSE_CACHE
        .lock()
        .expect("font cache mutex must not be poisoned")
        .entry(key)
        .or_insert(arc)
        .clone();
    Ok(cached)
}

/// Return a complex-script face that PDF readers can address by its shaped glyph ids.
///
/// A PDF font stream cannot select a nonzero face from an embedded TrueType
/// collection. When a requested bold face lives later in a collection, use
/// the family's regular face-zero font and preserve correct shaping instead
/// of emitting the bold face's glyph ids against the wrong outlines.
pub(super) fn shaping_font_arc(family: &FontFamily, bold: bool) -> Result<Arc<ParsedFont>> {
    let requested = font_arc(family, bold)?;
    if requested.original_index == 0 {
        return Ok(requested);
    }
    let regular = font_arc(family, false)?;
    if regular.original_index == 0 {
        return Ok(regular);
    }
    bail!(
        "Font '{}' has no face-zero variant safe for shaped PDF embedding",
        family.name()
    )
}

/// Return whether the parsed font carries a real (non-notdef) glyph for the
/// character.
pub(super) fn carries(font: &ParsedFont, ch: char) -> bool {
    font.lookup_glyph_index(ch as u32)
        .is_some_and(|gid| gid != 0)
}

/// Return the leading for one point size in millimeters.
pub(super) fn leading(size: f32) -> f32 {
    size * 1.2 * 25.4 / 72.0
}

/// Return whether the character belongs to a script that does not separate
/// words with spaces — Hiragana, Katakana, Hangul, and the CJK ideographic
/// ranges. Treated as line-break candidates per character.
pub(super) fn is_cjk(ch: char) -> bool {
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

/// Return one PDF RGB color value.
pub(super) fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(Rgb::new(
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
        None,
    ))
}

/// Return the scaling factor that fits one image side into the target width.
pub(super) fn target(mm: f32, pixels: f32) -> f32 {
    if pixels == 0.0 {
        return 1.0;
    }
    let points = mm * 72.0 / 25.4;
    points * 300.0 / (pixels * 72.0)
}

#[cfg(test)]
mod tests {
    use super::{FontPalette, carries, font_arc, shaping_font_arc};

    /// Regression for the Hiragino Sans GB pitfall: font-kit used to resolve
    /// the bold weight to the exact same .ttc subface as the regular weight,
    /// so bold CJK never actually rendered bold. The default CJK family must
    /// expose distinct files (or distinct face indices) for the two weights.
    #[test]
    fn cjk_bold_resolves_to_a_distinct_face_from_cjk_regular() {
        let palette = FontPalette::default();
        let regular = palette
            .cjk()
            .regular()
            .expect("CJK regular must resolve on the test platform");
        let bold = palette
            .cjk()
            .bold()
            .expect("CJK bold must resolve on the test platform");
        let same_face = regular.path == bold.path && regular.face_index == bold.face_index;
        assert!(
            !same_face,
            "CJK bold ({bold:?}) collapsed onto the CJK regular face ({regular:?}) — bold weight will silently render as regular"
        );
    }

    /// The default supplemental faces are accepted only when their parsed
    /// cmaps carry a representative glyph from the intended script.
    #[test]
    fn supplemental_fonts_cover_every_complex_script() {
        let palette = FontPalette::default();
        let samples = ['한', 'ش', 'ש', 'क', 'ก'];
        let covered = palette
            .supplemental()
            .iter()
            .zip(samples)
            .all(|(family, sample)| {
                font_arc(family, false).is_ok_and(|font| carries(font.as_ref(), sample))
            });
        assert!(
            covered,
            "a default supplemental font resolved without actual glyph coverage"
        );
    }

    /// Shaped glyph ids must always be emitted against the collection face
    /// that a PDF reader actually embeds.
    #[test]
    fn supplemental_shaping_weights_resolve_to_pdf_addressable_faces() {
        let palette = FontPalette::default();
        let resolved = palette
            .supplemental()
            .iter()
            .map(|family| {
                (
                    family.name(),
                    font_arc(family, false).map(|font| font.original_index),
                    font_arc(family, true).map(|font| font.original_index),
                    shaping_font_arc(family, false).map(|font| font.original_index),
                    shaping_font_arc(family, true).map(|font| font.original_index),
                )
            })
            .collect::<Vec<_>>();
        let addressable = resolved.iter().all(|(_, _, _, regular, bold)| {
            regular.as_ref().is_ok_and(|index| *index == 0)
                && bold.as_ref().is_ok_and(|index| *index == 0)
        });
        assert!(
            addressable,
            "a supplemental shaping track kept a nonzero collection face that PDF cannot address: {resolved:?}"
        );
    }
}
