//! Centered modals.
//!
//! Two visual patterns share the same centred surround (solid border, padded
//! content, action row):
//!
//! 1. The text modals (`ChangeSomething`, `ChangeThisCard`) — single
//!    text-input field with an `[Esc] cancel · [Enter] send` row, used for the
//!    missing-sense and per-card Gemini flows.
//! 2. The language picker (`PickMyLanguage`) — horizontal row of language
//!    chips with the currently active one inverted, an `[← →] pick · [Enter]
//!    confirm · [Esc] cancel` row. No text input, no cursor.
//!
//! All modals are rendered last in the frame so they sit on top of the
//! fullscreen screen beneath them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::languages::catalog;
use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::screen::ModalKind;

const TEXT_MODAL_WIDTH: u16 = 64;
const TEXT_MODAL_HEIGHT: u16 = 9;
const SIMPLE_TEXT_MODAL_HEIGHT: u16 = 7;
const PICKER_MODAL_WIDTH: u16 = 66;
const PICKER_MODAL_HEIGHT: u16 = 7;
const HORIZONTAL_PADDING: u16 = 2;
const INPUT_LINE_OFFSET: u16 = 3;
const SIMPLE_INPUT_LINE_OFFSET: u16 = 1;

/// Draw the modal of the requested kind.
pub fn draw(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    match kind {
        ModalKind::PickMyLanguage => draw_picker(frame, area, app),
        ModalKind::ChangeSomething | ModalKind::ChangeThisCard => {
            draw_text_modal(frame, area, kind, app)
        }
    }
}

fn draw_text_modal(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    let inset = centered(area, TEXT_MODAL_WIDTH, text_modal_height(kind));
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = surround();
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let content = padded(inner);
    frame.render_widget(text_panel(kind, app, content.width as usize), content);
    paint_title(frame, inset, text_title(kind));
    let buffer_width = app.modal_buffer().chars().count() as u16;
    let cursor_x = (content.x + buffer_width).min(content.x + content.width.saturating_sub(1));
    let cursor_y = content.y + input_line_offset(kind);
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn draw_picker(frame: &mut Frame, area: Rect, app: &App) {
    let inset = centered(area, PICKER_MODAL_WIDTH, PICKER_MODAL_HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = surround();
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let content = padded(inner);
    frame.render_widget(picker_panel(app, content.width as usize), content);
    paint_title(frame, inset, "your language");
}

fn surround() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base())
}

fn paint_title(frame: &mut Frame, inset: Rect, label: &str) {
    let title = Span::styled(format!(" {label} "), palette::base());
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

fn text_title(kind: ModalKind) -> &'static str {
    match kind {
        ModalKind::ChangeSomething => "what meanings did we miss?",
        ModalKind::ChangeThisCard => "make this sentence different",
        ModalKind::PickMyLanguage => "your language",
    }
}

fn text_modal_height(kind: ModalKind) -> u16 {
    match kind {
        ModalKind::ChangeSomething => TEXT_MODAL_HEIGHT,
        ModalKind::ChangeThisCard => SIMPLE_TEXT_MODAL_HEIGHT,
        ModalKind::PickMyLanguage => PICKER_MODAL_HEIGHT,
    }
}

fn input_line_offset(kind: ModalKind) -> u16 {
    match kind {
        ModalKind::ChangeSomething => INPUT_LINE_OFFSET,
        ModalKind::ChangeThisCard => SIMPLE_INPUT_LINE_OFFSET,
        ModalKind::PickMyLanguage => SIMPLE_INPUT_LINE_OFFSET,
    }
}

fn text_panel<'a>(kind: ModalKind, app: &'a App, width: usize) -> Paragraph<'a> {
    let prompt = match kind {
        ModalKind::ChangeSomething => format!(
            "domain, slang, idiom, region, or rare use · {}",
            app.candidates()
                .get(app.selected())
                .map(|candidate| candidate.term())
                .unwrap_or("")
        ),
        ModalKind::ChangeThisCard => String::new(),
        ModalKind::PickMyLanguage => String::new(),
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
    let mut action_spans = super::common::FooterHint::ghost("Esc", "cancel").spans();
    action_spans.push(Span::styled(String::from("    "), palette::base()));
    action_spans.extend(super::common::FooterHint::primary("Enter", "send").spans());
    let actions = Line::from(action_spans);
    let lines = match kind {
        ModalKind::ChangeSomething => vec![
            Line::from(""),
            Line::from(Span::styled(prompt, palette::dim())),
            Line::from(""),
            input,
            Line::from(Span::styled(dashes, palette::rule())),
            Line::from(""),
            actions,
        ],
        ModalKind::ChangeThisCard | ModalKind::PickMyLanguage => vec![
            Line::from(""),
            input,
            Line::from(Span::styled(dashes, palette::rule())),
            Line::from(""),
            actions,
        ],
    };
    Paragraph::new(lines).style(palette::base())
}

fn picker_panel(app: &App, _width: usize) -> Paragraph<'static> {
    let codes = catalog().codes();
    let cursor = app.picker_cursor().min(codes.len() - 1);
    let mut chip_spans: Vec<Span<'static>> = Vec::with_capacity(codes.len() * 2);
    for (index, code) in codes.iter().enumerate() {
        let label = format!(" {} ", code.to_uppercase());
        let style = if index == cursor {
            palette::invert().add_modifier(Modifier::BOLD)
        } else {
            palette::dim()
        };
        chip_spans.push(Span::styled(label, style));
        if index + 1 < codes.len() {
            chip_spans.push(Span::styled("  ", palette::base()));
        }
    }
    let mut action_spans = super::common::FooterHint::secondary("← →", "pick").spans();
    action_spans.push(Span::styled(String::from("  "), palette::base()));
    action_spans.extend(super::common::FooterHint::primary("Enter", "confirm").spans());
    action_spans.push(Span::styled(String::from("  "), palette::base()));
    action_spans.extend(super::common::FooterHint::ghost("Esc", "cancel").spans());
    let actions = Line::from(action_spans);
    let lines = vec![
        Line::from(""),
        Line::from(chip_spans),
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

/// Geometry helpers exported so the input layer can mouse-hit-test the chip
/// row inside the picker modal.
pub mod picker_geometry {
    use super::{HORIZONTAL_PADDING, PICKER_MODAL_HEIGHT, PICKER_MODAL_WIDTH, centered};
    use crate::languages::catalog;
    use ratatui::layout::Rect;

    /// Return the chip index that landed under `(x, y)` inside `area`, or
    /// `None` if the click missed every chip.
    pub fn chip_at(area: Rect, x: u16, y: u16) -> Option<usize> {
        let inset = centered(area, PICKER_MODAL_WIDTH, PICKER_MODAL_HEIGHT);
        let inner_x = inset.x + 1 + HORIZONTAL_PADDING;
        let inner_y = inset.y + 1; // top border
        let chip_row = inner_y + 1; // one blank line, then chip row
        if y != chip_row {
            return None;
        }
        let codes = catalog().codes();
        let mut cursor_x = inner_x;
        for (index, code) in codes.iter().enumerate() {
            let chip_w = code.chars().count() as u16 + 2; // " XX "
            let start = cursor_x;
            let end = start + chip_w;
            if x >= start && x < end {
                return Some(index);
            }
            cursor_x = end + if index + 1 < codes.len() { 2 } else { 0 };
        }
        None
    }
}
