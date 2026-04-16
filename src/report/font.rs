use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use font_kit::family_name::FamilyName as QueryName;
use font_kit::properties::{Properties, Weight};
use font_kit::source::SystemSource;
use printpdf::{Color, ParsedFont, Rgb};

use crate::languages::FontFamily as ProfileFontFamily;

/// Resolve one system font family to one filesystem path.
#[derive(Clone, Debug, PartialEq)]
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

    /// Return the resolved filesystem path for the font.
    pub fn resolved(&self) -> Result<PathBuf> {
        let font = SystemSource::new()
            .select_best_match(&[QueryName::Title(self.family.clone())], &self.properties())
            .map_err(|_| anyhow!("Font '{}' was not resolved by font-kit", self.label()))?
            .load()
            .map_err(|_| {
                anyhow!(
                    "Font '{}' could not be loaded from the system source",
                    self.label()
                )
            })?;
        let bytes = font
            .copy_font_data()
            .ok_or_else(|| anyhow!("Font '{}' could not expose raw font data", self.label()))?;
        self.materialized(bytes.as_slice())
    }

    /// Return the matching system-font properties for the resolver.
    fn properties(&self) -> Properties {
        let mut value = Properties::new();
        if self.bold {
            value.weight(Weight::BOLD);
        }
        value
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

/// Parse one filesystem font file into one PDF font.
pub(super) fn parsed(path: &Path) -> Result<ParsedFont> {
    let bytes = fs::read(path)?;
    ParsedFont::from_bytes(bytes.as_slice(), 0, &mut Vec::new())
        .ok_or_else(|| anyhow!("Font '{}' could not be parsed", path.display()))
}

/// Measure one text span in millimeters for one parsed font and point size.
pub(super) fn measure(font: &ParsedFont, text: &str, size: f32) -> f32 {
    let units = f32::from(font.font_metrics.units_per_em).max(1.0);
    let advance: f32 = text
        .chars()
        .map(|ch| {
            font.lookup_glyph_index(ch as u32)
                .map(|gid| f32::from(font.get_horizontal_advance(gid)))
                .unwrap_or_else(|| units * 0.5)
        })
        .sum();
    advance / units * size * 25.4 / 72.0
}

/// Return the leading for one point size in millimeters.
pub(super) fn leading(size: f32) -> f32 {
    size * 1.2 * 25.4 / 72.0
}

/// Wrap one report line to fit the target width using actual glyph advances.
pub(super) fn wrap(text: &str, size: f32, width: f32, font: &ParsedFont) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            String::from(word)
        } else {
            format!("{current} {word}")
        };
        if measure(font, candidate.as_str(), size) <= width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if measure(font, word, size) <= width {
            current = String::from(word);
            continue;
        }
        let mut head = String::new();
        for ch in word.chars() {
            let mut next = head.clone();
            next.push(ch);
            if measure(font, next.as_str(), size) <= width {
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

impl From<ProfileFontFamily> for FontFamily {
    /// Create one report font family from one profile font family name.
    fn from(value: ProfileFontFamily) -> Self {
        FontFamily::new(value.name())
    }
}
