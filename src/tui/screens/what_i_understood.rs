//! Renderer for the `What I understood` screen.
//!
//! Anchors on `docs/tui-states/current-pdf/02-what-i-understood.png`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::App;

const HEADLINE: &str = "What I understood";
const TAGLINE: &str = "a quick look before making the cards";
const HINT_LEFT: &str =
    "один экран, один вопрос: «я правильно понял?». Всё про одно слово — в одну строку.";
const HINT_RIGHT: &str = "[↑↓] nav · [d] drop · [R] change something · [Enter] make cards";
const CONFIRM_PROMPT_OK: &str = "looks right?   [Enter] make cards";
const CONFIRM_PROMPT_NO: &str = "not quite?     [R] change something";
const PENDING: &str = "understanding your words…";
const EMPTY_AFTER_DROP: &str = "nothing left to review — add more words or go back";

/// Draw the `What I understood` screen for the current `App`.
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
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(candidates_panel(app), sections[0]);
    frame.render_widget(prompt_panel(), sections[1]);
    frame.render_widget(divider(sections[2].width), sections[2]);
    frame.render_widget(footer_line(sections[3].width), sections[3]);
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

fn candidates_panel(app: &App) -> Paragraph<'_> {
    if app.candidates().is_empty() {
        let message = if app.target_pending() {
            PENDING
        } else {
            EMPTY_AFTER_DROP
        };
        return Paragraph::new(Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(Color::Gray),
        )))
        .style(Style::default().bg(Color::Black).fg(Color::White));
    }
    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(""));
    for (index, candidate) in app.candidates().iter().enumerate() {
        let marker = if index == app.selected() { "▸" } else { " " };
        let row = format!(
            "  {marker} {number:>2}.  {term:<12}  {kind:<18}  {preview}",
            marker = marker,
            number = index + 1,
            term = candidate.term(),
            kind = candidate.kind().label(),
            preview = candidate.preview(),
        );
        let style = if index == app.selected() {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(row, style)));
    }
    lines.push(Line::from(""));
    Paragraph::new(lines).style(Style::default().bg(Color::Black).fg(Color::White))
}

fn prompt_panel() -> Paragraph<'static> {
    let lines = vec![
        Line::from(Span::styled(
            format!("  {CONFIRM_PROMPT_OK}"),
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            format!("  {CONFIRM_PROMPT_NO}"),
            Style::default().fg(Color::Gray),
        )),
    ];
    Paragraph::new(lines).style(Style::default().bg(Color::Black).fg(Color::White))
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
