//! Renderer for the `what i understood` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-1.jsx` (StepUnderstood). One row
//! per word: number, term, selected-sense count, em-dash, and active sense.
//! Excluded items get a strikethrough term and
//! the explanation as the gloss so the user can see what was rejected and why.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::session::RawInputBatch;
use crate::session::{Sense, WordCandidate};
use crate::tui::app::{App, ReviewFocus};
use crate::tui::disclosure::DisclosureControls;
use crate::tui::palette;

use super::sentence_labels::BatchEditorControl;

const HEADLINE: &str = "what i understood";
const HINT: &str = "quick check before i build the cards";
const SETTINGS_LABEL: &str = "generation guidance";
const DEFAULT_GUIDANCE_LABEL: &str = "best fit";
const ALTERNATES_LABEL: &str = "also plausible: ";
const ALTERNATES_SEPARATOR: &str = "  ·  ";

/// One clickable surface in the batch sentence-settings block.
pub(crate) enum SentenceSettingsControl {
    Open,
    Editor(BatchEditorControl),
}

/// `ScreenView` handle for the sense-check screen.
pub struct WhatIUnderstood;

impl ScreenView for WhatIUnderstood {
    fn title(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HEADLINE)
    }

    fn hint(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HINT)
    }

    fn footer(&self, app: &App, width: u16) -> Paragraph<'static> {
        footer(app, width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        frame.render_widget(body(app, area.width).scroll((app.body_scroll(), 0)), area);
    }
}

fn body(app: &App, width: u16) -> Paragraph<'_> {
    Paragraph::new(body_lines(app, width)).style(palette::base())
}

/// Build every body line of the review screen in render order.
///
/// `content_height` counts the same rows without building them, and the two
/// must agree or the scroll clamp walks past content that is really there —
/// `screen_lines_match_the_counted_height` pins that.
fn body_lines(app: &App, width: u16) -> Vec<Line<'_>> {
    if app.candidates().is_empty() {
        let typed: Vec<&str> = app
            .blob()
            .split('\n')
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        if !typed.is_empty() {
            let term_width = typed
                .iter()
                .map(|line| super::common::display_width(line))
                .max()
                .unwrap_or(12)
                .max(12);
            let lines = typed
                .iter()
                .enumerate()
                .map(|(index, line)| pending_line(index, line, term_width, width))
                .collect::<Vec<_>>();
            return lines;
        }
        let copy = if app.learning_pending() {
            "understanding your words…"
        } else {
            "nothing left to review"
        };
        return vec![Line::from(Span::styled(
            copy,
            palette::Ink::Detail.on(false),
        ))];
    }
    let term_width = candidate_label_width(app.candidates(), 12);
    let mut lines = vec![settings_summary_line(app)];
    if let Some(focused) = app.sentence_settings_editor() {
        lines.extend(super::sentence_labels::batch_editor_lines(
            app.sentence_settings(),
            focused,
            usize::from(width),
        ));
    }
    lines.extend(alternates_line(app));
    lines.push(Line::from(""));
    let selected = if app.sentence_settings_editor().is_some() {
        usize::MAX
    } else {
        match app.review_focus() {
            ReviewFocus::Head(row) => row,
            ReviewFocus::Sense { .. } => usize::MAX,
        }
    };
    for (index, candidate) in app.candidates().iter().enumerate() {
        if app.sense_list_open(index) && candidate.ok() {
            let cursor = match app.review_focus() {
                ReviewFocus::Sense {
                    row,
                    index: position,
                } if row == index => position,
                _ => usize::MAX,
            };
            lines.extend(candidate_line(
                index,
                candidate,
                selected,
                Some(candidate.selected_count()),
                head_gloss(candidate, true),
                term_width,
                width,
            ));
            for (sense_index, sense) in candidate.senses().iter().enumerate() {
                lines.extend(sense_line(
                    sense_index,
                    sense,
                    cursor,
                    candidate.selected_senses(),
                    term_width,
                    width,
                ));
            }
            lines.push(add_more_line(
                cursor == candidate.senses().len(),
                term_width,
                width,
            ));
        } else if candidate.ok() && candidate.selected_count() > 1 {
            lines.extend(candidate_block(
                index, candidate, selected, term_width, width,
            ));
        } else {
            lines.extend(candidate_line(
                index,
                candidate,
                selected,
                None,
                head_gloss(candidate, false),
                term_width,
                width,
            ));
        }
    }
    lines
}

/// Total number of lines `body` will render for the current state of `app`.
/// One row per confirmed candidate (or per typed-but-not-yet-understood line),
/// zero when neither set is populated. Used by the scroll clamp in `tui::app`.
pub(crate) fn content_height(app: &App, width: usize) -> u16 {
    if !app.candidates().is_empty() {
        let term_width = candidate_label_width(app.candidates(), 12);
        let total = candidate_content_height(app, width, term_width)
            .saturating_add(review_prefix_height(app, width));
        return u16::try_from(total).unwrap_or(u16::MAX);
    }
    let typed = RawInputBatch::new(app.blob()).word_count();
    u16::try_from(typed).unwrap_or(u16::MAX)
}

fn settings_summary_line(app: &App) -> Line<'static> {
    let settings = app.sentence_settings();
    let mut spans = vec![Span::styled(
        format!("{SETTINGS_LABEL}  "),
        palette::Ink::Detail.on(false),
    )];
    if app.sentence_settings_editor().is_some() {
        return Line::from(spans);
    }
    if settings.level().is_none() && !settings.types().pins() {
        spans.push(Span::styled(
            format!(" {DEFAULT_GUIDANCE_LABEL} "),
            super::sentence_labels::tag_style(false),
        ));
        return Line::from(spans);
    }
    if let Some(level) = settings.level() {
        spans.push(Span::styled(
            format!(" {} ", level.token()),
            super::sentence_labels::tag_style(true),
        ));
    }
    if settings.types().pins() {
        if settings.level().is_some() {
            spans.push(Span::styled(" ", palette::base()));
        }
        spans.push(Span::styled(
            format!(" {} ", settings.types().token()),
            super::sentence_labels::tag_style(true),
        ));
    }
    Line::from(spans)
}

/// Name the learning languages the pass judged equally plausible for this
/// batch, each one a click away from re-reading the words as that language.
///
/// Rendered directly below the guidance summary — or below the guidance editor
/// while that is open, so the editor stays glued to the row that opens it.
fn alternates_line(app: &App) -> Option<Line<'static>> {
    let codes = app.alternates();
    if codes.is_empty() {
        return None;
    }
    let mut spans = vec![Span::styled(
        ALTERNATES_LABEL,
        palette::Ink::Aside.on(false),
    )];
    for (index, code) in codes.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(ALTERNATES_SEPARATOR, palette::rule()));
        }
        spans.push(Span::styled(
            code.to_uppercase(),
            palette::Ink::Detail.link(false),
        ));
    }
    Some(Line::from(spans))
}

/// Return the index of the alternate language covering one body-content cell.
pub(crate) fn alternate_at(app: &App, width: usize, column: usize, row: usize) -> Option<usize> {
    if app.candidates().is_empty() || app.alternates().is_empty() {
        return None;
    }
    if row != editor_height(app, width).saturating_add(1) {
        return None;
    }
    let mut start = super::common::display_width(ALTERNATES_LABEL);
    for (index, code) in app.alternates().iter().enumerate() {
        if index > 0 {
            start = start.saturating_add(super::common::display_width(ALTERNATES_SEPARATOR));
        }
        let end = start.saturating_add(super::common::display_width(code));
        if column >= start && column < end {
            return Some(index);
        }
        start = end;
    }
    None
}

fn editor_height(app: &App, width: usize) -> usize {
    app.sentence_settings_editor()
        .map(|focused| {
            super::sentence_labels::batch_editor_lines(app.sentence_settings(), focused, width)
                .len()
        })
        .unwrap_or(0)
}

fn review_prefix_height(app: &App, width: usize) -> usize {
    2usize
        .saturating_add(editor_height(app, width))
        .saturating_add(usize::from(!app.alternates().is_empty()))
}

fn candidate_content_height(app: &App, width: usize, term_width: usize) -> usize {
    app.candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            candidate_rows(
                candidate,
                app.sense_list_open(index) && candidate.ok(),
                term_width,
                width,
            )
        })
        .sum()
}

/// Return the batch sentence-settings control occupying one body-content cell.
pub(crate) fn sentence_settings_control_at(
    app: &App,
    width: usize,
    column: usize,
    row: usize,
) -> Option<SentenceSettingsControl> {
    if app.candidates().is_empty() {
        return None;
    }
    if row == 0 {
        let summary_width = settings_summary_line(app).width();
        return (column < summary_width).then_some(SentenceSettingsControl::Open);
    }
    let focused = app.sentence_settings_editor()?;
    let editor_row = row.checked_sub(1)?;
    super::sentence_labels::batch_editor_control_at(
        app.sentence_settings(),
        focused,
        width,
        column,
        editor_row,
    )
    .map(SentenceSettingsControl::Editor)
}

fn pending_line<'a>(index: usize, raw: &'a str, term_width: usize, width: u16) -> Line<'a> {
    let term = super::common::pad_right(raw, term_width);
    let used = 4 + term_width;
    let pad = (width as usize).saturating_sub(used);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(
        format!("{:0>2}  ", index + 1),
        palette::Ink::Aside.on(false),
    ));
    spans.push(Span::styled(term, palette::Ink::Detail.on(false)));
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), palette::base()));
    }
    Line::from(spans)
}

/// What fills the gloss column of a candidate row: an active-sense `Sentence`
/// introduced by an em-dash, a plain `Heading` label (the multi-meaning header)
/// with no dash — a label is not a sentence, so it carries no dash — or
/// `Silent`, the column deliberately left empty because everything it could say
/// is already listed underneath.
enum Gloss {
    Sentence(String),
    Heading(String),
    Silent,
}

impl Gloss {
    /// Width of the separator this gloss is introduced by.
    fn separator(&self) -> &'static str {
        match self {
            Self::Sentence(_) => "  —  ",
            Self::Heading(_) => "  ",
            Self::Silent => "",
        }
    }

    /// The text this gloss puts in the column, empty when it says nothing.
    fn text(&self) -> &str {
        match self {
            Self::Sentence(text) | Self::Heading(text) => text.as_str(),
            Self::Silent => "",
        }
    }
}

/// Return what the head row of one candidate says in its gloss column.
///
/// An open list already enumerates every sense underneath the head, so the head
/// stops repeating one of them: it carries the meanings heading when there is
/// more than one sense to choose between — the same heading a collapsed word
/// with several chosen meanings shows — and says nothing at all when the list
/// below holds a single row. The heading therefore appears exactly when the
/// `X/Y` counter does.
fn head_gloss(candidate: &WordCandidate, open: bool) -> Gloss {
    if open {
        return if candidate.has_multiple_senses() {
            Gloss::Heading(String::from(MEANINGS_LABEL))
        } else {
            Gloss::Silent
        };
    }
    if candidate.ok() && candidate.selected_count() > 1 {
        return Gloss::Heading(String::from(MEANINGS_LABEL));
    }
    Gloss::Sentence(sense_text(candidate.sense()))
}

fn candidate_line<'a>(
    index: usize,
    candidate: &'a WordCandidate,
    selected: usize,
    expanded_count: Option<usize>,
    gloss: Gloss,
    term_width: usize,
    width: u16,
) -> Vec<Line<'a>> {
    let is_selected = index == selected;
    let row_style = palette::Ink::Detail.on(is_selected);
    let num_style = palette::Ink::Aside.on(is_selected);
    let term_style = if candidate.ok() {
        palette::Ink::Subject.on(is_selected)
    } else {
        palette::Ink::Detail
            .on(is_selected)
            .add_modifier(Modifier::CROSSED_OUT)
    };
    let dash_style = palette::Ink::Aside.on(is_selected);
    let gloss_style = palette::Ink::Detail.on(is_selected);
    let label_width = candidate_label_len(candidate);
    let indicator = inline_indicator(candidate, expanded_count);
    let pad_after_label = term_width.saturating_sub(label_width);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(format!("{:0>2}  ", index + 1), num_style));
    spans.push(Span::styled(candidate.term().to_string(), term_style));
    if let Some(indicator) = indicator {
        spans.push(Span::styled(format!(" {indicator}"), dash_style));
    }
    if pad_after_label > 0 {
        spans.push(Span::styled(" ".repeat(pad_after_label), row_style));
    }
    let separator = gloss.separator();
    let gloss = String::from(gloss.text());
    let gloss_start = 4 + term_width + super::common::display_width(separator);
    let available = (width as usize).saturating_sub(gloss_start).max(1);
    let chunks = super::common::wrap_words(gloss.as_str(), available, available);
    let first = chunks.first().cloned().unwrap_or_default();
    spans.push(Span::styled(separator, dash_style));
    spans.push(Span::styled(first.clone(), gloss_style));
    let used = gloss_start + super::common::display_width(first.as_str());
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    let mut lines = vec![Line::from(spans)];
    for chunk in chunks.into_iter().skip(1) {
        let used = gloss_start + super::common::display_width(chunk.as_str());
        let pad = (width as usize).saturating_sub(used);
        let mut spans = vec![
            Span::styled(" ".repeat(gloss_start), row_style),
            Span::styled(chunk, gloss_style),
        ];
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn sense_line<'a>(
    index: usize,
    sense: &'a Sense,
    cursor: usize,
    selected: &[usize],
    term_width: usize,
    width: u16,
) -> Vec<Line<'a>> {
    let focused = index == cursor;
    let checked = selected.contains(&index);
    let marker = if checked { "  ✓ " } else { "    " };
    let style = if checked {
        palette::Ink::Detail.on(focused)
    } else {
        palette::Ink::Aside.on(focused)
    };
    let text = sense_text(sense);
    let indent = 4 + term_width + 5;
    marked_lines(marker, text.as_str(), indent, style, style, width)
}

fn sense_text(sense: &Sense) -> String {
    match sense.tag() {
        Some(tag) => format!("[{tag}] {}", sense.understanding()),
        None => sense.understanding().to_string(),
    }
}

/// Label shown in the gloss column of a multi-meaning header row, in place of a
/// single understanding sentence. The selected meanings are listed beneath it.
const MEANINGS_LABEL: &str = "multiple meanings:";

/// Number of body rows one candidate occupies, so `content_height` and the
/// scroll clamp agree with `body`. Expanded → header + one row per sense +
/// add-more; a collapsed word with several selected meanings → header + one row
/// per selected meaning; otherwise a single line.
pub(crate) fn candidate_rows(
    candidate: &WordCandidate,
    expanded: bool,
    term_width: usize,
    width: usize,
) -> usize {
    let head = candidate_line_rows(
        candidate,
        &head_gloss(candidate, expanded),
        term_width,
        width,
    );
    if expanded {
        let senses = candidate
            .senses()
            .iter()
            .map(|sense| sense_rows(sense, term_width, width))
            .sum::<usize>();
        return head.saturating_add(senses).saturating_add(1);
    }
    if candidate.ok() && candidate.selected_count() > 1 {
        let selected = candidate
            .selected_senses()
            .iter()
            .map(|index| selected_meaning_rows(&candidate.senses()[*index], term_width, width))
            .sum::<usize>();
        return head.saturating_add(selected);
    }
    head
}

fn candidate_line_rows(
    candidate: &WordCandidate,
    gloss: &Gloss,
    term_width: usize,
    width: usize,
) -> usize {
    let separator_width = super::common::display_width(gloss.separator());
    let label_width = candidate_label_len(candidate);
    let gloss_start = 4 + term_width.max(label_width) + separator_width;
    let available = width.saturating_sub(gloss_start).max(1);
    super::common::wrap_words(gloss.text(), available, available)
        .len()
        .max(1)
}

fn sense_rows(sense: &Sense, term_width: usize, width: usize) -> usize {
    marked_line_rows(sense_text(sense).as_str(), 4 + term_width + 5, 4, width)
}

fn selected_meaning_rows(sense: &Sense, term_width: usize, width: usize) -> usize {
    marked_line_rows(sense_text(sense).as_str(), 4 + term_width + 5, 3, width)
}

fn marked_line_rows(text: &str, indent: usize, marker_width: usize, width: usize) -> usize {
    let start = indent + marker_width;
    let available = width.saturating_sub(start).max(1);
    super::common::wrap_words(text, available, available).len()
}

/// Return the focused review block range for scroll snapping.
pub(crate) fn focused_range(app: &App, width: usize) -> Option<(u16, u16)> {
    if app.candidates().is_empty() {
        return None;
    }
    let term_width = candidate_label_width(app.candidates(), 12);
    if let Some(focused) = app.sentence_settings_editor() {
        let (start, height) = super::sentence_labels::batch_editor_focus_range(
            app.sentence_settings(),
            focused,
            width,
        );
        return Some((
            u16::try_from(1usize.saturating_add(start)).unwrap_or(u16::MAX),
            u16::try_from(height).unwrap_or(u16::MAX),
        ));
    }
    let focus = app.review_focus();
    let target = focus.row().min(app.candidates().len() - 1);
    let mut offset = review_prefix_height(app, width);
    for (index, candidate) in app.candidates().iter().enumerate() {
        let open = app.sense_list_open(index) && candidate.ok();
        if index != target {
            offset = offset.saturating_add(candidate_rows(candidate, open, term_width, width));
            continue;
        }
        if open && let ReviewFocus::Sense { index: cursor, .. } = focus {
            offset = offset.saturating_add(candidate_line_rows(
                candidate,
                &head_gloss(candidate, true),
                term_width,
                width,
            ));
            for sense in candidate.senses().iter().take(cursor) {
                offset = offset.saturating_add(sense_rows(sense, term_width, width));
            }
            let height = if cursor >= candidate.senses().len() {
                1
            } else {
                sense_rows(&candidate.senses()[cursor], term_width, width)
            };
            return Some((
                u16::try_from(offset).unwrap_or(u16::MAX),
                u16::try_from(height).unwrap_or(u16::MAX),
            ));
        }
        let height = if open {
            candidate_line_rows(candidate, &head_gloss(candidate, true), term_width, width)
        } else {
            candidate_rows(candidate, false, term_width, width)
        };
        return Some((
            u16::try_from(offset).unwrap_or(u16::MAX),
            u16::try_from(height).unwrap_or(u16::MAX),
        ));
    }
    Some((u16::try_from(offset).unwrap_or(u16::MAX), 1))
}

/// Render a collapsed word that has several selected meanings: a header row
/// (`NN  term  X/Y  multiple meanings:`, a label and so introduced by no dash)
/// followed by one dimmed, read-only line per selected meaning. Navigation
/// never lands on the meaning rows.
fn candidate_block<'a>(
    index: usize,
    candidate: &'a WordCandidate,
    selected: usize,
    term_width: usize,
    width: u16,
) -> Vec<Line<'a>> {
    let mut lines = Vec::with_capacity(candidate.selected_count().saturating_add(1));
    lines.extend(candidate_line(
        index,
        candidate,
        selected,
        None,
        head_gloss(candidate, false),
        term_width,
        width,
    ));
    for sense_index in candidate.selected_senses() {
        lines.extend(selected_meaning_line(
            &candidate.senses()[*sense_index],
            term_width,
            width,
        ));
    }
    lines
}

/// One dimmed, read-only meaning line under a multi-meaning header. Led by an
/// em-dash indented one gloss column past the top-level dash, so the meanings
/// read as nested under the word rather than as peers of the other rows.
fn selected_meaning_line<'a>(sense: &'a Sense, term_width: usize, width: u16) -> Vec<Line<'a>> {
    let indent = 4 + term_width + 5;
    let marker = "—  ";
    let text = sense_text(sense);
    marked_lines(
        marker,
        text.as_str(),
        indent,
        palette::Ink::Detail.on(false),
        palette::base(),
        width,
    )
}

fn marked_lines<'a>(
    marker: &'static str,
    text: &str,
    indent: usize,
    style: Style,
    pad_style: Style,
    width: u16,
) -> Vec<Line<'a>> {
    let start = indent + super::common::display_width(marker);
    let available = (width as usize).saturating_sub(start).max(1);
    let chunks = super::common::wrap_words(text, available, available);
    let mut lines = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.into_iter().enumerate() {
        let row_marker = if index == 0 { marker } else { "" };
        let row_indent = if index == 0 { indent } else { start };
        let row_start = row_indent + super::common::display_width(row_marker);
        let used = row_start + super::common::display_width(chunk.as_str());
        let pad = (width as usize).saturating_sub(used);
        let mut spans = vec![
            Span::styled(" ".repeat(row_indent), pad_style),
            Span::styled(String::from(row_marker), style),
            Span::styled(chunk, style),
        ];
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), pad_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn add_more_line<'a>(focused: bool, term_width: usize, width: u16) -> Line<'a> {
    let indent = 4 + term_width + 5;
    let marker = "    ";
    let text = "+ add more";
    let used = indent + super::common::display_width(marker) + super::common::display_width(text);
    let style = palette::Ink::Detail.on(focused);
    let mut spans = vec![
        Span::styled(" ".repeat(indent), style),
        Span::styled(marker, style),
        Span::styled(text, style),
    ];
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), style));
    }
    Line::from(spans)
}

fn inline_indicator(candidate: &WordCandidate, selected_count: Option<usize>) -> Option<String> {
    if candidate.ok() && candidate.has_multiple_senses() {
        return Some(format!(
            "{}/{}",
            selected_count.unwrap_or_else(|| candidate.selected_count()),
            candidate.senses().len()
        ));
    }
    None
}

fn candidate_label_width(candidates: &[WordCandidate], minimum: usize) -> usize {
    candidates
        .iter()
        .map(candidate_label_len)
        .max()
        .unwrap_or(minimum)
        .max(minimum)
}

fn candidate_label_len(candidate: &WordCandidate) -> usize {
    let indicator = inline_indicator(candidate, None)
        .map(|value| 1 + super::common::display_width(value.as_str()))
        .unwrap_or(0);
    super::common::display_width(candidate.term()) + indicator
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 2/3", palette::Ink::Aside.on(false)));
    left.push(super::common::status_sep());
    let count = review_card_count(app);
    if count > 0 {
        left.push(Span::styled(
            count.to_string(),
            palette::Ink::Subject.on(false),
        ));
        let noun = if count == 1 { "card" } else { "cards" };
        left.push(Span::styled(
            format!(" {noun}"),
            palette::Ink::Detail.on(false),
        ));
    } else {
        left.push(Span::styled(
            "nothing to make",
            palette::Ink::Aside.on(false),
        ));
    }
    if let Some(message) = app.review_notice() {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            message.to_string(),
            palette::Ink::Detail.on(false),
        ));
    }
    let mut hints: Vec<super::common::FooterHint> = Vec::new();
    if app.sentence_settings_editor().is_some() {
        if count > 0 {
            hints.push(super::common::FooterHint::primary("Ctrl+G", "generate"));
        }
        hints.push(super::common::FooterHint::secondary("← →", "pick"));
        hints.push(super::common::FooterHint::ghost("↑ ↓", "row"));
        hints.push(super::common::FooterHint::ghost("Enter/Esc", "close"));
    } else if matches!(app.review_focus(), ReviewFocus::Sense { .. }) {
        let controls = if app.expanded_add_more_focused() {
            DisclosureControls::new(true).with_action("add")
        } else {
            DisclosureControls::new(true).with_action("select")
        };
        if let Some(hint) = controls.primary_action() {
            hints.push(hint);
        }
        if count > 0 {
            hints.push(super::common::FooterHint::secondary("Ctrl+G", "generate"));
        }
        hints.push(controls.secondary_toggle());
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
        hints.push(super::common::FooterHint::secondary("C", "collapse"));
    } else {
        if count > 0 {
            hints.push(super::common::FooterHint::primary("Ctrl+G", "generate"));
        }
        if !app.candidates().is_empty() && app.selected() == 0 {
            hints.push(super::common::sentence_settings_hint());
        }
        hints.push(DisclosureControls::new(false).secondary_toggle());
        hints.push(super::common::back_hint());
        hints.push(super::common::FooterHint::secondary("D", "drop"));
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
        if app.any_sense_list_open() {
            hints.push(super::common::FooterHint::secondary("C", "collapse"));
        } else if app
            .candidates()
            .iter()
            .any(|candidate| candidate.ok() && candidate.has_multiple_senses())
        {
            hints.push(super::common::FooterHint::ghost("C", "expand"));
        }
        hints.push(super::common::FooterHint::ghost("Ctrl+L", "languages"));
    }
    if app.sentence_settings_editor().is_none() {
        hints.push(super::common::quit_hint(app.quit_pending()));
    }
    super::common::footer_bar(left, hints, width)
}

fn review_card_count(app: &App) -> usize {
    app.candidates()
        .iter()
        .filter(|candidate| candidate.ok())
        .map(WordCandidate::selected_count)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::LanguagePair;
    use crate::tui::Screen;

    fn wordy() -> WordCandidate {
        WordCandidate::with_senses(
            "bank",
            vec![
                Sense::plain(
                    "A financial institution that takes deposits from the public, lends them out at interest, keeps its customers' current accounts, and stores their valuables in a guarded vault downstairs.",
                ),
                Sense::plain("The sloping side of a river."),
                Sense::tagged("To tilt an aircraft into a turn.", "aviation"),
            ],
            0,
            true,
        )
    }

    fn review(candidates: Vec<WordCandidate>) -> App {
        App::new(LanguagePair::new("en", "ru"))
            .with_screen(Screen::WhatIUnderstood)
            .confirmed_learning("en")
            .understood(candidates)
    }

    #[test]
    fn screen_lines_match_the_counted_height() {
        let single = WordCandidate::new("bittersweet", "Both glad and sad at once.", true);
        let shapes = [
            review(vec![wordy()]),
            review(vec![wordy()]).sense_list_toggled(),
            review(vec![single.clone()]),
            review(vec![single]).sense_list_toggled(),
        ];
        let counted = shapes
            .iter()
            .map(|app| {
                (
                    body_lines(app, 90).len(),
                    usize::from(content_height(app, 90)),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            counted.iter().all(|(drawn, counted)| drawn == counted),
            "the renderer and the scroll clamp disagreed on a review shape, got {counted:?}"
        );
    }
}
