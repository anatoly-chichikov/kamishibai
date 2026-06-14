//! Universal blocking loader overlay.
//!
//! Shown over the current screen while a background text pass is running.
//! Single solid-bordered card with the same circular half-step spinner the
//! terminal progress reporter uses — keeps a single visual language for
//! waiting state across the entire app.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::app::BusyView;
use crate::tui::palette;

const WIDTH: u16 = 50;
const HEIGHT: u16 = 5;
const HORIZONTAL_PADDING: u16 = 2;
const FRAME_MILLIS: u128 = 250;
const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// Draw the universal blocking loader over the full terminal area.
pub fn draw(frame: &mut Frame, area: Rect, busy: &BusyView) {
    let inset = centered(area, WIDTH, HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base());
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    frame.render_widget(panel(busy), padded(inner));
    let title = Span::styled(" working ", palette::base());
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

fn padded(inner: Rect) -> Rect {
    Rect {
        x: inner.x + HORIZONTAL_PADDING,
        y: inner.y,
        width: inner.width.saturating_sub(HORIZONTAL_PADDING * 2),
        height: inner.height,
    }
}

fn panel(busy: &BusyView) -> Paragraph<'static> {
    let label = busy.kind().label();
    let line = Line::from(vec![
        Span::styled(spinner(busy), palette::base()),
        Span::styled("  ", palette::base()),
        Span::styled(
            String::from(label),
            palette::base().add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(vec![Line::from(""), line, Line::from("")]).style(palette::base())
}

fn spinner(busy: &BusyView) -> &'static str {
    let index = (busy.elapsed().as_millis() / FRAME_MILLIS) as usize % FRAMES.len();
    FRAMES[index]
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
