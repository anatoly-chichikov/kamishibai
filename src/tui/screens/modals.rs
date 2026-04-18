//! Shared renderer for the two correction modals.
//!
//! `Change something` anchors on `docs/tui-states/current-pdf/03-change-something-modal.png`.
//! `Change this card` anchors on `docs/tui-states/current-pdf/05-change-this-card-modal.png`.
//! Both share the same visual pattern — a centered rounded panel with a
//! prompt, a freeform textarea, and an `[Esc] cancel / [Enter] send` footer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::tui::app::App;
use crate::tui::screen::ModalKind;

pub fn draw(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    let inset = centered(area, 72, 16);
    frame.render_widget(Clear, inset);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title_top(Line::from(Span::styled(
            format!(" {} ", title(kind)),
            Style::default().add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(Color::Black).fg(Color::White));
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(prompt_line(kind, app), sections[0]);
    frame.render_widget(dashed(sections[1].width), sections[1]);
    frame.render_widget(textarea(app), sections[2]);
    frame.render_widget(dashed(sections[3].width), sections[3]);
    frame.render_widget(footer(sections[4].width), sections[4]);
}

fn title(kind: ModalKind) -> &'static str {
    match kind {
        ModalKind::ChangeSomething => "How should I change these?",
        ModalKind::ChangeThisCard => "How should I change this card?",
    }
}

fn prompt_line<'a>(kind: ModalKind, app: &'a App) -> Paragraph<'a> {
    let text = match kind {
        ModalKind::ChangeSomething => format!(
            "  tell me in your own words — applies to all {count}:",
            count = app.candidates().len()
        ),
        ModalKind::ChangeThisCard => {
            String::from("  tell me in your own words — applies to this card only:")
        }
    };
    Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::Gray),
    )))
    .style(Style::default().bg(Color::Black).fg(Color::White))
}

fn textarea(app: &App) -> Paragraph<'_> {
    let buffer = app.modal_buffer();
    let rendered = if buffer.is_empty() {
        String::from("  █")
    } else {
        let mut out = String::new();
        for line in buffer.split('\n') {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        let trimmed = out.trim_end_matches('\n').to_string();
        format!("{trimmed}█")
    };
    Paragraph::new(rendered)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(Color::Black).fg(Color::White))
}

fn dashed(width: u16) -> Paragraph<'static> {
    let line = "-".repeat(width as usize);
    Paragraph::new(Line::from(Span::styled(
        line,
        Style::default().fg(Color::DarkGray),
    )))
    .style(Style::default().bg(Color::Black))
}

fn footer(width: u16) -> Paragraph<'static> {
    let text = "[Esc] cancel    [Enter] send";
    let padding = (width as usize).saturating_sub(text.chars().count()) / 2;
    let line = Line::from(vec![
        Span::raw(" ".repeat(padding)),
        Span::styled(
            text,
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(line).style(Style::default().bg(Color::Black).fg(Color::White))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
