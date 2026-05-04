//! Renderer for the `your words` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-1.jsx` (StepWords). Line-numbered
//! gutter on the left, the typed blob in the middle, the active line lit by
//! a soft `--hl` background, and the chrome status bar pinned to the bottom.
//! The terminal cursor is parked on the active line so the host terminal
//! handles its own blink natively.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "words you want to learn";
const HINT: &str = "each line becomes one anki card";
const PLACEHOLDER_LINES: usize = 4;
const PLACEHOLDER_HINT: &str = "type or paste, one item per line";

/// `ScreenView` handle for the `your words` screen.
pub struct YourWords;

impl ScreenView for YourWords {
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
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(4), Constraint::Min(1)])
            .split(area);
        let lines = body_lines(app, columns[1].width);
        let active_row = active_line_index(app);
        if !app.blob().is_empty()
            && let Some(strip) = highlight_strip(area, active_row)
        {
            paint_strip(frame, strip);
        }
        frame.render_widget(gutter(app, area.height as usize), columns[0]);
        frame.render_widget(Paragraph::new(lines).style(palette::base()), columns[1]);
        place_cursor(frame, app, columns[1]);
    }
}

fn place_cursor(frame: &mut Frame, app: &App, body: Rect) {
    let row = if app.blob().is_empty() {
        0
    } else {
        active_line_index(app) as u16
    };
    let column = if app.blob().is_empty() {
        0
    } else {
        let active = active_line_index(app);
        app.blob()
            .split('\n')
            .nth(active)
            .map(|line| line.chars().count() as u16)
            .unwrap_or(0)
    };
    let cursor_x = body.x + column.min(body.width.saturating_sub(1));
    let cursor_y = body.y + row.min(body.height.saturating_sub(1));
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn active_line_index(app: &App) -> usize {
    if app.blob().is_empty() {
        return 0;
    }
    app.blob().split('\n').count().saturating_sub(1)
}

fn highlight_strip(body: Rect, row: usize) -> Option<Rect> {
    let row = row as u16;
    if row >= body.height {
        return None;
    }
    Some(Rect {
        x: body.x,
        y: body.y + row,
        width: body.width,
        height: 1,
    })
}

fn paint_strip(frame: &mut Frame, area: Rect) {
    let filler = " ".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(filler, palette::highlight()))),
        area,
    );
}

fn gutter(app: &App, available: usize) -> Paragraph<'static> {
    let actual = if app.blob().is_empty() {
        0
    } else {
        app.blob().split('\n').count()
    };
    let visible = available.min(actual.max(PLACEHOLDER_LINES));
    let active = active_line_index(app);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible);
    for index in 0..visible {
        let label = format!("{:0>2} ", index + 1);
        let style = if index >= actual {
            palette::base().fg(palette::RULE)
        } else if index == active && !app.blob().is_empty() {
            palette::highlight().add_modifier(Modifier::BOLD)
        } else {
            palette::dim2()
        };
        lines.push(Line::from(Span::styled(label, style)));
    }
    Paragraph::new(lines).style(palette::base())
}

fn body_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if app.blob().is_empty() {
        return placeholder_lines();
    }
    let active = active_line_index(app);
    app.blob()
        .split('\n')
        .enumerate()
        .map(|(index, raw)| line_for_row(index, raw, active, width))
        .collect()
}

fn line_for_row(index: usize, raw: &str, active: usize, width: u16) -> Line<'static> {
    let style = if index == active {
        palette::highlight()
    } else {
        palette::base()
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(String::from(raw), style));
    if index == active {
        let used = raw.chars().count();
        let pad = (width as usize).saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), palette::highlight()));
        }
    }
    Line::from(spans)
}

fn placeholder_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(PLACEHOLDER_HINT, palette::dim2())),
    ]
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let count = word_count(app.blob());
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 1/3", palette::dim2()));
    left.push(super::common::status_sep());
    if count == 0 {
        left.push(Span::styled("no words yet", palette::dim2()));
    } else {
        let noun = if count == 1 { "card" } else { "cards" };
        left.push(Span::styled(
            count.to_string(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(format!(" {noun}"), palette::dim()));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    if count > 0 {
        right.extend(super::common::key_hint("Shift+Enter", "continue"));
    } else {
        right.extend(super::common::key_hint("Cmd+V", "paste a list"));
    }
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}

fn word_count(blob: &str) -> usize {
    blob.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count()
}
