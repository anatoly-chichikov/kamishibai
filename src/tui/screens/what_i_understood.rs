//! Renderer for the `what i understood` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-1.jsx` (StepUnderstood). One row
//! per word: number, term, selected-sense count, em-dash, and active sense.
//! Excluded items get a strikethrough term and
//! the explanation as the gloss so the user can see what was rejected and why.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::session::{Sense, WordCandidate};
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "what i understood";
const HINT: &str = "quick check before i build the cards";

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
                .map(|line| line.chars().count())
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
    let mut lines = Vec::new();
    for (index, candidate) in app.candidates().iter().enumerate() {
        let expanded = app.expanded_sense();
        if expanded.as_ref().map(|item| item.row) == Some(index) {
            lines.push(candidate_line(
                index,
                candidate,
                app.selected(),
                true,
                Gloss::Sentence(sense_text(candidate.sense())),
                term_width,
                width,
            ));
            let expanded = expanded
                .as_ref()
                .expect("invariant: row is the expanded one");
            for (sense_index, sense) in candidate.senses().iter().enumerate() {
                lines.push(sense_line(
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
                index,
                candidate,
                app.selected(),
                term_width,
                width,
            ));
        } else {
            lines.push(candidate_line(
                index,
                candidate,
                app.selected(),
                false,
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
pub(crate) fn content_height(app: &App) -> u16 {
    if !app.candidates().is_empty() {
        let expanded_row = app.expanded_sense().map(|item| item.row);
        let total: usize = app
            .candidates()
            .iter()
            .enumerate()
            .map(|(index, candidate)| candidate_rows(candidate, expanded_row == Some(index)))
            .sum();
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
    expanded: bool,
    gloss: Gloss,
    term_width: usize,
    width: u16,
) -> Line<'a> {
    let is_selected = index == selected && !expanded;
    let is_dimmed_parent = index == selected && expanded;
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
    } else if is_dimmed_parent {
        palette::dim()
    } else if is_selected {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::base()
    };
    let dash_style = if !candidate.ok() {
        palette::dim2()
    } else if is_selected {
        palette::highlight_dim()
    } else {
        palette::dim2()
    };
    let gloss_style = if !candidate.ok() || is_dimmed_parent {
        palette::dim()
    } else if is_selected {
        palette::highlight_dim().add_modifier(Modifier::BOLD)
    } else {
        palette::dim()
    };
    let label_width = candidate_label_len(candidate);
    let indicator = inline_indicator(candidate);
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
    let used = 4 + term_width + separator.chars().count() + gloss.chars().count();
    spans.push(Span::styled(separator, dash_style));
    spans.push(Span::styled(gloss, gloss_style));
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    Line::from(spans)
}

fn sense_line<'a>(
    index: usize,
    sense: &'a Sense,
    cursor: usize,
    selected: &[usize],
    term_width: usize,
    width: u16,
) -> Line<'a> {
    let focused = index == cursor;
    let marker = if selected.contains(&index) {
        "  ✓ "
    } else {
        "    "
    };
    let style = if focused {
        palette::highlight_dim().add_modifier(Modifier::BOLD)
    } else {
        palette::dim()
    };
    let text = sense_text(sense);
    let indent = 4 + term_width + 5;
    let used = indent + marker.chars().count() + text.chars().count();
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
pub(crate) fn candidate_rows(candidate: &WordCandidate, expanded: bool) -> usize {
    if expanded {
        candidate.senses().len().saturating_add(2)
    } else if candidate.ok() && candidate.selected_count() > 1 {
        candidate.selected_count().saturating_add(1)
    } else {
        1
    }
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
    lines.push(candidate_line(
        index,
        candidate,
        selected,
        false,
        Gloss::Heading(String::from(MEANINGS_LABEL)),
        term_width,
        width,
    ));
    for sense_index in candidate.selected_senses() {
        lines.push(selected_meaning_line(
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
fn selected_meaning_line<'a>(sense: &'a Sense, term_width: usize, width: u16) -> Line<'a> {
    let indent = 4 + term_width + 5;
    let marker = "—  ";
    let text = sense_text(sense);
    let used = indent + marker.chars().count() + text.chars().count();
    let mut spans = vec![
        Span::styled(" ".repeat(indent), palette::base()),
        Span::styled(String::from(marker), palette::dim2()),
        Span::styled(text, palette::dim()),
    ];
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), palette::base()));
    }
    Line::from(spans)
}

fn add_more_line<'a>(focused: bool, term_width: usize, width: u16) -> Line<'a> {
    let indent = 4 + term_width + 5;
    let marker = "    ";
    let text = "+ add more";
    let used = indent + marker.chars().count() + text.chars().count();
    let style = if focused {
        palette::highlight_dim().add_modifier(Modifier::BOLD)
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

fn inline_indicator(candidate: &WordCandidate) -> Option<String> {
    if candidate.ok() && candidate.has_multiple_senses() {
        return Some(format!(
            "{}/{}",
            candidate.selected_count(),
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
    let indicator = inline_indicator(candidate)
        .map(|value| 1 + value.chars().count())
        .unwrap_or(0);
    candidate.term().chars().count() + indicator
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 2/3", palette::dim2()));
    left.push(super::common::status_sep());
    let count = app
        .candidates()
        .iter()
        .filter(|candidate| candidate.ok())
        .map(WordCandidate::selected_count)
        .sum::<usize>();
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
    if app.expanded_sense().is_some() {
        let enter_label = if app.expanded_add_more_focused() {
            "add"
        } else {
            "done"
        };
        hints.push(super::common::FooterHint::primary("Space", "select"));
        hints.push(super::common::FooterHint::secondary("Enter", enter_label));
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
    } else {
        if count > 0 {
            hints.push(super::common::FooterHint::primary("Ctrl+G", "generate"));
        }
        hints.push(super::common::FooterHint::secondary("Enter", "pick"));
        hints.push(super::common::FooterHint::secondary("D", "drop"));
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
    }
    hints.push(super::common::quit_hint(app.quit_pending()));
    super::common::footer_bar(left, hints, width)
}
