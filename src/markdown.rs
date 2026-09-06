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

/// Limit a recognized card context to five meaning groups and remove its glossary recap.
#[must_use]
pub fn compact_card_context(input: &str) -> String {
    let lines = input.lines().collect::<Vec<_>>();
    let headers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| card_header(line.trim()).map(|_| index))
        .collect::<Vec<_>>();
    if headers.len() != 4
        || lines[..headers[0]]
            .iter()
            .any(|line| !line.trim().is_empty())
        || !card_header(lines[headers[0]].trim()).is_some_and(crate::languages::is_meaning_header)
    {
        return input.to_string();
    }
    let mut indentation = None;
    let mut meanings = 0;
    let mut overflow = None;
    let mut separated = false;
    let mut recap = None;
    for (index, line) in lines
        .iter()
        .enumerate()
        .take(headers[1])
        .skip(headers[0] + 1)
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            separated = indentation.is_some();
            continue;
        }
        if let Some(indent) = meaning_bullet(line) {
            if recap.is_some() {
                return input.to_string();
            }
            let base = *indentation.get_or_insert(indent);
            if indent < base {
                return input.to_string();
            }
            if indent == base {
                meanings += 1;
                if meanings > crate::session::MAX_CARD_MEANINGS {
                    overflow.get_or_insert(index);
                }
            }
            separated = false;
        } else if indentation.is_none() {
            return input.to_string();
        } else if recap.is_none() && (separated || !line.starts_with(char::is_whitespace)) {
            if !trimmed
                .split_once([':', '：'])
                .is_some_and(|(label, text)| !label.trim().is_empty() && !text.trim().is_empty())
            {
                return input.to_string();
            }
            recap.get_or_insert(index);
        }
    }
    let Some(start) = overflow.or(recap) else {
        return input.to_string();
    };
    let first = lines[..start].join("\n");
    format!(
        "{}\n\n{}{}",
        first.trim_end(),
        lines[headers[1]..].join("\n"),
        if input.ends_with('\n') { "\n" } else { "" }
    )
}

/// Recognize the standalone bold headings used by the card explanation contract.
fn card_header(line: &str) -> Option<&str> {
    line.strip_prefix("**")
        .and_then(|inner| inner.strip_suffix("**"))
        .filter(|inner| {
            !inner.trim().is_empty() && !inner.contains("**") && meaning_bullet(line).is_none()
        })
}

/// Identify the indentation of a canonical or legacy fully emphasized meaning bullet.
fn meaning_bullet(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix("**")
        .and_then(|inner| inner.strip_suffix("**"))
        .filter(|inner| !inner.contains("**"))
        .unwrap_or(trimmed);
    body.strip_prefix("- ")
        .or_else(|| body.strip_prefix("* "))
        .map(|_| line.chars().take_while(|ch| ch.is_whitespace()).count())
}

/// Parse a card explanation after limiting its glossary and removing its recap.
#[must_use]
pub fn parse_card_context(input: &str) -> Vec<Block> {
    parse_markdown(&compact_card_context(input))
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
        if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '\\' | '*') {
            buffer.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if chars[i] == '*' {
            if i + 1 < chars.len() && chars[i + 1] == '*' {
                if let Some(end) = find_marker(&chars, i + 2, &['*', '*']) {
                    flush(&mut chunks, &mut buffer);
                    chunks.push(TextChunk {
                        text: unescaped(&chars[i + 2..end]),
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
                    text: unescaped(&chars[i + 1..end]),
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
        if !escaped(chars, i) && chars[i..i + marker.len()] == *marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn escaped(chars: &[char], index: usize) -> bool {
    chars[..index]
        .iter()
        .rev()
        .take_while(|character| **character == '\\')
        .count()
        % 2
        == 1
}

fn unescaped(chars: &[char]) -> String {
    let mut text = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && index + 1 < chars.len() && matches!(chars[index + 1], '\\' | '*')
        {
            text.push(chars[index + 1]);
            index += 2;
        } else {
            text.push(chars[index]);
            index += 1;
        }
    }
    text
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
    use super::{
        Block, TextChunk, compact_card_context, parse_markdown, to_html, to_plain, to_ratatui,
    };
    use ratatui::style::Modifier;

    #[test]
    fn card_context_cannot_repeat_the_meaning_list_before_usage() {
        let context = "**Meaning.**\n- **approval**\n- a health benefit\ndifference: one expresses approval and the other describes health.\n\n**Usage.**\nSay it to congratulate a friend.\n\n**Limits.**\nA dry reply can sound dismissive.\n\n**Nuance.**\nTone matters: \"Good for you!\" can sound sincere or sarcastic.";
        assert_eq!(
            compact_card_context(context),
            "**Meaning.**\n- **approval**\n- a health benefit\n\n**Usage.**\nSay it to congratulate a friend.\n\n**Limits.**\nA dry reply can sound dismissive.\n\n**Nuance.**\nTone matters: \"Good for you!\" can sound sincere or sarcastic.",
            "the obsolete recap survived or useful card content changed"
        );
    }

    #[test]
    fn compact_context_cannot_rewrite_a_list_only_card() {
        let context = "**Meaning.**\n- **approval**\n- benefit\n\n**Usage.**\nCongratulating a friend.\n\n**Limits.**\nA dry tone can sound dismissive.\n\n**Nuance.**\nKeep the explanation and its quoted example.";
        assert_eq!(
            compact_card_context(context),
            context,
            "a compact card was changed"
        );
    }

    #[test]
    fn compact_context_cannot_remove_indented_meaning_continuations() {
        let context = "**Meaning.**\n- **approval**\n- a benefit\n  for health or well-being\nrecap: the first is approval and the second is a benefit.\n\n**Usage.**\nCommon.\n\n**Limits.**\nNone.\n\n**Nuance.**\nNo additional peculiarity.";
        let compact = compact_card_context(context);
        assert!(
            compact.contains("- a benefit\n  for health or well-being\n\n**Usage.**"),
            "a wrapped reviewed meaning was removed"
        );
    }

    #[test]
    fn compact_context_cannot_guess_at_an_unstructured_description() {
        let context = "**Meaning.**\n- a benefit\nAn explanation supplied by the user.\n\n**Notes.**\nKeep all of it.";
        assert_eq!(
            compact_card_context(context),
            context,
            "an unrecognized description was silently shortened"
        );
    }

    #[test]
    fn compact_context_cannot_shorten_an_unrelated_four_section_document() {
        let context = "**Ingredients**\n- sesame\nWarning: contains a severe allergen.\n\n**Preparation**\nMix.\n\n**Storage**\nKeep chilled.\n\n**Serving**\nServe cold.";
        assert_eq!(
            compact_card_context(context),
            context,
            "an unrelated four-section document lost its warning"
        );
    }

    #[test]
    fn compact_context_cannot_keep_a_blank_separated_indented_recap() {
        let context = "**Meaning.**\n- **approval**\n- benefit\n\n  recap: one means approval and the other means benefit.\n\n**Usage.**\nCommon.\n\n**Limits.**\nNone.\n\n**Nuance.**\nNo additional peculiarity.";
        assert_eq!(
            compact_card_context(context),
            "**Meaning.**\n- **approval**\n- benefit\n\n**Usage.**\nCommon.\n\n**Limits.**\nNone.\n\n**Nuance.**\nNo additional peculiarity.",
            "an indented recap survived as a false meaning continuation"
        );
    }

    #[test]
    fn compact_context_cannot_remove_an_unlabelled_user_paragraph() {
        let context = "**Meaning.**\n- benefit\nAn explanation supplied by the user.\n\n**Usage.**\nCommon.\n\n**Limits.**\nNone.\n\n**Nuance.**\nKeep this.";
        assert_eq!(
            compact_card_context(context),
            context,
            "an unrecognized paragraph was mistaken for a labelled recap"
        );
    }

    #[test]
    fn compact_context_cannot_remove_a_comparison_from_the_usage_sections() {
        let context = "**Meaning.**\n- **approval**\n\n**Usage.**\nUse: say it to congratulate a friend.\n\n**Limits.**\nDifference: a dry reply can sound dismissive.\n\n**Nuance.**\nContrast: tone changes the impression.";
        assert_eq!(
            compact_card_context(context),
            context,
            "useful usage guidance was mistaken for a glossary recap"
        );
    }

    /// Build a recognized four-section context for meaning-limit regressions.
    fn meaning_context(meanings: &str) -> String {
        format!(
            "**Значение.**\n{meanings}\n\n**Где встречается.**\nПодходит в обычной беседе.\n\n**Где неуместно.**\nОсобых ограничений нет.\n\n**Нюанс.**\nПолезное пояснение с одним примером: «example»."
        )
    }

    #[test]
    fn a_card_context_cannot_display_more_than_five_reviewed_meanings() {
        let input = meaning_context(
            "- **[selected] первое значение**\n- [noun] второе\n- третье\n- четвёртое\n- пятое\n- шестое",
        );
        assert_eq!(
            compact_card_context(&input),
            meaning_context(
                "- **[selected] первое значение**\n- [noun] второе\n- третье\n- четвёртое\n- пятое"
            ),
            "the sixth meaning survived or the selected meaning, tags, order, or usage sections changed"
        );
    }

    #[test]
    fn five_reviewed_meanings_cannot_change_during_projection() {
        let input =
            meaning_context("- **[selected] первое**\n- второе\n- третье\n- четвёртое\n- пятое");
        assert_eq!(
            compact_card_context(&input),
            input,
            "a card within the meaning limit changed"
        );
    }

    #[test]
    fn wrapped_meaning_lines_cannot_count_as_additional_meanings() {
        let kept = "- **[selected] первое**\n  продолжение первого\n- второе\n  продолжение второго\n- третье\n- четвёртое\n- пятое\n  продолжение пятого";
        let input = meaning_context(&format!("{kept}\n- шестое\n  продолжение шестого"));
        assert_eq!(
            compact_card_context(&input),
            meaning_context(kept),
            "wrapped definition text was counted separately or a removed meaning left its continuation behind"
        );
    }

    #[test]
    fn meaning_limit_and_recap_removal_cannot_leave_either_overflow() {
        let kept = "- **первое**\n- второе\n- третье\n- четвёртое\n- пятое";
        let input = meaning_context(&format!(
            "{kept}\n- шестое\n  продолжение шестого\n\n  различие: бессмысленный пересказ значений."
        ));
        assert_eq!(
            compact_card_context(&input),
            meaning_context(kept),
            "the sixth meaning or obsolete recap survived combined projection"
        );
    }

    #[test]
    fn a_legacy_bold_bullet_marker_cannot_disable_the_meaning_limit() {
        let kept = "**- [selected] первое**\n- второе\n- третье\n- четвёртое\n- пятое";
        assert_eq!(
            compact_card_context(&meaning_context(&format!("{kept}\n- шестое"))),
            meaning_context(kept),
            "the emphasized bullet marker became a heading or bypassed the five-meaning limit"
        );
    }

    #[test]
    fn nested_definition_bullets_cannot_consume_the_meaning_limit() {
        let kept = "- **первое**\n  - уточнение первого\n- второе\n- третье\n- четвёртое\n- пятое";
        assert_eq!(
            compact_card_context(&meaning_context(&format!("{kept}\n- шестое"))),
            meaning_context(kept),
            "a nested definition detail consumed a reviewed-meaning slot"
        );
    }

    #[test]
    fn an_unknown_document_cannot_lose_lines_under_the_meaning_limit() {
        let input = meaning_context("- **first**\n- second\n- third\n- fourth\n- fifth\n- sixth")
            .replace("**Значение.**", "**Ingredients**");
        assert_eq!(
            compact_card_context(&input),
            input,
            "an unrelated document lost content under the card meaning limit"
        );
    }

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
    fn escaped_markers_and_backslashes_stay_literal_in_every_projection() {
        let blocks = parse_markdown(r"- **literal \*\*stars\*\* and a\\b**");
        let lines = to_ratatui(&blocks);
        let text = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(
            (
                blocks,
                to_plain(&parse_markdown(r"- **literal \*\*stars\*\* and a\\b**")),
                to_html(&parse_markdown(r"- **literal \*\*stars\*\* and a\\b**")),
                text,
            ),
            (
                vec![Block::Bullet {
                    indent: 0,
                    chunks: vec![bold(r"literal **stars** and a\b")],
                }],
                String::from(r"- literal **stars** and a\b"),
                String::from(
                    r#"<ul style="margin: 0.4em 0; padding-left: 1.2em;"><li><strong>literal **stars** and a\b</strong></li></ul>"#,
                ),
                String::from(r"• literal **stars** and a\b"),
            ),
            "escaped markdown changed literal stars or backslashes in a shared projection"
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
