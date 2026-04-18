//! Renderer for the `Your words` screen.
//!
//! Anchors on `docs/tui-states/current-pdf/01-your-words.png`. A compact
//! language pair badge is added on top of the PDF as the missing language
//! layer (see CTX-183 / CTX-184).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::tui::app::App;

const PLACEHOLDER: &str = "paste one per line, or comma-separated, or a messy blob:";
const HEADLINE: &str = "Your words";
const TAGLINE: &str = "paste anything — I figure out the rest";
const HINT_LEFT: &str = "минимум трения. Только слова.";
const HINT_RIGHT: &str = "[⌘V] paste · [Enter] continue";

/// Draw the `Your words` screen into the given area for the current `App`.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(language_badge(app), areas[0]);
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title_top(Line::from(Span::styled(
            format!(" {HEADLINE} "),
            Style::default().add_modifier(Modifier::BOLD),
        )))
        .title_top(
            Line::from(Span::styled(
                format!(" {TAGLINE} "),
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Right),
        );
    let inner = outer.inner(areas[1]);
    frame.render_widget(outer, areas[1]);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(input_panel(app), sections[0]);
    frame.render_widget(divider(sections[1].width), sections[1]);
    frame.render_widget(footer_line(sections[2].width), sections[2]);
}

fn language_badge(app: &App) -> Paragraph<'_> {
    let target = if app.target_pending() {
        String::from("detecting…")
    } else {
        app.pair().target().to_uppercase()
    };
    let support = app.pair().support().to_uppercase();
    let text = format!("kamishibai · {target} → {support}");
    Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )))
}

fn input_panel(app: &App) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {PLACEHOLDER}"),
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    let typed = app.blob();
    if typed.is_empty() {
        lines.push(Line::from(Span::styled(
            "   █",
            Style::default().fg(Color::White),
        )));
    } else {
        for (row, raw) in typed.split('\n').enumerate() {
            let mut spans = vec![Span::raw("   ")];
            spans.push(Span::styled(
                String::from(raw),
                Style::default().fg(Color::White),
            ));
            if row + 1 == typed.split('\n').count() {
                spans.push(Span::styled("█", Style::default().fg(Color::White)));
            }
            lines.push(Line::from(spans));
        }
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(Color::Black).fg(Color::White))
}

fn divider(width: u16) -> Paragraph<'static> {
    let dashes = "-".repeat(width as usize);
    Paragraph::new(Line::from(Span::styled(
        dashes,
        Style::default().fg(Color::DarkGray),
    )))
}

fn footer_line(width: u16) -> Paragraph<'static> {
    let left = HINT_LEFT;
    let right = HINT_RIGHT;
    let total = left.chars().count() + right.chars().count();
    let gap = (width as usize).saturating_sub(total);
    let padding = " ".repeat(gap);
    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::Gray)),
        Span::raw(padding),
        Span::styled(
            right,
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(line)
}
