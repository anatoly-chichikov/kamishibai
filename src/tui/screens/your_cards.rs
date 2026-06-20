//! Renderer for the `your cards` / `building your cards` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-2.jsx` (StepGenerating). One block
//! per card: head row plus four step lines (meta · scene · audio · picture).
//! "meta" is the rich card metadata produced by the Gemini card pass and is the first
//! real step in the pipeline. When a card is selected and finished the row
//! expands into a meta preview + artifact pane.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::markdown::{parse_markdown, to_ratatui};
use crate::session::{
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, GenerationCost,
};
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE_WORKING: &str = "building your cards";
const HEADLINE_DONE: &str = "your cards";
const HINT_WORKING: &str = "drawing each card one by one";
const HINT_DONE: &str = "all done";
const HINT_DONE_FAILED: &str = "some cards didn't make it";
const SPINNER_FRAME_MILLIS: u128 = 250;
const STEP_LABEL_COL_CHARS: usize = 14;
const STEP_SIZE_COL_CHARS: usize = 8;
const STEPS: [(&str, Artifact); 4] = [
    ("meta", Artifact::Meta),
    ("audio", Artifact::Sound),
    ("scene", Artifact::Scene),
    ("picture", Artifact::Picture),
];
const SPINNER_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// `ScreenView` handle for the `your cards` / generating screen. Title and
/// hint switch from the `building` copy to the `done` copy once every card
/// has either succeeded or terminally failed.
pub struct YourCards;

impl ScreenView for YourCards {
    fn title(&self, app: &App) -> Cow<'static, str> {
        let copy = if all_finished(app) {
            HEADLINE_DONE
        } else {
            HEADLINE_WORKING
        };
        Cow::Borrowed(copy)
    }

    fn hint(&self, app: &App) -> Cow<'static, str> {
        let copy = if !all_finished(app) {
            HINT_WORKING
        } else if app.cards_failed() > 0 {
            HINT_DONE_FAILED
        } else {
            HINT_DONE
        };
        Cow::Borrowed(copy)
    }

    fn footer(&self, app: &App, width: u16) -> Paragraph<'static> {
        footer(app, width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        let finished = all_finished(app);
        let banner_rows = if finished {
            super::banner::height(app)
        } else {
            0
        };
        if banner_rows == 0 {
            frame.render_widget(
                cards_paragraph(app, area.width as usize).scroll((app.body_scroll(), 0)),
                area,
            );
            return;
        }
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(banner_rows), Constraint::Min(0)])
            .split(area);
        frame.render_widget(super::banner::widget(app), split[0]);
        frame.render_widget(
            cards_paragraph(app, area.width as usize).scroll((app.body_scroll(), 0)),
            split[1],
        );
    }
}

fn cards_paragraph(app: &App, width: usize) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    if app.cards().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("preparing cards…", palette::dim())));
        return Paragraph::new(lines).style(palette::base());
    }
    let spinner_frame =
        (app.elapsed().as_millis() / SPINNER_FRAME_MILLIS) as usize % SPINNER_FRAMES.len();
    let running_target = app.cards_running_target();
    for (index, draft) in app.cards().iter().enumerate() {
        let focused = index == app.card_selected();
        let expanded = focused && app.card_expanded();
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == index { Some(kind) } else { None });
        lines.extend(card_block(
            draft,
            index,
            focused,
            expanded,
            width,
            running_for_card,
            spinner_frame,
        ));
    }
    Paragraph::new(lines).style(palette::base())
}

fn card_block<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    width: usize,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Vec<Line<'a>> {
    let artifacts = draft.artifacts();
    let progressed = card_progressed(artifacts, running);
    let mut lines: Vec<Line<'a>> = Vec::new();
    lines.extend(card_head(draft, idx, focused, expanded, progressed, width));
    if progressed {
        for &(_, kind) in &STEPS {
            let slot = slot_for(artifacts, kind);
            if !slot_visible(slot, kind, running) {
                continue;
            }
            lines.push(step_line(kind, slot, running, spinner_frame));
        }
    }
    if expanded {
        lines.extend(detail_pane(draft, width));
    }
    if progressed || expanded {
        lines.push(Line::from(""));
    }
    lines
}

const HEAD_PREFIX_CHARS: usize = 7;
const HEAD_ARROW: &str = " → ";
const HEAD_ARROW_CHARS: usize = 3;

fn card_head<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    progressed: bool,
    width: usize,
) -> Vec<Line<'a>> {
    let row_style = if focused {
        palette::highlight()
    } else {
        palette::base()
    };
    let glyph = if expanded {
        "▾"
    } else if card_finished(draft) {
        "▸"
    } else if progressed {
        "·"
    } else {
        " "
    };
    let glyph_style = if focused {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::dim2()
    };
    let num_style = if focused {
        palette::highlight()
    } else {
        palette::dim2()
    };
    let term_style = match (progressed, focused) {
        (true, true) => palette::highlight().add_modifier(Modifier::BOLD),
        (true, false) => palette::base(),
        (false, true) => palette::highlight_dim(),
        (false, false) => palette::dim2(),
    };
    let sentence_style = if focused {
        palette::highlight_dim()
    } else {
        palette::dim()
    };
    let term_chars = draft.term().chars().count();
    let head_used = HEAD_PREFIX_CHARS + term_chars;
    let mut head_spans: Vec<Span<'a>> = Vec::new();
    head_spans.push(Span::styled(format!(" {glyph} "), glyph_style));
    head_spans.push(Span::styled(format!("{:0>2}  ", idx + 1), num_style));
    head_spans.push(Span::styled(String::from(draft.term()), term_style));
    let Some(meta) = draft.meta() else {
        let pad = width.saturating_sub(head_used);
        if pad > 0 {
            head_spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        return vec![Line::from(head_spans)];
    };
    let row1_used = head_used + HEAD_ARROW_CHARS;
    let cost_label = visible_card_cost_label(draft, row1_used, width);
    let cost_chars = cost_label
        .as_ref()
        .map(|label| label.chars().count())
        .unwrap_or(0);
    let avail_first = width.saturating_sub(row1_used + cost_chars);
    let chunks = wrap_sentence(meta.target_sentence(), avail_first, avail_first);
    let first = chunks.first().cloned().unwrap_or_default();
    head_spans.push(Span::styled(HEAD_ARROW, sentence_style));
    let first_len = first.chars().count();
    head_spans.push(Span::styled(first, sentence_style));
    if let Some(label) = cost_label {
        let cost_style = if focused {
            palette::highlight_dim()
        } else {
            palette::dim2()
        };
        head_spans.push(Span::styled(label, cost_style));
    }
    let pad = width.saturating_sub(row1_used + first_len + cost_chars);
    if pad > 0 {
        head_spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    let mut lines: Vec<Line<'a>> = Vec::with_capacity(chunks.len().max(1));
    lines.push(Line::from(head_spans));
    let cont_indent: String = " ".repeat(row1_used);
    for chunk in chunks.into_iter().skip(1) {
        let chunk_len = chunk.chars().count();
        let mut spans: Vec<Span<'a>> = Vec::new();
        spans.push(Span::styled(cont_indent.clone(), row_style));
        spans.push(Span::styled(chunk, sentence_style));
        let pad = width.saturating_sub(row1_used + chunk_len);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn wrap_sentence(sentence: &str, first_avail: usize, cont_avail: usize) -> Vec<String> {
    super::common::wrap_words(sentence, first_avail, cont_avail)
}

fn head_rows(draft: &CardDraft, width: usize) -> usize {
    let Some(meta) = draft.meta() else {
        return 1;
    };
    let term_chars = draft.term().chars().count();
    let row1_used = HEAD_PREFIX_CHARS + term_chars + HEAD_ARROW_CHARS;
    let cost_chars = visible_card_cost_label(draft, row1_used, width)
        .as_ref()
        .map(|label| label.chars().count())
        .unwrap_or(0);
    let avail = width.saturating_sub(row1_used + cost_chars);
    wrap_sentence(meta.target_sentence(), avail, avail)
        .len()
        .max(1)
}

fn visible_card_cost_label(draft: &CardDraft, row_used: usize, width: usize) -> Option<String> {
    let label = card_cost(draft).map(|cost| format!("  {}", cost.dollars()))?;
    let label_len = label.chars().count();
    if width.saturating_sub(row_used) < label_len + 8 {
        return None;
    }
    Some(label)
}

fn card_finished(draft: &CardDraft) -> bool {
    let artifacts = draft.artifacts();
    artifacts.all_ready() || artifacts.has_failed()
}

fn slot_for(artifacts: &CardArtifacts, kind: Artifact) -> &ArtifactSlot {
    match kind {
        Artifact::Meta => artifacts.meta(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    }
}

fn slot_visible(slot: &ArtifactSlot, kind: Artifact, running: Option<Artifact>) -> bool {
    slot.ready()
        || slot.discarded()
        || slot.failed_terminally()
        || slot.tally().done() > 0
        || running == Some(kind)
}

fn card_progressed(artifacts: &CardArtifacts, running: Option<Artifact>) -> bool {
    if running.is_some() {
        return true;
    }
    STEPS
        .iter()
        .any(|&(_, kind)| slot_visible(slot_for(artifacts, kind), kind, None))
}

fn step_line<'a>(
    kind: Artifact,
    slot: &'a ArtifactSlot,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Line<'a> {
    let active = running == Some(kind);
    let label = step_label(kind, slot);
    let (glyph, status_style, label_style, note_spans) = step_state(slot, active, spinner_frame);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled("    ", palette::base()));
    spans.push(Span::styled(format!("{glyph} "), status_style));
    spans.push(Span::styled(label.clone(), label_style));
    spans.push(Span::styled(" ".repeat(label_gap(&label)), palette::dim()));
    spans.extend(note_spans);
    Line::from(spans)
}

fn step_label(kind: Artifact, slot: &ArtifactSlot) -> String {
    match slot.file() {
        Some(file) => artifact_file_label(kind, file),
        None => String::from(step_name(kind)),
    }
}

fn step_state<'a>(
    slot: &'a ArtifactSlot,
    active: bool,
    spinner_frame: usize,
) -> (
    String,
    ratatui::style::Style,
    ratatui::style::Style,
    Vec<Span<'a>>,
) {
    let row_dim = palette::dim();
    let row_dim2 = palette::dim2();
    let row_fg = palette::base();
    if slot.ready() {
        let mut note: Vec<Span<'a>> = Vec::new();
        if let Some(file) = slot.file() {
            note.push(Span::styled(
                pad_left(file.size(), STEP_SIZE_COL_CHARS),
                palette::dim(),
            ));
            if let Some(cost) = file.cost() {
                note.push(Span::styled("  ", palette::dim()));
                note.push(Span::styled(cost.dollars(), palette::dim2()));
            }
        }
        return (String::from("✓"), row_fg, palette::link(), note);
    }
    if slot.discarded() {
        return (
            String::from("⊘"),
            row_dim,
            row_dim,
            vec![Span::styled(String::from("discarded"), palette::dim())],
        );
    }
    if slot.failed_terminally() {
        return (
            String::from("✗"),
            row_fg,
            row_fg,
            vec![Span::styled(
                String::from("gave up after 3 tries"),
                palette::dim(),
            )],
        );
    }
    let attempts = slot.tally().done();
    if attempts > 0 {
        let label = if active {
            format!("retry {}/3…", attempts + 1)
        } else {
            format!("retry {}/3 paused", attempts + 1)
        };
        let glyph = if active {
            String::from(SPINNER_FRAMES[spinner_frame])
        } else {
            String::from("·")
        };
        return (
            glyph,
            row_fg,
            row_fg,
            vec![Span::styled(label, palette::dim())],
        );
    }
    if active {
        return (
            String::from(SPINNER_FRAMES[spinner_frame]),
            row_fg,
            row_fg,
            vec![Span::styled(String::from("working…"), palette::dim())],
        );
    }
    (
        String::from("○"),
        row_dim2,
        row_dim2,
        vec![Span::styled(String::from("queued"), palette::dim())],
    )
}

fn step_name(kind: Artifact) -> &'static str {
    match kind {
        Artifact::Meta => "meta",
        Artifact::Sound => "audio",
        Artifact::Scene => "scene",
        Artifact::Picture => "picture",
    }
}

pub(crate) fn artifact_file_label(kind: Artifact, file: &ArtifactFile) -> String {
    match file_extension(file.name()) {
        Some(extension) => format!("{}.{}", step_name(kind), extension),
        None => String::from(step_name(kind)),
    }
}

fn file_extension(filename: &str) -> Option<&str> {
    filename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
}

fn label_gap(label: &str) -> usize {
    STEP_LABEL_COL_CHARS
        .saturating_sub(label.chars().count())
        .max(2)
}

fn pad_left(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return String::from(text);
    }
    format!("{}{}", " ".repeat(width - len), text)
}

fn detail_pane(draft: &CardDraft, width: usize) -> Vec<Line<'_>> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    let indent = "      ";
    lines.push(Line::from(""));
    if let Some(meta) = draft.meta() {
        lines.extend(meta_preview(meta, indent, width));
    } else {
        lines.push(Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled("meta not generated yet", palette::dim2()),
        ]));
    }
    lines
}

fn meta_preview<'a>(meta: &'a CardMeta, indent: &'static str, width: usize) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let label = |text: &'static str| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::dim2()),
        ])
    };
    lines.push(label("target"));
    lines.extend(value_lines(
        meta.target_sentence(),
        indent,
        width,
        palette::base(),
    ));
    lines.push(Line::from(""));
    lines.push(label("source"));
    lines.extend(highlight_lines(meta, indent, width));
    lines.push(Line::from(""));
    lines.push(label("hint"));
    lines.extend(value_lines(
        meta.source_hint(),
        indent,
        width,
        palette::base(),
    ));
    lines.push(Line::from(""));
    lines.push(label(
        "meaning · pronunciation · transcription · importance",
    ));
    lines.extend(value_lines(
        format!(
            "{} · /{}/ · /{}/ · {}/10",
            meta.meaning(),
            meta.pronunciation(),
            meta.transcription(),
            meta.importance(),
        )
        .as_str(),
        indent,
        width,
        palette::base(),
    ));
    if !meta.source_context().trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(label("context"));
        let indent_w = super::common::display_width(indent);
        let inner = width.saturating_sub(indent_w).max(20);
        for line in to_ratatui(&parse_markdown(meta.source_context())) {
            for wrapped in softwrap_line(line, inner) {
                lines.push(restyle_with_indent(wrapped, indent));
            }
        }
    }
    lines
}

fn value_lines<'a>(
    text: &str,
    indent: &'static str,
    width: usize,
    style: ratatui::style::Style,
) -> Vec<Line<'a>> {
    let indent_w = super::common::display_width(indent);
    let inner = width.saturating_sub(indent_w).max(20);
    super::common::wrap_words(text, inner, inner)
        .into_iter()
        .map(|chunk| {
            Line::from(vec![
                Span::styled(indent, palette::base()),
                Span::styled(chunk, style),
            ])
        })
        .collect()
}

/// Soft-wrap one markdown-rendered line into chunks that each fit `width`
/// display columns. Preserves per-span style + modifiers across wrap points.
/// CJK code points count as two cells, others as one — close enough to the
/// real terminal width without pulling in `unicode-width`.
fn softwrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;
    let mut pending_space: Option<ratatui::style::Style> = None;
    for span in line.spans {
        for (token, whitespace) in styled_tokens(span.content.as_ref()) {
            if whitespace {
                if current_width > 0 {
                    pending_space = Some(span.style);
                }
                continue;
            }
            let token_width = super::common::display_width(token.as_str());
            let gap = usize::from(current_width > 0 && pending_space.is_some());
            if current_width + gap + token_width <= width {
                if let Some(style) = pending_space.take() {
                    current_spans.push(Span::styled(String::from(" "), style));
                    current_width += 1;
                }
                current_spans.push(Span::styled(token, span.style));
                current_width += token_width;
                continue;
            }
            if !current_spans.is_empty() {
                out.push(Line::from(std::mem::take(&mut current_spans)));
                current_width = 0;
                pending_space = None;
            }
            if token_width <= width {
                current_spans.push(Span::styled(token, span.style));
                current_width = token_width;
                continue;
            }
            let parts = split_token(token.as_str(), width);
            let last = parts.len().saturating_sub(1);
            for (index, part) in parts.into_iter().enumerate() {
                current_spans.push(Span::styled(part.clone(), span.style));
                current_width = super::common::display_width(part.as_str());
                if index < last {
                    out.push(Line::from(std::mem::take(&mut current_spans)));
                    current_width = 0;
                }
            }
        }
    }
    if !current_spans.is_empty() {
        out.push(Line::from(current_spans));
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

fn styled_tokens(text: &str) -> Vec<(String, bool)> {
    let mut tokens: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_space: Option<bool> = None;
    for ch in text.chars() {
        let space = ch.is_whitespace();
        if current_space == Some(space) {
            current.push(ch);
            continue;
        }
        if !current.is_empty() {
            tokens.push((std::mem::take(&mut current), current_space.unwrap_or(false)));
        }
        current.push(ch);
        current_space = Some(space);
    }
    if !current.is_empty() {
        tokens.push((current, current_space.unwrap_or(false)));
    }
    tokens
}

fn split_token(text: &str, width: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in text.chars() {
        let char_width = super::common::char_width(ch);
        if current_width > 0 && current_width + char_width > width {
            parts.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += char_width;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

/// Repaint one markdown-rendered line with the preview palette and prepend the
/// shared indent span. The markdown layer emits neutral spans carrying only
/// BOLD / ITALIC modifiers — we keep those and force the foreground colour to
/// `palette::base()` so context blends with the surrounding preview rows.
fn restyle_with_indent(line: Line<'static>, indent: &'static str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::styled(indent, palette::base()));
    for span in line.spans {
        let style = palette::base().add_modifier(span.style.add_modifier);
        spans.push(Span::styled(span.content, style));
    }
    Line::from(spans)
}

fn highlight_lines<'a>(meta: &'a CardMeta, indent: &'static str, width: usize) -> Vec<Line<'a>> {
    let sentence = meta.source_sentence();
    let highlight = meta.source_highlight();
    if highlight.is_empty() {
        return value_lines(sentence, indent, width, palette::base());
    }
    let line = if let Some(pos) = sentence.find(highlight) {
        let head = &sentence[..pos];
        let middle = &sentence[pos..pos + highlight.len()];
        let tail = &sentence[pos + highlight.len()..];
        Line::from(vec![
            Span::styled(head.to_string(), palette::base()),
            Span::styled(
                middle.to_string(),
                palette::base().add_modifier(Modifier::BOLD),
            ),
            Span::styled(tail.to_string(), palette::base()),
        ])
    } else {
        Line::from(vec![Span::styled(sentence.to_string(), palette::base())])
    };
    let indent_w = super::common::display_width(indent);
    let inner = width.saturating_sub(indent_w).max(20);
    softwrap_line(line, inner)
        .into_iter()
        .map(|line| restyle_with_indent(line, indent))
        .collect()
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}

/// Row offset and height of the currently focused card inside the scrolling
/// card list, in body-rect rows. Returns `None` when there are no cards yet.
/// Mirrors the per-card layout used by `cards_paragraph` so scroll-snapping
/// and renderer stay in lockstep. `width` is the body-rect width in chars.
pub(crate) fn focused_card_range(app: &App, width: usize) -> Option<(u16, u16)> {
    if app.cards().is_empty() {
        return None;
    }
    let running_target = app.cards_running_target();
    let mut offset: usize = 0;
    for (idx, draft) in app.cards().iter().enumerate() {
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let expanded = idx == app.card_selected() && app.card_expanded();
        let (rows, trailing) = card_layout(draft, running_for_card, expanded, width);
        if idx == app.card_selected() {
            let focus_rows = if expanded {
                head_rows(draft, width)
                    .saturating_add(step_rows_for(draft, running_for_card).len())
                    .max(1)
            } else {
                rows
            };
            return Some((
                u16::try_from(offset).unwrap_or(u16::MAX),
                u16::try_from(focus_rows).unwrap_or(u16::MAX),
            ));
        }
        offset = offset.saturating_add(rows + trailing);
    }
    None
}

/// Total number of lines `cards_paragraph` will produce for the current state
/// of `app`. Mirrors the per-card layout: head rows (one or more, depending on
/// how the term + meta sentence wrap) + visible step rows + optional detail
/// pane (only on the focused, expanded card) + trailing blank line for any
/// card that emitted extra rows. Used by both the scroll clamp in `tui::app`
/// and the click hit tester in `tui::links`, so they stay in lockstep with the
/// renderer. `width` is the body-rect width in chars.
pub(crate) fn content_height(app: &App, width: usize) -> u16 {
    if app.cards().is_empty() {
        return 0;
    }
    let running_target = app.cards_running_target();
    let mut total: usize = 0;
    for (idx, draft) in app.cards().iter().enumerate() {
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let expanded = idx == app.card_selected() && app.card_expanded();
        let (rows, trailing) = card_layout(draft, running_for_card, expanded, width);
        total = total.saturating_add(rows + trailing);
    }
    u16::try_from(total).unwrap_or(u16::MAX)
}

/// Number of head rows the focused card produces given the current body-rect
/// width. Mirrors the wrap that `card_head` runs at render time, so the click
/// hit-tester in `tui::links` can find the start of the step rows.
pub(crate) fn head_rows_for(draft: &CardDraft, width: usize) -> usize {
    head_rows(draft, width)
}

/// Return the artifact step rows visible for one card.
pub(crate) fn step_rows_for(draft: &CardDraft, running: Option<Artifact>) -> Vec<Artifact> {
    let artifacts = draft.artifacts();
    if !card_progressed(artifacts, running) {
        return Vec::new();
    }
    STEPS
        .iter()
        .filter_map(|&(_, kind)| {
            if slot_visible(slot_for(artifacts, kind), kind, running) {
                Some(kind)
            } else {
                None
            }
        })
        .collect()
}

fn card_layout(
    draft: &CardDraft,
    running: Option<Artifact>,
    expanded: bool,
    width: usize,
) -> (usize, usize) {
    let steps = step_rows_for(draft, running);
    let mut rows = head_rows(draft, width);
    rows += steps.len();
    if expanded {
        rows = rows.saturating_add(detail_pane_height(draft, width));
    }
    let trailing = if !steps.is_empty() || expanded { 1 } else { 0 };
    (rows, trailing)
}

/// Number of body-rect rows the expanded meta-preview pane consumes for one
/// card. Verbatim mirror of `detail_pane` / `meta_preview` so callers can keep
/// scroll offsets and click hit-tests aligned with the rendered output.
pub(crate) fn detail_pane_height(draft: &CardDraft, width: usize) -> usize {
    let Some(meta) = draft.meta() else {
        return 2;
    };
    1 + meta_preview(meta, "      ", width).len()
}

/// Return one card's known Gemini cost across generated artifacts.
pub(crate) fn card_cost(draft: &CardDraft) -> Option<GenerationCost> {
    draft.artifacts().cost()
}

/// Return the known Gemini cost across every card in the current batch.
pub(crate) fn total_cost(app: &App) -> Option<GenerationCost> {
    let costs = app.cards().iter().filter_map(card_cost).collect::<Vec<_>>();
    if costs.is_empty() {
        return None;
    }
    Some(costs.into_iter().sum())
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 3/3", palette::dim2()));
    left.push(super::common::status_sep());
    left.push(Span::styled(
        app.cards_ready().to_string(),
        palette::base().add_modifier(Modifier::BOLD),
    ));
    left.push(Span::styled(
        format!("/{} ready", app.cards().len()),
        palette::dim(),
    ));
    if app.cards_failed() > 0 {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            app.cards_failed().to_string(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(" gave up", palette::dim()));
    }
    if all_finished(app)
        && let Some(cost) = total_cost(app)
    {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            cost.dollars_cents(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
    }
    left.push(super::common::status_sep());
    left.push(Span::styled(elapsed(app), palette::dim2()));
    let hints = vec![
        super::common::FooterHint::primary("Enter", "expand"),
        super::common::FooterHint::secondary("R", "change"),
        super::common::FooterHint::secondary("Ctrl+G", "regenerate"),
        super::common::FooterHint::ghost("↑↓", "nav"),
        super::common::quit_hint(app.quit_pending()),
    ];
    super::common::footer_bar(left, hints, width)
}

fn elapsed(app: &App) -> String {
    let seconds = app.elapsed().as_secs();
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes:02}:{remainder:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn markdown_softwrap_moves_whole_words_to_the_next_row() {
        let rows = softwrap_line(Line::from("alpha beta gamma"), 10)
            .iter()
            .map(plain)
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![String::from("alpha beta"), String::from("gamma")],
            "markdown preview wrap must move whole words instead of splitting them"
        );
    }
}
