//! Recoverable request error overlay.
//!
//! Shown when a background text pass returns an error. Mirrors the modal
//! style — solid border, dim message, single key hint to dismiss.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::app::App;
use crate::tui::palette;

const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 8;
const HORIZONTAL_MARGIN: u16 = 8;
const HORIZONTAL_PADDING: u16 = 2;
const VERTICAL_MARGIN: u16 = 4;

/// Draw one recoverable request error over the current screen.
pub fn draw(frame: &mut Frame, area: Rect, _app: &App, message: &str) {
    let inset = panel_rect(area);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base());
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(message_panel(message), padded(chunks[1]));
    frame.render_widget(hint_panel(), padded(chunks[2]));
    let title = Span::styled(" can't reach gemini ", palette::base());
    let title_rect = Rect {
        x: inset.x + 2,
        y: inset.y,
        width: title.content.chars().count() as u16,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(title)).style(palette::base()),
        title_rect,
    );
}

fn message_panel(message: &str) -> Paragraph<'_> {
    Paragraph::new(Span::styled(
        String::from(message),
        palette::base().add_modifier(Modifier::BOLD),
    ))
    .wrap(Wrap { trim: false })
    .style(palette::base())
}

fn hint_panel() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "press any key to dismiss",
        palette::dim(),
    )))
    .style(palette::base())
}

fn panel_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(HORIZONTAL_MARGIN).max(MIN_WIDTH);
    let height = area.height.saturating_sub(VERTICAL_MARGIN).max(MIN_HEIGHT);
    centered(area, width, height)
}

fn padded(area: Rect) -> Rect {
    Rect {
        x: area.x + HORIZONTAL_PADDING,
        y: area.y,
        width: area.width.saturating_sub(HORIZONTAL_PADDING * 2),
        height: area.height,
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let actual_width = width.min(area.width);
    let actual_height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(actual_width) / 2,
        y: area.y + area.height.saturating_sub(actual_height) / 2,
        width: actual_width,
        height: actual_height,
    }
}
