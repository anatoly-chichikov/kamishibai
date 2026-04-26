//! Universal blocking loader overlay.
//!
//! It covers the current screen while a background text pass is running and
//! gives the render loop something visible to animate until the result arrives.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::BusyView;
use crate::tui::palette;

const WIDTH: u16 = 48;
const HEIGHT: u16 = 7;
const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];

/// Draw the universal blocking loader over the full terminal area.
pub fn draw(frame: &mut Frame, area: Rect, busy: &BusyView) {
    scrim(frame, area);
    let inset = centered(area, WIDTH, HEIGHT);
    frame.render_widget(Clear, inset);
    frame.render_widget(panel(busy, inset.width), inset);
}

fn scrim(frame: &mut Frame, area: Rect) {
    for row in 0..area.height {
        let mut pattern = String::with_capacity(area.width as usize);
        for column in 0..area.width {
            if (row + column) % 2 == 0 {
                pattern.push(' ');
            } else {
                pattern.push('░');
            }
        }
        let strip = Rect {
            x: area.x,
            y: area.y + row,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(pattern, palette::dim()))),
            strip,
        );
    }
}

fn panel(busy: &BusyView, width: u16) -> Paragraph<'static> {
    let inner = width as usize;
    let top = double_edge_line("Working", width);
    let bottom = format!("╚{}╝", "═".repeat(inner.saturating_sub(2)));
    let blank = format!("║{}║", " ".repeat(inner.saturating_sub(2)));
    let frame = spinner(busy);
    let message = format!("{frame} {}", busy.kind().label());
    Paragraph::new(vec![
        Line::from(Span::styled(top, palette::base())),
        Line::from(Span::styled(blank.clone(), palette::base())),
        centered_text(
            &message,
            width,
            palette::base().add_modifier(Modifier::BOLD),
        ),
        centered_text("the request is still running", width, palette::dim()),
        Line::from(Span::styled(blank, palette::base())),
        Line::from(Span::styled(bottom, palette::base())),
    ])
    .style(palette::base())
}

fn centered_text(text: &str, width: u16, style: Style) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    let clipped = text.chars().take(inner).collect::<String>();
    let text_width = clipped.chars().count();
    let left = inner.saturating_sub(text_width) / 2;
    let right = inner.saturating_sub(text_width).saturating_sub(left);
    Line::from(vec![
        Span::styled("║", palette::base()),
        Span::styled(" ".repeat(left), palette::base()),
        Span::styled(clipped, style),
        Span::styled(" ".repeat(right), palette::base()),
        Span::styled("║", palette::base()),
    ])
}

fn spinner(busy: &BusyView) -> &'static str {
    let index = (busy.elapsed().as_millis() / 160) as usize % FRAMES.len();
    FRAMES[index]
}

fn double_edge_line(title: &str, width: u16) -> String {
    let inner = (width as usize).saturating_sub(2);
    let adorned = format!("═ {title} ");
    let fill = inner.saturating_sub(adorned.chars().count());
    format!("╔{adorned}{}╗", "═".repeat(fill))
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
