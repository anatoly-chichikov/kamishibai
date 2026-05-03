//! Centered correction modal.
//!
//! Single visual pattern — solid border, dashed input rule, and a right-aligned
//! `[Esc] cancel  [↵] send` action row. Shared between the bulk-correction and
//! per-card-correction flows. The terminal cursor is placed on the input line
//! so the host terminal handles its own blink natively.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::screen::ModalKind;

const MODAL_WIDTH: u16 = 64;
const MODAL_HEIGHT: u16 = 9;
const HORIZONTAL_PADDING: u16 = 2;
const INPUT_LINE_OFFSET: u16 = 3;

/// Draw the correction modal of the requested kind.
pub fn draw(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    let inset = centered(area, MODAL_WIDTH, MODAL_HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base());
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let content = padded(inner);
    frame.render_widget(panel(kind, app, content.width as usize), content);
    let title_label = match kind {
        ModalKind::ChangeSomething => "change",
        ModalKind::ChangeThisCard => "change · this card",
    };
    let title = Span::styled(format!(" {title_label} "), palette::base());
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
    let buffer_width = app.modal_buffer().chars().count() as u16;
    let cursor_x = (content.x + buffer_width).min(content.x + content.width.saturating_sub(1));
    let cursor_y = content.y + INPUT_LINE_OFFSET;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn padded(inner: Rect) -> Rect {
    Rect {
        x: inner.x + HORIZONTAL_PADDING,
        y: inner.y,
        width: inner.width.saturating_sub(HORIZONTAL_PADDING * 2),
        height: inner.height,
    }
}

fn panel<'a>(kind: ModalKind, app: &'a App, width: usize) -> Paragraph<'a> {
    let prompt = match kind {
        ModalKind::ChangeSomething => "tell me what to change — applies to all".to_string(),
        ModalKind::ChangeThisCard => format!(
            "tell me what to change · {}",
            app.cards()
                .get(app.card_selected())
                .map(|draft| draft.term())
                .unwrap_or("")
        ),
    };
    let input = if app.modal_buffer().is_empty() {
        Line::from("")
    } else {
        Line::from(Span::styled(
            String::from(app.modal_buffer()),
            palette::base(),
        ))
    };
    let dashes = "─".repeat(width);
    let actions = Line::from(vec![
        Span::styled("[Esc]", palette::base().add_modifier(Modifier::BOLD)),
        Span::styled(" cancel    ", palette::dim()),
        Span::styled("[Enter]", palette::base().add_modifier(Modifier::BOLD)),
        Span::styled(" send", palette::base()),
    ]);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(prompt, palette::dim())),
        Line::from(""),
        input,
        Line::from(Span::styled(dashes, palette::rule())),
        Line::from(""),
        actions,
    ];
    Paragraph::new(lines).style(palette::base())
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
