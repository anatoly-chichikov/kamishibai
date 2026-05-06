//! Renderer for the `your cards` / `building your cards` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-2.jsx` (StepGenerating). One block
//! per card: head row plus four step lines (meta · scene · audio · picture).
//! "meta" is the rich body produced by the Pro Gemini pass and is the first
//! real step in the pipeline. When a card is selected and finished the row
//! expands into a body preview + artifact pane.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::session::{Artifact, ArtifactSlot, CardArtifacts, CardBody, CardDraft};
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE_WORKING: &str = "building your cards";
const HEADLINE_DONE: &str = "your cards";
const HINT_WORKING: &str = "drawing each card one by one";
const HINT_DONE: &str = "all done";
const HINT_DONE_FAILED: &str = "some cards didn't make it";
const STEPS: [(&str, Artifact); 4] = [
    ("meta", Artifact::Body),
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
        let lift = super::banner::LIFT.min(area.y);
        let lifted = Rect {
            x: area.x,
            y: area.y - lift,
            width: area.width,
            height: area.height + lift,
        };
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(banner_rows),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(lifted);
        frame.render_widget(super::banner::widget(app), split[0]);
        frame.render_widget(
            cards_paragraph(app, area.width as usize).scroll((app.body_scroll(), 0)),
            split[2],
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
    let spinner_frame = (app.elapsed().as_millis() / 180) as usize % SPINNER_FRAMES.len();
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
        for &(name, kind) in &STEPS {
            let slot = slot_for(artifacts, kind);
            if !slot_visible(slot, kind, running) {
                continue;
            }
            lines.push(step_line(name, kind, slot, running, spinner_frame));
        }
    }
    if expanded {
        lines.extend(detail_pane(draft));
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
    let Some(body) = draft.body() else {
        let pad = width.saturating_sub(head_used);
        if pad > 0 {
            head_spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        return vec![Line::from(head_spans)];
    };
    let row1_used = head_used + HEAD_ARROW_CHARS;
    let avail_first = width.saturating_sub(row1_used);
    let chunks = wrap_sentence(body.target_sentence(), avail_first, avail_first);
    let first = chunks.first().cloned().unwrap_or_default();
    head_spans.push(Span::styled(HEAD_ARROW, sentence_style));
    let first_len = first.chars().count();
    head_spans.push(Span::styled(first, sentence_style));
    let pad = width.saturating_sub(row1_used + first_len);
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
    if first_avail == 0 || cont_avail == 0 {
        return vec![String::from(sentence)];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len: usize = 0;
    let mut limit = first_avail;
    for word in sentence.split_whitespace() {
        let word_len = word.chars().count();
        let separator = usize::from(!current.is_empty());
        if current_len + separator + word_len <= limit {
            if separator == 1 {
                current.push(' ');
            }
            current.push_str(word);
            current_len += separator + word_len;
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            limit = cont_avail;
        }
        let mut tail = word;
        loop {
            let tail_len = tail.chars().count();
            if tail_len <= limit {
                current.push_str(tail);
                current_len = tail_len;
                break;
            }
            let mut byte_idx = tail.len();
            for (i, (pos, _)) in tail.char_indices().enumerate() {
                if i == limit {
                    byte_idx = pos;
                    break;
                }
            }
            chunks.push(String::from(&tail[..byte_idx]));
            tail = &tail[byte_idx..];
            limit = cont_avail;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

fn head_rows(draft: &CardDraft, width: usize) -> usize {
    let Some(body) = draft.body() else {
        return 1;
    };
    let term_chars = draft.term().chars().count();
    let row1_used = HEAD_PREFIX_CHARS + term_chars + HEAD_ARROW_CHARS;
    let avail = width.saturating_sub(row1_used);
    wrap_sentence(body.target_sentence(), avail, avail)
        .len()
        .max(1)
}

fn card_finished(draft: &CardDraft) -> bool {
    let artifacts = draft.artifacts();
    artifacts.all_ready() || artifacts.has_failed()
}

fn slot_for(artifacts: &CardArtifacts, kind: Artifact) -> &ArtifactSlot {
    match kind {
        Artifact::Body => artifacts.body(),
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
    name: &'a str,
    kind: Artifact,
    slot: &'a ArtifactSlot,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Line<'a> {
    let active = running == Some(kind);
    let (glyph, status_style, name_style, note_spans) = step_state(slot, active, spinner_frame);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled("    ", palette::base()));
    spans.push(Span::styled(format!("{glyph} "), status_style));
    spans.push(Span::styled(super::common::pad_right(name, 9), name_style));
    spans.extend(note_spans);
    Line::from(spans)
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
            note.push(Span::styled(String::from(file.name()), palette::link()));
            note.push(Span::styled(format!(" · {}", file.size()), palette::dim()));
        }
        return (String::from("✓"), row_fg, row_fg, note);
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

fn detail_pane(draft: &CardDraft) -> Vec<Line<'_>> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    let indent = "      ";
    lines.push(Line::from(""));
    if let Some(body) = draft.body() {
        lines.extend(body_preview(body, indent));
    } else {
        lines.push(Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled("body not generated yet", palette::dim2()),
        ]));
    }
    lines
}

fn body_preview<'a>(body: &'a CardBody, indent: &'static str) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let label = |text: &'static str| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::dim2()),
        ])
    };
    let value = |text: String| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::base()),
        ])
    };
    lines.push(label("target"));
    lines.push(value(body.target_sentence().to_string()));
    lines.push(Line::from(""));
    lines.push(label("source"));
    lines.push(highlight_line(body, indent));
    lines.push(Line::from(""));
    lines.push(label("hint"));
    lines.push(value(body.source_hint().to_string()));
    lines.push(Line::from(""));
    lines.push(label(
        "meaning · pronunciation · transcription · importance",
    ));
    lines.push(value(format!(
        "{} · /{}/ · /{}/ · {}/10",
        body.meaning(),
        body.pronunciation(),
        body.transcription(),
        body.importance(),
    )));
    if !body.source_context().trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(label("context"));
        for chunk in body.source_context().lines() {
            lines.push(value(chunk.to_string()));
        }
    }
    lines
}

fn highlight_line<'a>(body: &'a CardBody, indent: &'static str) -> Line<'a> {
    let sentence = body.source_sentence();
    let highlight = body.source_highlight();
    if highlight.is_empty() {
        return Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(sentence.to_string(), palette::base()),
        ]);
    }
    if let Some(pos) = sentence.find(highlight) {
        let head = &sentence[..pos];
        let middle = &sentence[pos..pos + highlight.len()];
        let tail = &sentence[pos + highlight.len()..];
        return Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(head.to_string(), palette::base()),
            Span::styled(
                middle.to_string(),
                palette::base().add_modifier(Modifier::BOLD),
            ),
            Span::styled(tail.to_string(), palette::base()),
        ]);
    }
    Line::from(vec![
        Span::styled(indent, palette::base()),
        Span::styled(sentence.to_string(), palette::base()),
    ])
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
            return Some((
                u16::try_from(offset).unwrap_or(u16::MAX),
                u16::try_from(rows).unwrap_or(u16::MAX),
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

fn card_layout(
    draft: &CardDraft,
    running: Option<Artifact>,
    expanded: bool,
    width: usize,
) -> (usize, usize) {
    let artifacts = draft.artifacts();
    let progressed = card_progressed(artifacts, running);
    let mut rows = head_rows(draft, width);
    if progressed {
        for &(_, kind) in &STEPS {
            if slot_visible(slot_for(artifacts, kind), kind, running) {
                rows += 1;
            }
        }
    }
    if expanded {
        rows = rows.saturating_add(detail_pane_height(draft));
    }
    let trailing = if progressed || expanded { 1 } else { 0 };
    (rows, trailing)
}

/// Number of body-rect rows the expanded body-preview pane consumes for one
/// card. Verbatim mirror of `detail_pane` / `body_preview` so callers can keep
/// scroll offsets and click hit-tests aligned with the rendered output.
pub(crate) fn detail_pane_height(draft: &CardDraft) -> usize {
    let mut h = 1;
    let Some(body) = draft.body() else {
        return h + 1;
    };
    h += 2;
    h += 1 + 2;
    h += 1 + 2;
    h += 1 + 2;
    if !body.source_context().trim().is_empty() {
        h += 1 + 1;
        h += body.source_context().lines().count();
    }
    h
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
    left.push(super::common::status_sep());
    left.push(Span::styled(elapsed(app), palette::dim2()));
    let mut right: Vec<Span<'static>> = Vec::new();
    right.extend(super::common::key_hint("↑↓", "nav"));
    right.push(super::common::status_sep());
    right.extend(super::common::key_hint("Enter", "expand"));
    right.push(super::common::status_sep());
    right.extend(super::common::key_hint("R", "change"));
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}

fn elapsed(app: &App) -> String {
    let seconds = app.elapsed().as_secs();
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes:02}:{remainder:02}")
}
