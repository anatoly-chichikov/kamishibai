//! Match serialized PDF font declarations to their embedded outline format.

use std::collections::BTreeMap;
use std::fmt::Write;

use allsorts::binary::read::ReadScope;
use allsorts::cff::{CFF, Operand, Operator};
use anyhow::{Context, Result, anyhow, bail};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use rustybuzz::ttf_parser::{Face, GlyphId, Tag};

/// Embed CFF outlines with matching character ids while preserving text codes and metrics.
pub(super) fn normalized(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let mut document = Document::load_mem(&bytes).context("report PDF could not be decoded")?;
    let fonts = document
        .objects
        .iter()
        .filter_map(|(id, object)| {
            let font = object.as_dict().ok()?;
            font.get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Type0")
                .then(|| (*id, font.clone()))
        })
        .collect::<Vec<_>>();
    let mut changed = false;
    for (id, mut font) in fonts {
        let mut descendants = font.get(b"DescendantFonts")?.as_array()?.clone();
        let mut corrected = false;
        for child in &mut descendants {
            let mut descendant = document.dereference(child)?.1.as_dict()?.clone();
            if normalize_descendant(&mut document, &mut descendant, &mut font)? {
                replace_dictionary(&mut document, child, descendant);
                corrected = true;
            }
        }
        if corrected {
            font.set("DescendantFonts", descendants);
            document.objects.insert(id, Object::Dictionary(font));
            changed = true;
        }
    }
    if !changed {
        return Ok(bytes);
    }
    let mut output = Vec::new();
    document
        .save_to(&mut output)
        .context("report PDF font declarations could not be saved")?;
    Ok(output)
}

/// Extract a CID-keyed CFF program and map existing PDF glyph codes to its character ids.
fn normalize_descendant(
    document: &mut Document,
    descendant: &mut Dictionary,
    parent: &mut Dictionary,
) -> Result<bool> {
    let descriptor = descendant.get(b"FontDescriptor")?.clone();
    let mut dictionary = document.dereference(&descriptor)?.1.as_dict()?.clone();
    let Ok(file) = dictionary
        .get(b"FontFile3")
        .or_else(|_| dictionary.get(b"FontFile2"))
        .cloned()
    else {
        return Ok(false);
    };
    let stream = document.dereference(&file)?.1.as_stream()?;
    let bytes = stream.get_plain_content()?;
    if !bytes.starts_with(b"OTTO") {
        return Ok(false);
    }
    let stream_id = file.as_reference()?;
    if let Some(program) = CffProgram::from_opentype(&bytes)? {
        let encoding = program.encoding(&format!("KamishibaiGlyphs{}", stream_id.0))?;
        parent.set("Encoding", document.add_object(encoding));
        descendant.set(
            "W",
            remapped_widths(descendant.get(b"W")?.as_array()?, &program.cids)?,
        );
        descendant.set("CIDSystemInfo", program.system.dictionary());
        document.objects.insert(
            stream_id,
            Object::Stream(Stream::new(
                dictionary! { "Subtype" => "CIDFontType0C" },
                program.bytes,
            )),
        );
    } else {
        if dictionary.has(b"FontFile3")
            && stream
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"OpenType")
            && descendant.get(b"Subtype")?.as_name()? == b"CIDFontType0"
        {
            return Ok(false);
        }
        document
            .get_object_mut(stream_id)?
            .as_stream_mut()?
            .dict
            .set("Subtype", "OpenType");
        if matches!(
            document.version.as_str(),
            "1.0" | "1.1" | "1.2" | "1.3" | "1.4" | "1.5"
        ) {
            document.version = String::from("1.6");
        }
    }
    dictionary.remove(b"FontFile2");
    dictionary.set("FontFile3", file);
    let mut descriptor = descriptor;
    replace_dictionary(document, &mut descriptor, dictionary);
    descendant.set("FontDescriptor", descriptor);
    descendant.set("Subtype", "CIDFontType0");
    descendant.remove(b"CIDToGIDMap");
    Ok(true)
}

/// One raw CFF program with its glyph-to-character mapping and collection identity.
struct CffProgram {
    bytes: Vec<u8>,
    cids: Vec<u16>,
    system: CharacterCollection,
}

impl CffProgram {
    /// Read the existing CID-keyed CFF data without recompiling its outlines.
    fn from_opentype(bytes: &[u8]) -> Result<Option<Self>> {
        let face = Face::parse(bytes, 0).context("embedded OpenType font could not be parsed")?;
        if face.tables().cff2.is_some() {
            return Ok(None);
        }
        let table = face
            .tables()
            .cff
            .context("embedded OpenType font has no CFF outlines")?;
        let bytes = face
            .raw_face()
            .table(Tag::from_bytes(b"CFF "))
            .context("embedded OpenType font has no CFF table")?;
        let cff = ReadScope::new(bytes)
            .read::<CFF<'_>>()
            .map_err(|error| anyhow!("embedded CFF collection could not be parsed: {error}"))?;
        let font = cff
            .fonts
            .first()
            .context("embedded CFF collection is empty")?;
        let Some(
            [
                Operand::Integer(registry),
                Operand::Integer(ordering),
                Operand::Integer(supplement),
            ],
        ) = font.top_dict.get(Operator::ROS)
        else {
            return Ok(None);
        };
        let system = CharacterCollection {
            registry: cff
                .read_string(u16::try_from(*registry)?)
                .map_err(|error| anyhow!("embedded CFF registry could not be read: {error}"))?
                .to_string(),
            ordering: cff
                .read_string(u16::try_from(*ordering)?)
                .map_err(|error| anyhow!("embedded CFF ordering could not be read: {error}"))?
                .to_string(),
            supplement: *supplement,
        };
        let cids = (0..face.number_of_glyphs())
            .map(|glyph| {
                table
                    .glyph_cid(GlyphId(glyph))
                    .context("embedded CFF glyph has no character id")
            })
            .collect::<Result<Vec<_>>>()?;
        if cids.iter().collect::<std::collections::BTreeSet<_>>().len() != cids.len() {
            bail!("embedded CFF character ids are not unique");
        }
        Ok(Some(Self {
            bytes: bytes.to_vec(),
            cids,
            system,
        }))
    }

    /// Encode unchanged two-byte text codes into the actual CFF character collection.
    fn encoding(&self, name: &str) -> Result<Stream> {
        let mut text = format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry {} /Ordering {} /Supplement {} >> def\n/CMapName /{name} def\n/CMapType 1 def\n/WMode 0 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
            literal(self.system.registry.as_bytes()),
            literal(self.system.ordering.as_bytes()),
            self.system.supplement
        );
        let mapping = self.cids.iter().enumerate().collect::<Vec<_>>();
        for chunk in mapping.chunks(100) {
            writeln!(&mut text, "{} begincidchar", chunk.len())?;
            for (glyph, cid) in chunk {
                writeln!(&mut text, "<{glyph:04X}> {cid}")?;
            }
            text.push_str("endcidchar\n");
        }
        text.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        Ok(Stream::new(
            dictionary! { "Type" => "CMap", "CMapName" => Object::Name(name.as_bytes().to_vec()), "CIDSystemInfo" => self.system.dictionary(), "WMode" => 0 },
            text.into_bytes(),
        ))
    }
}

/// Registry, ordering and supplement shared by one CFF font and its encoding CMap.
struct CharacterCollection {
    registry: String,
    ordering: String,
    supplement: i32,
}

impl CharacterCollection {
    /// Return the exact character collection declared by the embedded font program.
    fn dictionary(&self) -> Dictionary {
        dictionary! { "Registry" => Object::string_literal(self.registry.as_bytes()), "Ordering" => Object::string_literal(self.ordering.as_bytes()), "Supplement" => self.supplement }
    }
}

/// Reindex existing widths by CID without changing their numeric values.
fn remapped_widths(widths: &[Object], cids: &[u16]) -> Result<Vec<Object>> {
    let mut mapped = BTreeMap::new();
    let mut position = 0;
    while position < widths.len() {
        let start = usize::try_from(widths[position].as_i64()?)?;
        let next = widths
            .get(position + 1)
            .context("embedded font width range is incomplete")?;
        if let Object::Array(values) = next {
            for (offset, width) in values.iter().enumerate() {
                let glyph = start
                    .checked_add(offset)
                    .context("embedded font width index overflowed")?;
                let cid = *cids
                    .get(glyph)
                    .context("embedded font width references an absent glyph")?;
                mapped.insert(cid, width.clone());
            }
            position += 2;
        } else {
            let end = usize::try_from(next.as_i64()?)?;
            if end < start {
                bail!("embedded font width interval is reversed");
            }
            let width = widths
                .get(position + 2)
                .context("embedded font width interval is incomplete")?;
            for glyph in start..=end {
                let cid = *cids
                    .get(glyph)
                    .context("embedded font width references an absent glyph")?;
                mapped.insert(cid, width.clone());
            }
            position += 3;
        }
    }
    Ok(mapped
        .into_iter()
        .flat_map(|(cid, width)| [Object::Integer(i64::from(cid)), Object::Array(vec![width])])
        .collect())
}

/// Encode collection strings using the escaped literal syntax accepted by Quartz CMaps.
fn literal(bytes: &[u8]) -> String {
    let mut text = String::from("(");
    for byte in bytes {
        match *byte {
            b'(' | b')' | b'\\' => {
                text.push('\\');
                text.push(char::from(*byte));
            }
            32..=126 => text.push(char::from(*byte)),
            _ => text.push_str(&format!("\\{byte:03o}")),
        }
    }
    text.push(')');
    text
}

/// Preserve indirect references while replacing one font dictionary's metadata.
fn replace_dictionary(document: &mut Document, object: &mut Object, dictionary: Dictionary) {
    match object {
        Object::Reference(id) => {
            document.objects.insert(*id, Object::Dictionary(dictionary));
        }
        value => *value = Object::Dictionary(dictionary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::font::font_arc;
    use crate::report::{FontFamily, FontPalette};
    use printpdf::{
        Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Pt, TextItem,
    };

    /// Serialize a real CJK font with the upstream writer before boundary normalization.
    fn uncorrected() -> Vec<u8> {
        let family = FontPalette::default();
        font_document(family.cjk(), "意味俳優")
    }

    /// Serialize one subset font as emitted by the upstream writer.
    fn font_document(family: &FontFamily, text: &str) -> Vec<u8> {
        let font = font_arc(family, true).expect("font must resolve");
        let glyphs = text
            .chars()
            .map(|ch| {
                (
                    font.lookup_glyph_index(u32::from(ch))
                        .expect("CJK glyph must exist"),
                    ch,
                )
            })
            .collect();
        let subset = printpdf::subset_font(&font, &glyphs).expect("CJK subset must succeed");
        let font = ParsedFont::from_bytes(&subset.bytes, 0, &mut Vec::new())
            .expect("CJK subset must parse");
        let mut document = PdfDocument::new("Font regression");
        let id = document.add_font(&font);
        document
            .with_pages(vec![PdfPage::new(
                Mm(210.0),
                Mm(297.0),
                vec![
                    Op::StartTextSection,
                    Op::SetFont {
                        font: PdfFontHandle::External(id),
                        size: Pt(12.0),
                    },
                    Op::ShowText {
                        items: vec![TextItem::Text(String::from(text))],
                    },
                    Op::EndTextSection,
                ],
            )])
            .save(&PdfSaveOptions::default(), &mut Vec::new())
    }

    /// Read original page operators and Unicode maps independently of embedding metadata.
    fn protected(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let document = Document::load_mem(bytes).expect("font PDF must parse");
        let pages = document
            .get_pages()
            .values()
            .map(|id| {
                document
                    .get_page_content(*id)
                    .expect("page content must decode")
            })
            .collect();
        let unicode = document
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .filter_map(|font| font.get_deref(b"ToUnicode", &document).ok())
            .map(|object| {
                object
                    .as_stream()
                    .expect("Unicode map must be a stream")
                    .get_plain_content()
                    .expect("Unicode map must decode")
            })
            .collect();
        (pages, unicode)
    }

    #[test]
    fn corrected_fonts_cannot_change_on_a_second_pass() {
        let once = normalized(uncorrected()).expect("font embedding must normalize");
        assert_eq!(
            normalized(once.clone()).expect("repeat must succeed"),
            once,
            "a repeated embedding pass rewrote an already-correct PDF"
        );
    }

    #[test]
    fn cff_embedding_cannot_rewrite_page_text_or_unicode_maps() {
        let original = uncorrected();
        let corrected = normalized(original.clone()).expect("font embedding must normalize");
        assert_eq!(
            protected(&corrected),
            protected(&original),
            "CFF embedding changed page operators or Unicode extraction maps"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn named_cff_outlines_cannot_be_mislabeled_as_cid_keyed_programs() {
        let bytes = normalized(font_document(&FontFamily::new("STIXGeneral"), "Abc"))
            .expect("named CFF embedding must normalize");
        let document = Document::load_mem(&bytes).expect("named CFF PDF must parse");
        let compatible = document
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .any(|stream| {
                stream
                    .dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == b"OpenType")
                    && stream
                        .get_plain_content()
                        .is_ok_and(|data| data.starts_with(b"OTTO"))
            });
        assert!(
            compatible && normalized(bytes.clone()).expect("second pass must succeed") == bytes,
            "a named CFF font lost its OpenType wrapper or changed on a second pass"
        );
    }

    #[test]
    fn cid_widths_cannot_change_the_existing_numeric_advances() {
        let widths = vec![
            Object::Integer(2),
            Object::Array(vec![Object::Integer(317), Object::Integer(829)]),
            Object::Integer(0),
            Object::Integer(1),
            Object::Integer(451),
        ];
        assert_eq!(
            remapped_widths(&widths, &[0, 635, 847, 992]).expect("widths must remap"),
            vec![
                Object::Integer(0),
                Object::Array(vec![Object::Integer(451)]),
                Object::Integer(635),
                Object::Array(vec![Object::Integer(451)]),
                Object::Integer(847),
                Object::Array(vec![Object::Integer(317)]),
                Object::Integer(992),
                Object::Array(vec![Object::Integer(829)])
            ],
            "CID remapping lost an interval or changed an existing advance"
        );
    }

    #[test]
    fn cmap_identity_cannot_use_quartz_incompatible_hex_strings() {
        let program = CffProgram {
            bytes: Vec::new(),
            cids: vec![0],
            system: CharacterCollection {
                registry: String::from("Adobe"),
                ordering: String::from("Japan(test)\\name\n"),
                supplement: 7,
            },
        };
        let stream = program
            .encoding("FontRegression")
            .expect("encoding must serialize");
        let body = String::from_utf8(stream.content).expect("CMap must be text");
        assert!(
            body.contains(
                "/Registry (Adobe) /Ordering (Japan\\(test\\)\\\\name\\012) /Supplement 7"
            ),
            "CMap collection strings use Quartz-incompatible hex syntax or unsafe literal escaping"
        );
    }
}
