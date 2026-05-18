//! Renderer for the `what i understood` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-1.jsx` (StepUnderstood). One row
//! per word: number, term, em-dash, and the human-language understanding the
//! model produced. Excluded items (kind=Skipped) get a strikethrough term and
//! the explanation as the gloss so the user can see what was rejected and why.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::session::WordCandidate;
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
        let copy = if app.target_pending() {
            "understanding your words…"
        } else {
            "nothing left to review"
        };
        return Paragraph::new(Line::from(Span::styled(copy, palette::dim())))
            .style(palette::base());
    }
    let term_width = padded_width(app.candidates(), |candidate| candidate.term(), 12);
    let lines = app
        .candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            candidate_line(index, candidate, app.selected(), term_width, width)
        })
        .collect::<Vec<_>>();
    Paragraph::new(lines).style(palette::base())
}

/// Total number of lines `body` will render for the current state of `app`.
/// One row per confirmed candidate (or per typed-but-not-yet-understood line),
/// zero when neither set is populated. Used by the scroll clamp in `tui::app`.
pub(crate) fn content_height(app: &App) -> u16 {
    if !app.candidates().is_empty() {
        return u16::try_from(app.candidates().len()).unwrap_or(u16::MAX);
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

fn candidate_line<'a>(
    index: usize,
    candidate: &'a WordCandidate,
    selected: usize,
    term_width: usize,
    width: u16,
) -> Line<'a> {
    let is_selected = index == selected;
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
    let gloss_style = if !candidate.ok() {
        palette::dim()
    } else if is_selected {
        palette::highlight_dim().add_modifier(Modifier::BOLD)
    } else {
        palette::dim()
    };
    let gloss = candidate.understanding().to_string();
    let term = super::common::pad_right(candidate.term(), term_width);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(format!("{:0>2}  ", index + 1), num_style));
    spans.push(Span::styled(term, term_style));
    spans.push(Span::styled("  —  ", dash_style));
    spans.push(Span::styled(gloss.clone(), gloss_style));
    let used = 4 + term_width + 5 + gloss.chars().count();
    let pad = (width as usize).saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    Line::from(spans)
}

fn padded_width<F>(candidates: &[WordCandidate], value: F, minimum: usize) -> usize
where
    F: Fn(&WordCandidate) -> &str,
{
    candidates
        .iter()
        .map(value)
        .map(|item| item.chars().count())
        .max()
        .unwrap_or(minimum)
        .max(minimum)
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 2/3", palette::dim2()));
    left.push(super::common::status_sep());
    let count = app
        .candidates()
        .iter()
        .filter(|candidate| candidate.ok())
        .count();
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
    let mut right: Vec<Span<'static>> = Vec::new();
    right.extend(super::common::key_hint("↑↓", "nav"));
    right.push(super::common::status_sep());
    right.extend(super::common::key_hint("D", "drop"));
    right.push(super::common::status_sep());
    right.extend(super::common::key_hint("Enter", "refine"));
    right.push(super::common::status_sep());
    right.extend(super::common::key_hint("Ctrl+G", "generate"));
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}
