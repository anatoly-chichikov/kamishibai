//! Lightweight markdown layer for Gemini-produced card fields.
//!
//! `parse_markdown` turns `**bold**`, `*italic*`, and `- bullet` lines into a
//! sequence of `Block`s with styled `TextChunk`s. The three renderers project
//! the same AST into the three production consumers:
//!
//! - `to_html` for the Anki note body (uses `<strong>` / `<em>` / `<ul><li>`
//!   to match the existing card template).
//! - `to_ratatui` for the TUI preview (ratatui `Line` + `Span` with BOLD /
//!   ITALIC modifiers, bullets get a `•` glyph prefix and a left indent).
//! - `to_plain` for legacy text reports that cannot render any styling.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// One styled run inside a paragraph or bullet line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextChunk {
    /// Rendered text of the run.
    pub text: String,
    /// Whether the bold weight applies to the run.
    pub bold: bool,
    /// Whether the synthetic italic slant applies to the run.
    pub italic: bool,
}

/// One parsed markdown block: a paragraph or a single-line bullet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    /// Free-flowing paragraph with inline runs.
    Paragraph(Vec<TextChunk>),
    /// Bullet line at a leading indent level (0, 1, or 2).
    Bullet {
        /// Indent level: 0 for `- ` at column 0, 1 for `  - `, 2 for `    - `.
        indent: u8,
        /// Inline runs after the bullet marker.
        chunks: Vec<TextChunk>,
    },
}

/// Parse one markdown blob into blocks for downstream renderers. Lines
/// starting with `- ` or `* ` (after 0..4 leading spaces) become bullets at
/// indent `leading_spaces / 2`, capped at 2. Other non-empty lines group into
/// paragraphs separated by blank lines. Inline markers `**xxx**` (bold) and
/// `*xxx*` (italic) are matched greedily and do not nest; unmatched markers
/// stay literal.
#[must_use]
pub fn parse_markdown(input: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<TextChunk> = Vec::new();
    for raw_line in input.lines() {
        let leading_spaces = raw_line.chars().take_while(|ch| *ch == ' ').count().min(4);
        let trimmed = &raw_line[leading_spaces..];
        if trimmed.is_empty() {
            if !paragraph.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut paragraph)));
            }
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !paragraph.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut paragraph)));
            }
            let indent = (u8::try_from(leading_spaces).unwrap_or(0) / 2).min(2);
            blocks.push(Block::Bullet {
                indent,
                chunks: parse_inline(rest),
            });
            continue;
        }
        let mut chunks = parse_inline(trimmed);
        if !paragraph.is_empty() {
            paragraph.push(TextChunk {
                text: String::from(" "),
                bold: false,
                italic: false,
            });
        }
        paragraph.append(&mut chunks);
    }
    if !paragraph.is_empty() {
        blocks.push(Block::Paragraph(paragraph));
    }
    blocks
}

/// Render markdown blocks into an Anki-flavoured HTML string. Paragraphs land
/// inside `<p>` so consecutive sections get the browser's default paragraph
/// margin and never visually collapse into a wall of text. Consecutive
/// bullets fold into one `<ul>` list, and inline weights use `<strong>` /
/// `<em>` to match the existing sentence-highlight markup in
/// `src/anki/format.rs`. The inline margin styles are spelled out verbatim
/// because Anki's web view strips most class-based CSS.
#[must_use]
pub fn to_html(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for block in blocks {
        match block {
            Block::Bullet { chunks, .. } => {
                if !in_list {
                    out.push_str("<ul style=\"margin: 0.4em 0; padding-left: 1.2em;\">");
                    in_list = true;
                }
                out.push_str("<li>");
                push_html_chunks(&mut out, chunks);
                out.push_str("</li>");
            }
            Block::Paragraph(chunks) => {
                if in_list {
                    out.push_str("</ul>");
                    in_list = false;
                }
                out.push_str("<p style=\"margin: 0.4em 0;\">");
                push_html_chunks(&mut out, chunks);
                out.push_str("</p>");
            }
        }
    }
    if in_list {
        out.push_str("</ul>");
    }
    out
}

/// Render markdown blocks into ratatui `Line`s for TUI preview. One line per
/// block — the calling widget should enable word-wrap. Bullets prepend a
/// `• ` glyph and a two-space-per-level left indent.
#[must_use]
pub fn to_ratatui(blocks: &[Block]) -> Vec<Line<'static>> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(chunks) => paragraph_line(chunks),
            Block::Bullet { indent, chunks } => bullet_line(*indent, chunks),
        })
        .collect()
}

/// Render markdown blocks back into plain text, stripping every styling
/// marker. Bullets keep their `- ` prefix and indent so the output stays
/// readable in legacy CSV / log sinks.
#[must_use]
pub fn to_plain(blocks: &[Block]) -> String {
    let mut out = String::new();
    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        match block {
            Block::Paragraph(chunks) => {
                for chunk in chunks {
                    out.push_str(&chunk.text);
                }
            }
            Block::Bullet { indent, chunks } => {
                for _ in 0..*indent {
                    out.push_str("  ");
                }
                out.push_str("- ");
                for chunk in chunks {
                    out.push_str(&chunk.text);
                }
            }
        }
    }
    out
}

/// Split one inline string into bold/italic runs. `**xxx**` matches first
/// (greedy, non-nested), then `*xxx*`. Unmatched markers stay literal.
fn parse_inline(text: &str) -> Vec<TextChunk> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' {
            if i + 1 < chars.len() && chars[i + 1] == '*' {
                if let Some(end) = find_marker(&chars, i + 2, &['*', '*']) {
                    flush(&mut chunks, &mut buffer);
                    chunks.push(TextChunk {
                        text: chars[i + 2..end].iter().collect(),
                        bold: true,
                        italic: false,
                    });
                    i = end + 2;
                    continue;
                }
            } else if let Some(end) = find_marker(&chars, i + 1, &['*'])
                && end > i + 1
            {
                flush(&mut chunks, &mut buffer);
                chunks.push(TextChunk {
                    text: chars[i + 1..end].iter().collect(),
                    bold: false,
                    italic: true,
                });
                i = end + 1;
                continue;
            }
        }
        buffer.push(chars[i]);
        i += 1;
    }
    flush(&mut chunks, &mut buffer);
    chunks
}

/// Drain `buffer` into one plain `TextChunk` when it has anything to emit.
fn flush(chunks: &mut Vec<TextChunk>, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    chunks.push(TextChunk {
        text: std::mem::take(buffer),
        bold: false,
        italic: false,
    });
}

/// Return the index of the next occurrence of `marker` in `chars` starting at
/// `from`, or `None` if it is not present.
fn find_marker(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    if marker.is_empty() || chars.len() < marker.len() {
        return None;
    }
    let mut i = from;
    while i + marker.len() <= chars.len() {
        if chars[i..i + marker.len()] == *marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Render one paragraph block into a ratatui `Line` of styled spans.
fn paragraph_line(chunks: &[TextChunk]) -> Line<'static> {
    Line::from(chunks.iter().map(chunk_span).collect::<Vec<_>>())
}

/// Render one bullet block into a ratatui `Line`: indent + `• ` glyph +
/// styled spans.
fn bullet_line(indent: u8, chunks: &[TextChunk]) -> Line<'static> {
    let prefix = format!("{}• ", "  ".repeat(indent as usize));
    let mut spans = Vec::with_capacity(chunks.len() + 1);
    spans.push(Span::raw(prefix));
    spans.extend(chunks.iter().map(chunk_span));
    Line::from(spans)
}

/// Build one ratatui `Span` for a single styled chunk.
fn chunk_span(chunk: &TextChunk) -> Span<'static> {
    let mut style = Style::default();
    if chunk.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if chunk.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    Span::styled(chunk.text.clone(), style)
}

/// Emit one chunk into an HTML buffer with the matching opening / closing
/// inline tag pair and HTML-escaped text.
fn push_html_chunks(out: &mut String, chunks: &[TextChunk]) {
    for chunk in chunks {
        let (open, close) = match (chunk.bold, chunk.italic) {
            (true, true) => ("<strong><em>", "</em></strong>"),
            (true, false) => ("<strong>", "</strong>"),
            (false, true) => ("<em>", "</em>"),
            (false, false) => ("", ""),
        };
        out.push_str(open);
        push_html_escape(out, &chunk.text);
        out.push_str(close);
    }
}

/// Append `text` to `out` with the four HTML-special characters escaped.
fn push_html_escape(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, TextChunk, parse_markdown, to_html, to_plain, to_ratatui};
    use ratatui::style::Modifier;

    fn plain(text: &str) -> TextChunk {
        TextChunk {
            text: String::from(text),
            bold: false,
            italic: false,
        }
    }

    fn bold(text: &str) -> TextChunk {
        TextChunk {
            text: String::from(text),
            bold: true,
            italic: false,
        }
    }

    fn italic(text: &str) -> TextChunk {
        TextChunk {
            text: String::from(text),
            bold: false,
            italic: true,
        }
    }

    #[test]
    fn a_plain_line_becomes_one_paragraph_with_one_plain_chunk() {
        assert_eq!(
            parse_markdown("hello world"),
            vec![Block::Paragraph(vec![plain("hello world")])],
            "a plain line did not become a single plain paragraph"
        );
    }

    #[test]
    fn a_bold_run_inside_a_paragraph_becomes_a_bold_chunk() {
        assert_eq!(
            parse_markdown("**Регистр:** formal"),
            vec![Block::Paragraph(vec![bold("Регистр:"), plain(" formal")])],
            "a **bold** run did not parse into a bold chunk"
        );
    }

    #[test]
    fn an_italic_run_inside_a_paragraph_becomes_an_italic_chunk() {
        assert_eq!(
            parse_markdown("see *harbor* now"),
            vec![Block::Paragraph(vec![
                plain("see "),
                italic("harbor"),
                plain(" now"),
            ])],
            "an *italic* run did not parse into an italic chunk"
        );
    }

    #[test]
    fn a_hyphen_line_becomes_one_bullet_at_indent_zero() {
        assert_eq!(
            parse_markdown("- one bullet"),
            vec![Block::Bullet {
                indent: 0,
                chunks: vec![plain("one bullet")],
            }],
            "a '- ' line did not become a level-0 bullet"
        );
    }

    #[test]
    fn a_double_space_indent_becomes_one_bullet_at_indent_one() {
        assert_eq!(
            parse_markdown("  - nested"),
            vec![Block::Bullet {
                indent: 1,
                chunks: vec![plain("nested")],
            }],
            "a two-space indented bullet did not land at level 1"
        );
    }

    #[test]
    fn a_blank_line_separates_two_paragraphs() {
        assert_eq!(
            parse_markdown("first line\n\nsecond line"),
            vec![
                Block::Paragraph(vec![plain("first line")]),
                Block::Paragraph(vec![plain("second line")]),
            ],
            "a blank line did not split two paragraphs"
        );
    }

    #[test]
    fn an_unclosed_italic_marker_stays_literal() {
        assert_eq!(
            parse_markdown("an *unclosed italic"),
            vec![Block::Paragraph(vec![plain("an *unclosed italic")])],
            "an unclosed italic marker did not stay literal"
        );
    }

    #[test]
    fn to_html_wraps_paragraphs_and_bullets_into_the_anki_template_shape() {
        let blocks = parse_markdown("**Meaning.**\n- one\n- two\n\nbody text");
        assert_eq!(
            to_html(&blocks),
            "<p style=\"margin: 0.4em 0;\"><strong>Meaning.</strong></p>\
             <ul style=\"margin: 0.4em 0; padding-left: 1.2em;\"><li>one</li><li>two</li></ul>\
             <p style=\"margin: 0.4em 0;\">body text</p>",
            "html output did not match the expected paragraph + ul + paragraph shape"
        );
    }

    #[test]
    fn to_html_escapes_the_four_html_special_characters() {
        let blocks = parse_markdown("a < b & c > d \"e\"");
        assert_eq!(
            to_html(&blocks),
            "<p style=\"margin: 0.4em 0;\">a &lt; b &amp; c &gt; d &quot;e&quot;</p>",
            "html output did not escape <, &, >, or \""
        );
    }

    #[test]
    fn to_html_combines_bold_and_italic_into_nested_strong_em() {
        let blocks = vec![Block::Paragraph(vec![TextChunk {
            text: String::from("both"),
            bold: true,
            italic: true,
        }])];
        assert_eq!(
            to_html(&blocks),
            "<p style=\"margin: 0.4em 0;\"><strong><em>both</em></strong></p>",
            "html output did not nest <em> inside <strong> for a bold+italic chunk"
        );
    }

    #[test]
    fn to_ratatui_applies_the_bold_modifier_to_bold_chunks() {
        let blocks = parse_markdown("**Meaning.** body");
        let lines = to_ratatui(&blocks);
        let first_span = lines
            .first()
            .and_then(|line| line.spans.first())
            .expect("first span must exist for a single-paragraph render");
        let is_bold = first_span.style.add_modifier.contains(Modifier::BOLD);
        assert!(
            is_bold,
            "the first span of '**Meaning.**' was not styled BOLD"
        );
    }

    #[test]
    fn to_ratatui_prepends_a_bullet_glyph_and_indent_for_a_bullet_block() {
        let blocks = parse_markdown("  - nested");
        let lines = to_ratatui(&blocks);
        let prefix = lines
            .first()
            .and_then(|line| line.spans.first())
            .map(|span| span.content.as_ref().to_string())
            .expect("the bullet line must start with a prefix span");
        assert_eq!(
            prefix, "  • ",
            "the bullet line prefix did not include the two-space indent and the • glyph"
        );
    }

    #[test]
    fn to_plain_strips_markdown_markers_and_keeps_bullet_indent() {
        let blocks = parse_markdown("**Meaning.**\n  - nested\n\nbody");
        assert_eq!(
            to_plain(&blocks),
            "Meaning.\n  - nested\nbody",
            "plain output kept markdown markers or lost the bullet indent"
        );
    }
}
