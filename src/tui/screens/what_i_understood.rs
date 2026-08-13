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
use crate::session::{Sense, WordCandidate};
use crate::tui::app::App;
use crate::tui::disclosure::DisclosureControls;
use crate::tui::palette;

use super::sentence_labels::BatchEditorControl;

const HEADLINE: &str = "what i understood";
const HINT: &str = "quick check before i build the cards";
const SETTINGS_LABEL: &str = "generation guidance";
const DEFAULT_GUIDANCE_LABEL: &str = "best fit";

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
            return Paragraph::new(lines).style(palette::base());
        }
        let copy = if app.learning_pending() {
            "understanding your words…"
        } else {
            "nothing left to review"
        };
        return Paragraph::new(Line::from(Span::styled(copy, palette::dim())))
            .style(palette::base());
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
    lines.push(Line::from(""));
    let selected = if app.sentence_settings_editor().is_some() {
        usize::MAX
    } else {
        app.selected()
    };
    for (index, candidate) in app.candidates().iter().enumerate() {
        let expanded = app.expanded_sense();
        if expanded.as_ref().map(|item| item.row) == Some(index) {
            let expanded = expanded
                .as_ref()
                .expect("invariant: row is the expanded one");
            lines.extend(candidate_line(
                index,
                candidate,
                selected,
                Some(expanded.selected.len()),
                Gloss::Sentence(sense_text(candidate.sense())),
                term_width,
                width,
            ));
            for (sense_index, sense) in candidate.senses().iter().enumerate() {
                lines.extend(sense_line(
                    sense_index,
                    sense,
                    expanded.cursor,
                    &expanded.selected,
                    term_width,
                    width,
                ));
            }
            lines.push(add_more_line(
                expanded.cursor == candidate.senses().len(),
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
                Gloss::Sentence(sense_text(candidate.sense())),
                term_width,
                width,
            ));
        }
    }
    Paragraph::new(lines).style(palette::base())
}

/// Total number of lines `body` will render for the current state of `app`.
/// One row per confirmed candidate (or per typed-but-not-yet-understood line),
/// zero when neither set is populated. Used by the scroll clamp in `tui::app`.
pub(crate) fn content_height(app: &App, width: usize) -> u16 {
    if !app.candidates().is_empty() {
        let expanded_row = app.expanded_sense().map(|item| item.row);
        let term_width = candidate_label_width(app.candidates(), 12);
        let total = candidate_content_height(app, width, expanded_row, term_width)
            .saturating_add(review_prefix_height(app, width));
        return u16::try_from(total).unwrap_or(u16::MAX);
    }
    let typed = app
        .blob()
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    u16::try_from(typed).unwrap_or(u16::MAX)
}

fn settings_summary_line(app: &App) -> Line<'static> {
    let settings = app.sentence_settings();
    let mut spans = vec![Span::styled(format!("{SETTINGS_LABEL}  "), palette::dim())];
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

fn review_prefix_height(app: &App, width: usize) -> usize {
    2usize.saturating_add(
        app.sentence_settings_editor()
            .map(|focused| {
                super::sentence_labels::batch_editor_lines(app.sentence_settings(), focused, width)
                    .len()
            })
            .unwrap_or(0),
    )
}

fn candidate_content_height(
    app: &App,
    width: usize,
    expanded_row: Option<usize>,
    term_width: usize,
) -> usize {
    app.candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            candidate_rows(candidate, expanded_row == Some(index), term_width, width)
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
        return (app.expanded_sense().is_none() && column < summary_width)
            .then_some(SentenceSettingsControl::Open);
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
        palette::dim2(),
    ));
    spans.push(Span::styled(term, palette::dim()));
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), palette::base()));
    }
    Line::from(spans)
}

/// What fills the gloss column of a candidate row: an active-sense `Sentence`
/// introduced by an em-dash, or a plain `Heading` label (the multi-meaning
/// header) with no dash — a label is not a sentence, so it carries no dash.
enum Gloss {
    Sentence(String),
    Heading(String),
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
    let expanded = expanded_count.is_some();
    let is_selected = index == selected && !expanded;
    let is_expanded_parent = index == selected && expanded;
    let row_style = if is_selected {
        palette::highlight()
    } else {
        palette::base()
    };
    let num_style = if is_selected {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::dim2()
    };
    let term_style = if !candidate.ok() {
        palette::dim().add_modifier(Modifier::CROSSED_OUT)
    } else if is_expanded_parent {
        palette::base()
    } else if is_selected {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::base()
    };
    let dash_style = if !candidate.ok() {
        palette::dim2()
    } else if is_expanded_parent {
        palette::dim()
    } else if is_selected {
        palette::highlight_dim()
    } else {
        palette::dim2()
    };
    let gloss_style = if !candidate.ok() || is_expanded_parent {
        palette::dim()
    } else if is_selected {
        palette::highlight_dim().add_modifier(Modifier::BOLD)
    } else {
        palette::dim()
    };
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
    let (separator, gloss) = match gloss {
        Gloss::Sentence(text) => ("  —  ", text),
        Gloss::Heading(text) => ("  ", text),
    };
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
    let style = if focused {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else if checked {
        palette::base()
    } else {
        palette::dim()
    };
    let text = sense_text(sense);
    let indent = 4 + term_width + 5;
    let pad_style = if focused {
        palette::highlight()
    } else {
        palette::base()
    };
    marked_lines(marker, text.as_str(), indent, style, pad_style, width)
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
    if expanded {
        let parent = candidate_line_rows(
            candidate,
            GlossKind::Sentence,
            sense_text(candidate.sense()).as_str(),
            term_width,
            width,
        );
        let senses = candidate
            .senses()
            .iter()
            .map(|sense| sense_rows(sense, term_width, width))
            .sum::<usize>();
        parent.saturating_add(senses).saturating_add(1)
    } else if candidate.ok() && candidate.selected_count() > 1 {
        let header = candidate_line_rows(
            candidate,
            GlossKind::Heading,
            MEANINGS_LABEL,
            term_width,
            width,
        );
        let selected = candidate
            .selected_senses()
            .iter()
            .map(|index| selected_meaning_rows(&candidate.senses()[*index], term_width, width))
            .sum::<usize>();
        header.saturating_add(selected)
    } else {
        candidate_line_rows(
            candidate,
            GlossKind::Sentence,
            sense_text(candidate.sense()).as_str(),
            term_width,
            width,
        )
    }
}

enum GlossKind {
    Sentence,
    Heading,
}

fn candidate_line_rows(
    candidate: &WordCandidate,
    kind: GlossKind,
    gloss: &str,
    term_width: usize,
    width: usize,
) -> usize {
    let separator_width = match kind {
        GlossKind::Sentence => 5,
        GlossKind::Heading => 2,
    };
    let label_width = candidate_label_len(candidate);
    let gloss_start = 4 + term_width.max(label_width) + separator_width;
    let available = width.saturating_sub(gloss_start).max(1);
    super::common::wrap_words(gloss, available, available).len()
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
    let expanded_row = app.expanded_sense().map(|item| item.row);
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
    let selected = app.selected().min(app.candidates().len() - 1);
    let mut offset = review_prefix_height(app, width);
    for (index, candidate) in app.candidates().iter().enumerate() {
        if index != selected {
            offset = offset.saturating_add(candidate_rows(
                candidate,
                expanded_row == Some(index),
                term_width,
                width,
            ));
            continue;
        }
        if let Some(expanded) = app.expanded_sense()
            && expanded.row == selected
        {
            offset = offset.saturating_add(candidate_line_rows(
                candidate,
                GlossKind::Sentence,
                sense_text(candidate.sense()).as_str(),
                term_width,
                width,
            ));
            for sense in candidate.senses().iter().take(expanded.cursor) {
                offset = offset.saturating_add(sense_rows(sense, term_width, width));
            }
            let height = if expanded.cursor == candidate.senses().len() {
                1
            } else {
                sense_rows(&candidate.senses()[expanded.cursor], term_width, width)
            };
            return Some((
                u16::try_from(offset).unwrap_or(u16::MAX),
                u16::try_from(height).unwrap_or(u16::MAX),
            ));
        }
        let height = candidate_rows(candidate, false, term_width, width);
        return Some((
            u16::try_from(offset).unwrap_or(u16::MAX),
            u16::try_from(height).unwrap_or(u16::MAX),
        ));
    }
    Some((u16::try_from(offset).unwrap_or(u16::MAX), 1))
}

/// Render a collapsed word that has several selected meanings: a header row
/// (`NN  term  X/Y  —  multiple meanings:`) followed by one dimmed, read-only
/// line per selected meaning. Navigation never lands on the meaning rows.
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
        Gloss::Heading(String::from(MEANINGS_LABEL)),
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
        palette::dim(),
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
    let style = if focused {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::dim()
    };
    let pad_style = if focused {
        palette::highlight()
    } else {
        palette::base()
    };
    let mut spans = vec![
        Span::styled(" ".repeat(indent), palette::base()),
        Span::styled(marker, style),
        Span::styled(text, style),
    ];
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), pad_style));
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
    left.push(Span::styled("step 2/3", palette::dim2()));
    left.push(super::common::status_sep());
    let count = review_card_count(app);
    if count > 0 {
        left.push(Span::styled(
            count.to_string(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
        let noun = if count == 1 { "card" } else { "cards" };
        left.push(Span::styled(format!(" {noun}"), palette::dim()));
    } else {
        left.push(Span::styled("nothing to make", palette::dim2()));
    }
    if let Some(message) = app.review_notice() {
        left.push(super::common::status_sep());
        left.push(Span::styled(message.to_string(), palette::dim()));
    }
    let mut hints: Vec<super::common::FooterHint> = Vec::new();
    if app.sentence_settings_editor().is_some() {
        if count > 0 {
            hints.push(super::common::FooterHint::primary("Ctrl+G", "generate"));
        }
        hints.push(super::common::FooterHint::secondary("← →", "pick"));
        hints.push(super::common::FooterHint::ghost("↑ ↓", "row"));
        hints.push(super::common::FooterHint::ghost("Esc", "close"));
    } else if app.expanded_sense().is_some() {
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
    }
    if app.sentence_settings_editor().is_none() {
        hints.push(super::common::quit_hint(app.quit_pending()));
    }
    super::common::footer_bar(left, hints, width)
}

fn review_card_count(app: &App) -> usize {
    let expanded = app.expanded_sense();
    app.candidates()
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.ok())
        .map(|(row, candidate)| {
            expanded
                .as_ref()
                .filter(|selection| selection.row == row)
                .map_or_else(
                    || candidate.selected_count(),
                    |selection| selection.selected.len(),
                )
        })
        .sum()
}
