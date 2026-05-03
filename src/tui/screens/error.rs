//! Recoverable request error overlay.
//!
//! Shown when a background text pass returns an error. Mirrors the modal
//! style — solid border, dim message, single key hint to dismiss.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::palette;

const WIDTH: u16 = 60;
const HEIGHT: u16 = 5;

/// Draw one recoverable request error over the current screen.
pub fn draw(frame: &mut Frame, area: Rect, _app: &App, message: &str) {
    let inset = centered(area, WIDTH, HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base());
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    frame.render_widget(panel(message), inner);
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

fn panel(message: &str) -> Paragraph<'_> {
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            String::from(message),
            palette::base().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("press any key to dismiss", palette::dim())),
    ])
    .style(palette::base())
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
