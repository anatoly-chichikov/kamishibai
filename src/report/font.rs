use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Result, anyhow, bail};
use font_kit::family_name::FamilyName as QueryName;
use font_kit::handle::Handle;
use font_kit::properties::{Properties, Weight};
use font_kit::source::SystemSource;
use printpdf::{Color, ParsedFont, Rgb};

/// Resolve one font family + weight to one filesystem path through font-kit.
/// macOS ships Arial (Regular and Bold as separate .ttf files), Hiragino
/// Sans GB, and Arial Unicode MS — these three together cover every script
/// the report uses, so the binary stays fontless and the embed is whatever
/// subsetting trims the system files to.
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
    /// index points into the .ttc collection font-kit hands back (macOS ships
    /// every Helvetica Neue weight inside one .ttc), so the renderer reaches
    /// the correct subface instead of always reading face 0.
    pub fn resolved(&self) -> Result<ResolvedFont> {
        let resolved = self.matched()?;
        let path = self.materialized(resolved.bytes.as_slice())?;
        Ok(ResolvedFont {
            path,
            face_index: resolved.face_index,
        })
    }

    /// Return the matching system-font properties for the resolver.
    fn properties(&self) -> Properties {
        let mut value = Properties::new();
        if self.bold {
            value.weight(Weight::BOLD);
        }
        value
    }

    /// Return the raw bytes and face index for the first matching font query.
    fn matched(&self) -> Result<MatchedFont> {
        for query in self.queries() {
            if let Ok(font) = self.bytes(query) {
                return Ok(font);
            }
        }
        Err(anyhow!(
            "Font '{}' was not resolved by font-kit or platform fallbacks",
            self.label()
        ))
    }

    /// Return the font-kit family queries for the resolver.
    fn queries(&self) -> Vec<QueryName> {
        let mut value = vec![QueryName::Title(self.family.clone())];
        for item in self.aliases() {
            if *item != self.family.as_str() {
                value.push(QueryName::Title(String::from(*item)));
            }
        }
        value.push(QueryName::SansSerif);
        value
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
            "Hiragino Sans GB" => &[
                "PingFang SC",
                "STHeiti",
                "Heiti SC",
                "Noto Sans CJK SC",
                "Source Han Sans SC",
                "Microsoft YaHei",
                "SimHei",
            ],
            "Arial Unicode MS" => &["Arial Unicode", "Lucida Sans Unicode"],
            _ => &[],
        }
    }

    /// Return the raw bytes and face index for one font-kit family query.
    fn bytes(&self, query: QueryName) -> Result<MatchedFont> {
        let handle = SystemSource::new()
            .select_best_match(&[query], &self.properties())
            .map_err(|_| anyhow!("Font '{}' was not resolved by font-kit", self.label()))?;
        let face_index = match &handle {
            Handle::Path { font_index, .. } => *font_index,
            Handle::Memory { font_index, .. } => *font_index,
        };
        let font = handle.load().map_err(|_| {
            anyhow!(
                "Font '{}' could not be loaded from the system source",
                self.label()
            )
        })?;
        let bytes = font
            .copy_font_data()
            .ok_or_else(|| anyhow!("Font '{}' could not expose raw font data", self.label()))?
            .as_slice()
            .to_vec();
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
    face_index: u32,
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

/// Three-track palette: primary Latin/Greek/Cyrillic (Arial), CJK (Hiragino
/// Sans GB), and a wide-coverage fallback (Arial Unicode MS) for glyphs the
/// primary or CJK weight is missing — IPA in bold is the canonical example.
/// The renderer routes each glyph at the current weight to the first track
/// that carries it, so a single line can mix scripts and weights without
/// dropping glyphs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontPalette {
    primary: FontFamily,
    cjk: FontFamily,
    fallback: FontFamily,
}

impl Default for FontPalette {
    /// Return the default palette: macOS-shipped Arial (separate Regular and
    /// Bold .ttf files, so the .ttc-subface issue does not apply), Hiragino
    /// Sans GB for CJK, and Arial Unicode MS as the wide-coverage fallback.
    fn default() -> Self {
        Self {
            primary: FontFamily::new("Arial"),
            cjk: FontFamily::new("Hiragino Sans GB"),
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

/// Eagerly parse the default report palette on a background thread.
///
/// The five system fonts the report embeds (Arial regular + bold, Hiragino
/// Sans GB regular + bold, Arial Unicode MS) cost ~100 ms parallel and ~220 ms
/// serial in release. Calling this when the TUI starts means the parses run
/// while the user is reviewing cards, so `Report::save()` later finds every
/// face already in the process-wide cache and skips the parse phase entirely.
/// The thread is detached: failures land in the cache as a miss so the next
/// real call retries normally.
pub fn warm_fonts_async() {
    std::thread::spawn(|| {
        let palette = FontPalette::default();
        let _ = font_arc(palette.primary(), false);
        let _ = font_arc(palette.primary(), true);
        let _ = font_arc(palette.cjk(), false);
        let _ = font_arc(palette.cjk(), true);
        let _ = font_arc(palette.fallback(), false);
    });
}

/// Return the parsed font for one family + weight, cached process-wide.
///
/// The first call asks font-kit for the system match and parses the bytes
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
        matched.face_index as usize,
        &mut Vec::new(),
    )
    .ok_or_else(|| anyhow!("Font '{}' could not be parsed", path.label()))?;
    let arc = Arc::new(font);
    FONT_PARSE_CACHE
        .lock()
        .expect("font cache mutex must not be poisoned")
        .entry(key)
        .or_insert_with(|| arc.clone());
    Ok(arc)
}

/// Return whether the parsed font carries a real (non-notdef) glyph for the
/// character.
pub(super) fn carries(font: &ParsedFont, ch: char) -> bool {
    font.lookup_glyph_index(ch as u32)
        .is_some_and(|gid| gid != 0)
}

/// Measure one text span in millimeters using a primary + CJK + fallback
/// chain at the current weight. Each codepoint is measured against the first
/// track that carries it so wrap decisions match what the renderer emits.
pub(super) fn measure(
    primary: &ParsedFont,
    cjk: &ParsedFont,
    fallback: &ParsedFont,
    text: &str,
    size: f32,
) -> f32 {
    let mut total = 0.0_f32;
    for ch in text.chars() {
        let font = if carries(primary, ch) {
            primary
        } else if carries(cjk, ch) {
            cjk
        } else {
            fallback
        };
        let units = f32::from(font.font_metrics.units_per_em).max(1.0);
        let advance = font
            .lookup_glyph_index(ch as u32)
            .map(|gid| f32::from(font.get_horizontal_advance(gid)))
            .unwrap_or(units * 0.5);
        total += advance / units;
    }
    total * size * 25.4 / 72.0
}

/// Return the leading for one point size in millimeters.
pub(super) fn leading(size: f32) -> f32 {
    size * 1.2 * 25.4 / 72.0
}

/// Wrap one report line to fit the target width using the same primary + CJK
/// + fallback chain the renderer uses.
pub(super) fn wrap(
    text: &str,
    size: f32,
    width: f32,
    primary: &ParsedFont,
    cjk: &ParsedFont,
    fallback: &ParsedFont,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            String::from(word)
        } else {
            format!("{current} {word}")
        };
        if measure(primary, cjk, fallback, candidate.as_str(), size) <= width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if measure(primary, cjk, fallback, word, size) <= width {
            current = String::from(word);
            continue;
        }
        let mut head = String::new();
        for ch in word.chars() {
            let mut next = head.clone();
            next.push(ch);
            if measure(primary, cjk, fallback, next.as_str(), size) <= width {
                head = next;
                continue;
            }
            if !head.is_empty() {
                lines.push(std::mem::take(&mut head));
            }
            head.push(ch);
        }
        current = head;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
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
