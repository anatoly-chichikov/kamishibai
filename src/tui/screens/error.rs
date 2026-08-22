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

const TITLE: &str = " can't reach gemini ";
const HINT: &str = "press any key to dismiss";
const HORIZONTAL_MARGIN: u16 = 8;
const HORIZONTAL_PADDING: u16 = 2;
const VERTICAL_MARGIN: u16 = 4;
/// Rows the panel spends on chrome: two borders, the lead blank, the blank
/// under the message, and the hint.
const CHROME_ROWS: u16 = 5;
/// Columns the panel spends on chrome: two borders plus the padding either side.
const CHROME_COLUMNS: u16 = 2 + HORIZONTAL_PADDING * 2;

/// Draw one recoverable request error over the current screen.
pub fn draw(frame: &mut Frame, area: Rect, _app: &App, message: &str) {
    let inset = panel_rect(area, message);
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
    let title = Span::styled(TITLE, palette::base());
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
        HINT,
        palette::Ink::Detail.on(false),
    )))
    .style(palette::base())
}

/// Size the panel to the message it carries.
///
/// A one-line failure used to open a near-fullscreen box because the panel was
/// derived from the terminal rather than the text. It now takes the room the
/// message needs and no more, still bounded by the terminal and never narrower
/// than its own title.
fn panel_rect(area: Rect, message: &str) -> Rect {
    let ceiling = area.width.saturating_sub(HORIZONTAL_MARGIN).max(1);
    let chrome = super::common::display_width(TITLE).max(super::common::display_width(HINT));
    let floor = u16::try_from(chrome)
        .unwrap_or(u16::MAX)
        .saturating_add(CHROME_COLUMNS);
    let wanted = u16::try_from(super::common::display_width(message))
        .unwrap_or(u16::MAX)
        .saturating_add(CHROME_COLUMNS);
    let width = wanted.clamp(floor.min(ceiling), ceiling);
    let inner = usize::from(width.saturating_sub(CHROME_COLUMNS)).max(1);
    let rows =
        u16::try_from(super::common::wrap_words(message, inner, inner).len()).unwrap_or(u16::MAX);
    let tallest = area.height.saturating_sub(VERTICAL_MARGIN).max(1);
    let height = rows.saturating_add(CHROME_ROWS).min(tallest);
    super::common::overlay_rect(area, width, height)
}

fn padded(area: Rect) -> Rect {
    Rect {
        x: area.x + HORIZONTAL_PADDING,
        y: area.y,
        width: area.width.saturating_sub(HORIZONTAL_PADDING * 2),
        height: area.height,
    }
}
