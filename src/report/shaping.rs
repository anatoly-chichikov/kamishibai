use std::collections::HashSet;
use std::ops::Range;

use anyhow::{Result, anyhow};
use printpdf::ParsedFont;
use rustybuzz::{Direction, Face, UnicodeBuffer};
use unicode_bidi::{BidiInfo, Level};

use crate::languages::TextDirection;

/// One visually ordered directional run inside a logical line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VisualRun {
    pub range: Range<usize>,
    pub rtl: bool,
}

/// One checked Rustybuzz position that converts losslessly into PDF arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FontUnit(i16);

impl TryFrom<i32> for FontUnit {
    type Error = std::num::TryFromIntError;

    /// Narrow one Rustybuzz position into the exact OpenType font-unit range.
    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        i16::try_from(value).map(Self)
    }
}

impl From<FontUnit> for f32 {
    /// Convert one bounded font unit losslessly for PDF coordinate arithmetic.
    fn from(value: FontUnit) -> Self {
        Self::from(value.0)
    }
}

/// One shaped and positioned glyph in font units.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ShapedGlyph {
    pub gid: u16,
    pub x_advance: FontUnit,
    pub x_offset: FontUnit,
    pub y_offset: FontUnit,
    pub cid: Option<String>,
}

/// Return the paragraph base direction inferred from its first strong
/// character, falling back to LTR for neutral-only text.
pub(super) fn inferred_direction(text: &str) -> TextDirection {
    let bidi = BidiInfo::new(text, None);
    if bidi
        .paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.level.is_rtl())
    {
        return TextDirection::Rtl;
    }
    TextDirection::Ltr
}

/// Resolve one logical line into Unicode Bidirectional Algorithm visual runs.
pub(super) fn visual_runs(text: &str, direction: TextDirection) -> Vec<VisualRun> {
    if text.is_empty() {
        return Vec::new();
    }
    let level = match direction {
        TextDirection::Ltr => Level::ltr(),
        TextDirection::Rtl => Level::rtl(),
    };
    let bidi = BidiInfo::new(text, Some(level));
    let paragraph = bidi
        .paragraphs
        .first()
        .expect("invariant: non-empty text must have one bidi paragraph");
    let (levels, runs) = bidi.visual_runs(paragraph, 0..text.len());
    runs.into_iter()
        .map(|range| VisualRun {
            rtl: levels[range.start].is_rtl(),
            range,
        })
        .collect()
}

/// Shape one homogeneous font run with OpenType GSUB/GPOS and retain cluster
/// text for the PDF ToUnicode map.
pub(super) fn shape(font: &ParsedFont, text: &str, rtl: bool) -> Result<Vec<ShapedGlyph>> {
    shape_with_context(font, text, rtl, "", "")
}

/// Shape one homogeneous run while preserving joining context at styled or
/// fallback-font boundaries.
pub(super) fn shape_with_context(
    font: &ParsedFont,
    text: &str,
    rtl: bool,
    pre_context: &str,
    post_context: &str,
) -> Result<Vec<ShapedGlyph>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let face_index = u32::try_from(font.original_index)
        .map_err(|_| anyhow!("report font face index exceeds the u32 range"))?;
    let face = Face::from_slice(font.original_bytes.as_slice(), face_index)
        .ok_or_else(|| anyhow!("report font could not be opened for shaping"))?;
    let combined = format!("{pre_context}{text}{post_context}");
    let start = pre_context.len();
    let end = start + text.len();
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(combined.as_str());
    buffer.set_direction(if rtl {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    });
    buffer.guess_segment_properties();
    let output = rustybuzz::shape(&face, &[], buffer);
    let mut starts = output
        .glyph_infos()
        .iter()
        .map(|info| usize::try_from(info.cluster).unwrap_or(combined.len()))
        .collect::<Vec<_>>();
    starts.push(combined.len());
    starts.sort_unstable();
    starts.dedup();
    let mut emitted = HashSet::new();
    output
        .glyph_infos()
        .iter()
        .zip(output.glyph_positions())
        .filter(|(info, _)| {
            usize::try_from(info.cluster).is_ok_and(|cluster| (start..end).contains(&cluster))
        })
        .map(|(info, position)| -> Result<ShapedGlyph> {
            let cluster = usize::try_from(info.cluster).unwrap_or(combined.len());
            let cluster_end = starts
                .iter()
                .copied()
                .find(|candidate| *candidate > cluster)
                .unwrap_or(combined.len());
            let cid = emitted.insert(info.cluster).then(|| {
                combined
                    .get(cluster..cluster_end)
                    .unwrap_or_default()
                    .to_string()
            });
            Ok(ShapedGlyph {
                gid: u16::try_from(info.glyph_id)
                    .expect("invariant: rustybuzz glyph ids fit into u16"),
                x_advance: FontUnit::try_from(position.x_advance)
                    .map_err(|_| anyhow!("shaped x advance exceeds the i16 font-unit range"))?,
                x_offset: FontUnit::try_from(position.x_offset)
                    .map_err(|_| anyhow!("shaped x offset exceeds the i16 font-unit range"))?,
                y_offset: FontUnit::try_from(position.y_offset)
                    .map_err(|_| anyhow!("shaped y offset exceeds the i16 font-unit range"))?,
                cid,
            })
        })
        .collect()
}

/// Return the stable supplemental-font slot for one complex-script codepoint.
pub(super) fn supplemental_slot(ch: char) -> Option<usize> {
    match u32::from(ch) {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF => {
            Some(0)
        }
        0x0600..=0x06FF
        | 0x0750..=0x077F
        | 0x0870..=0x089F
        | 0x08A0..=0x08FF
        | 0xFB50..=0xFDFF
        | 0xFE70..=0xFEFF => Some(1),
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Some(2),
        0x0900..=0x097F | 0xA8E0..=0xA8FF => Some(3),
        0x0E00..=0x0E7F => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::languages::TextDirection;

    use super::{FontUnit, shape, shape_with_context, supplemental_slot, visual_runs};
    use crate::report::font::{FontPalette, font_arc};

    /// Mixed Arabic and Latin text follows the Unicode bidi visual run order.
    #[test]
    fn mixed_arabic_and_latin_text_produces_both_visual_directions() {
        let runs = visual_runs("مرحبا 2026 demo", TextDirection::Rtl);
        assert!(
            runs.iter().any(|run| run.rtl) && runs.iter().any(|run| !run.rtl),
            "mixed RTL text no longer resolves into both visual directions"
        );
    }

    /// Every newly supported complex script has its own stable coverage slot.
    #[test]
    fn complex_scripts_have_dedicated_font_slots() {
        assert_eq!(
            [
                supplemental_slot('한'),
                supplemental_slot('ش'),
                supplemental_slot('ש'),
                supplemental_slot('क'),
                supplemental_slot('ก'),
            ],
            [Some(0), Some(1), Some(2), Some(3), Some(4)],
            "a complex script lost its dedicated font dispatch slot"
        );
    }

    /// Rustybuzz positions outside the exact OpenType font-unit range fail fast.
    #[test]
    fn out_of_range_rustybuzz_units_are_rejected() {
        assert_eq!(
            [
                FontUnit::try_from(i32::MIN).is_err(),
                FontUnit::try_from(i32::MAX).is_err(),
                FontUnit::try_from(i32::from(i16::MIN)).is_ok(),
                FontUnit::try_from(i32::from(i16::MAX)).is_ok(),
            ],
            [true, true, true, true],
            "an out-of-range Rustybuzz position bypassed the checked font-unit boundary"
        );
    }

    /// The default Arabic face performs contextual substitution instead of
    /// returning the font's isolated cmap glyphs.
    #[test]
    fn default_arabic_font_shapes_contextual_forms() {
        let palette = FontPalette::default();
        let font = font_arc(&palette.supplemental()[1], false)
            .expect("the default Arabic face must resolve locally");
        let raw = "سلام"
            .chars()
            .filter_map(|ch| font.lookup_glyph_index(u32::from(ch)))
            .collect::<Vec<_>>();
        let shaped = shape(font.as_ref(), "سلام", true)
            .expect("the resolved Arabic face must shape")
            .into_iter()
            .map(|glyph| glyph.gid)
            .collect::<Vec<_>>();
        assert_ne!(
            shaped, raw,
            "Arabic shaping collapsed to isolated Unicode cmap glyphs"
        );
    }

    /// Full prefix/suffix context keeps Arabic joining intact when a styled
    /// boundary lands after a transparent harakat.
    #[test]
    fn arabic_joining_survives_a_harakat_style_boundary() {
        let palette = FontPalette::default();
        let font = font_arc(&palette.supplemental()[1], false)
            .expect("the default Arabic face must resolve locally");
        let contextual = shape_with_context(font.as_ref(), "سَ", true, "", "لام")
            .expect("the first styled Arabic run must shape with context");
        let isolated = shape(font.as_ref(), "سَ", true)
            .expect("the isolated Arabic run must shape for comparison");
        assert_ne!(
            contextual.iter().map(|glyph| glyph.gid).collect::<Vec<_>>(),
            isolated.iter().map(|glyph| glyph.gid).collect::<Vec<_>>(),
            "a transparent harakat boundary severed contextual Arabic joining"
        );
    }
}
