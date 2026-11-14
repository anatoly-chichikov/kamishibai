//! Preserve logical Unicode independently of positioned PDF glyphs.

use std::collections::BTreeMap;
use std::fmt::Write;

use anyhow::{Context, Result, anyhow, bail};
use lopdf::{Dictionary, Document, Object, Stream};
use printpdf::{DictItem, Op, PdfDocument, PdfFontHandle, PdfSaveOptions, TextItem};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Serialize Armenian ligatures with every source letter in their Unicode mapping.
pub(super) fn encoded(source: &PdfDocument, options: &PdfSaveOptions) -> Result<Vec<u8>> {
    let bytes = source.save(options, &mut Vec::new());
    let mappings = mappings(source)?;
    if mappings.is_empty() {
        return Ok(bytes);
    }
    let mut document =
        Document::load_mem(&bytes).context("report PDF text could not be decoded")?;
    let mut replacements = BTreeMap::new();
    for page in document.get_pages().values() {
        for (name, font) in document.get_page_fonts(*page)? {
            let name =
                String::from_utf8(name).context("report PDF font resource name is not UTF-8")?;
            if let Some(mappings) = mappings.get(&name) {
                replacements.insert(font.get(b"ToUnicode")?.as_reference()?, cmap(mappings)?);
            }
        }
    }
    if replacements.len() != mappings.len() {
        bail!("report PDF serialization lost a shaped font Unicode map");
    }
    for (id, content) in replacements {
        document
            .objects
            .insert(id, Object::Stream(Stream::new(Dictionary::new(), content)));
    }
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .context("report PDF Unicode maps could not be saved")?;
    Ok(bytes)
}

/// Keep each visual line's original logical text for selection, search and extraction.
pub(super) fn logical(ops: &mut Vec<Op>, text: &str) {
    let data = [0xFEFFu16]
        .into_iter()
        .chain(text.encode_utf16())
        .flat_map(u16::to_be_bytes)
        .collect();
    ops.push(Op::BeginMarkedContentWithProperties {
        tag: String::from("Span"),
        properties: DictItem::Dict {
            map: BTreeMap::from([(
                String::from("ActualText"),
                DictItem::String {
                    data,
                    literal: false,
                },
            )]),
        },
    });
}

/// Reverse grapheme order and mirror punctuation only in resolved RTL spans.
#[must_use]
pub(super) fn ordered(text: &str, rtl: bool) -> String {
    if !rtl {
        return text.to_string();
    }
    text.graphemes(true)
        .rev()
        .flat_map(str::chars)
        .map(|ch| unicode_bidi_mirroring::get_mirrored(ch).unwrap_or(ch))
        .collect()
}

/// Gather Unicode text under the actual embedded font and glyph ids used by each operation.
fn mappings(document: &PdfDocument) -> Result<BTreeMap<String, BTreeMap<u16, Vec<u16>>>> {
    let mut mappings = BTreeMap::<String, BTreeMap<u16, Vec<u16>>>::new();
    for page in &document.pages {
        let mut selected = None;
        for op in &page.ops {
            match op {
                Op::SetFont {
                    font: PdfFontHandle::External(id),
                    ..
                } => selected = Some(id),
                Op::SetFont { .. } => selected = None,
                Op::ShowText { items } => {
                    let Some(id) = selected else {
                        continue;
                    };
                    let font = &document
                        .resources
                        .fonts
                        .map
                        .get(id)
                        .ok_or_else(|| anyhow!("report text names an unregistered font"))?
                        .parsed_font;
                    for item in items {
                        match item {
                            TextItem::Text(text) => {
                                for ch in text.chars() {
                                    if let Some(gid) = font.lookup_glyph_index(u32::from(ch)) {
                                        mappings
                                            .entry(id.0.clone())
                                            .or_default()
                                            .insert(gid, ch.to_string().encode_utf16().collect());
                                    }
                                }
                            }
                            TextItem::GlyphIds(glyphs) => {
                                for glyph in glyphs {
                                    let text = glyph.cid.clone().or_else(|| {
                                        font.get_glyph_primary_char(glyph.gid).map(String::from)
                                    });
                                    if let Some(text) = text.filter(|text| !text.is_empty()) {
                                        mappings
                                            .entry(id.0.clone())
                                            .or_default()
                                            .insert(glyph.gid, unicode(&text));
                                    }
                                }
                            }
                            TextItem::Offset(_) => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(mappings)
}

/// Keep complete Armenian ligatures and the existing scalar mapping of other scripts.
fn unicode(text: &str) -> Vec<u16> {
    if text.chars().all(|ch| ch.script() == Script::Armenian) {
        return text.encode_utf16().collect();
    }
    text.chars()
        .next()
        .map_or_else(Vec::new, |ch| ch.to_string().encode_utf16().collect())
}

/// Encode bounded bfchar sections with complete UTF-16BE destination sequences.
fn cmap(mappings: &BTreeMap<u16, Vec<u16>>) -> Result<Vec<u8>> {
    let mut text = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /KamishibaiUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries = mappings.iter().collect::<Vec<_>>();
    for chunk in entries.chunks(100) {
        writeln!(text, "{} beginbfchar", chunk.len())?;
        for (code, units) in chunk {
            write!(text, "<{code:04X}> <")?;
            for unit in *units {
                write!(text, "{unit:04X}")?;
            }
            writeln!(text, ">")?;
        }
        writeln!(text, "endbfchar")?;
    }
    text.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::{cmap, encoded, ordered};
    use crate::report::FontPalette;
    use crate::report::font::font_arc;
    use lopdf::{Document, Stream, dictionary};
    use printpdf::{
        Codepoint, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Pt, TextItem,
    };
    use std::collections::BTreeMap;

    /// Plain RTL punctuation mirrors without reversing the marks within a grapheme.
    #[test]
    fn rtl_plain_spans_cannot_separate_a_combining_mark_from_its_base() {
        assert_eq!(
            ordered("[e\u{301}", true),
            "e\u{301}]",
            "RTL reversal misplaced a combining mark or bracket"
        );
    }

    /// Ordinary LTR text must retain its original punctuation and order.
    #[test]
    fn ltr_plain_spans_cannot_mirror_punctuation() {
        assert_eq!(
            ordered("[book]", false),
            "[book]",
            "LTR text was reordered or mirrored"
        );
    }

    /// Destination sequences must retain both ligatures and non-BMP surrogate pairs.
    #[test]
    fn unicode_maps_cannot_shorten_clusters_or_surrogate_pairs() {
        let mut document = Document::new();
        let mappings = BTreeMap::from([(0x12A7, "ու𐀀".encode_utf16().collect())]);
        let stream = document.add_object(Stream::new(
            dictionary! {},
            cmap(&mappings).expect("test map must encode"),
        ));
        let font = dictionary! { "Type" => "Font", "Subtype" => "Type0", "Encoding" => "Identity-H", "ToUnicode" => stream };
        let encoding = font
            .get_font_encoding(&document)
            .expect("test map must parse");
        assert_eq!(
            encoding
                .bytes_to_string(&[0x12, 0xA7])
                .expect("test code must decode"),
            "ու𐀀",
            "a Unicode cluster or surrogate pair was truncated"
        );
    }

    /// A cluster painted as several ordinary glyphs must not repeat its continuation.
    #[test]
    fn non_armenian_multiglyph_clusters_cannot_duplicate_their_continuation() {
        let mut font = font_arc(FontPalette::default().primary(), false)
            .expect("test font must resolve")
            .as_ref()
            .clone();
        let continuation = font
            .lookup_glyph_index(u32::from('B'))
            .expect("test B must exist");
        font.cache_glyph_mapping(continuation, "B");
        let mut source = PdfDocument::new("multi-glyph Unicode");
        let id = source.add_font(&font);
        source.pages.push(PdfPage::new(
            Mm(50.0),
            Mm(50.0),
            vec![
                Op::StartTextSection,
                Op::SetFont {
                    font: PdfFontHandle::External(id),
                    size: Pt(12.0),
                },
                Op::ShowText {
                    items: vec![TextItem::GlyphIds(vec![
                        Codepoint {
                            gid: font
                                .lookup_glyph_index(u32::from('A'))
                                .expect("test A must exist"),
                            offset: 0.0,
                            cid: Some(String::from("AB")),
                        },
                        Codepoint {
                            gid: font
                                .lookup_glyph_index(u32::from('B'))
                                .expect("test B must exist"),
                            offset: 0.0,
                            cid: None,
                        },
                    ])],
                },
                Op::EndTextSection,
            ],
        ));
        let bytes = encoded(&source, &PdfSaveOptions::default()).expect("test PDF must encode");
        let document = Document::load_mem(&bytes).expect("test PDF must parse");
        assert_eq!(
            document
                .extract_text(&[1])
                .expect("test text must decode")
                .trim(),
            "AB",
            "a multi-glyph cluster repeated its continuation"
        );
    }
}
