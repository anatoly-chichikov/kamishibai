//! Recoverable request error overlay.
//!
//! It keeps the TUI alive when a background text request fails.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::palette;

const WIDTH: u16 = 62;
const HEIGHT: u16 = 8;

/// Draw one recoverable request error over the current screen.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, message: &str) {
    let inset = centered(area, WIDTH, HEIGHT);
    frame.render_widget(Clear, inset);
    frame.render_widget(panel(app, message, inset.width), inset);
}

fn panel(app: &App, message: &str, width: u16) -> Paragraph<'static> {
    let copy = copy(app);
    let inner = width as usize;
    let top = double_edge_line(copy.title, width);
    let bottom = format!("╚{}╝", "═".repeat(inner.saturating_sub(2)));
    let blank = format!("║{}║", " ".repeat(inner.saturating_sub(2)));
    Paragraph::new(vec![
        Line::from(Span::styled(top, palette::base())),
        Line::from(Span::styled(blank.clone(), palette::base())),
        centered_text(
            copy.summary,
            width,
            palette::base().add_modifier(Modifier::BOLD),
        ),
        centered_text(message, width, palette::dim()),
        centered_text(copy.dismiss, width, palette::key()),
        Line::from(Span::styled(blank, palette::base())),
        Line::from(Span::styled(bottom, palette::base())),
    ])
    .style(palette::base())
}

fn centered_text(text: &str, width: u16, style: ratatui::style::Style) -> Line<'static> {
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

fn copy(app: &App) -> ErrorCopy {
    match app.pair().support() {
        "ru" => ErrorCopy {
            title: "Не получилось",
            summary: "запрос к Gemini завершился ошибкой",
            dismiss: "нажми любую клавишу, чтобы продолжить",
        },
        _ => ErrorCopy {
            title: "Request failed",
            summary: "Gemini returned an error",
            dismiss: "press any key to continue",
        },
    }
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

struct ErrorCopy {
    title: &'static str,
    summary: &'static str,
    dismiss: &'static str,
}
