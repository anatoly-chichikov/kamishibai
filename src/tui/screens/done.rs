//! Renderer for the `Done` screen.
//!
//! Anchors on `docs/tui-states/current-pdf/08-done.png`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::App;

const HEADLINE: &str = "Done";
const TAGLINE: &str = "here's what I made";
const HINT_LEFT: &str = "просто ссылки. Никакого summary.";
const HINT_RIGHT: &str = "[n] new batch · [q] quit";

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
    frame.render_widget(artifacts_panel(app), sections[0]);
    frame.render_widget(divider(sections[1].width), sections[1]);
    frame.render_widget(footer_line(sections[2].width), sections[2]);
}

fn language_badge(app: &App) -> Paragraph<'_> {
    let target = app.pair().target().to_uppercase();
    let support = app.pair().support().to_uppercase();
    Paragraph::new(Line::from(Span::styled(
        format!("kamishibai · {target} → {support}"),
        Style::default().fg(Color::DarkGray),
    )))
}

fn artifacts_panel(app: &App) -> Paragraph<'_> {
    let done = app.done_artifacts();
    let deck = if done.deck.is_empty() {
        "—"
    } else {
        done.deck.as_str()
    };
    let report = if done.report.is_empty() {
        "—"
    } else {
        done.report.as_str()
    };
    let output = if done.output.is_empty() {
        "—"
    } else {
        done.output.as_str()
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("    ✓ Anki deck:   {deck}"),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            format!("    ✓ Report:      {report}"),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            format!("    ✓ Output:      {output}"),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
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
