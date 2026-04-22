//! Shared renderer for the two correction modals (states 02b and 03b).
//!
//! Both share the same visual pattern — double-line header `╔═ … ═╗`,
//! a card or list preview, a dashed textarea region, and a centered
//! `[Esc] cancel    [Enter] send` footer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::screen::ModalKind;

const MODAL_WIDTH: u16 = 72;
const MODAL_HEIGHT: u16 = 20;

/// Draw the correction modal of the requested kind.
pub fn draw(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    let inset = centered(area, MODAL_WIDTH, MODAL_HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    frame.render_widget(modal_body(kind, app, inset.width), inset);
}

fn modal_body<'a>(kind: ModalKind, app: &'a App, width: u16) -> Paragraph<'a> {
    let inner = width as usize;
    let title = match kind {
        ModalKind::ChangeSomething => "How should I change these?",
        ModalKind::ChangeThisCard => "How should I change this card?",
    };
    let top = double_edge_line(title, width);
    let blank_inner = format!("║{}║", " ".repeat(inner.saturating_sub(2)));
    let bottom = format!("╚{}╝", "═".repeat(inner.saturating_sub(2)));
    let mut lines: Vec<Line<'a>> = Vec::new();
    lines.push(Line::from(Span::styled(top, palette::base())));
    lines.push(side_line(&blank_inner));
    match kind {
        ModalKind::ChangeSomething => {
            lines.push(side_text_line(
                "     tell me in your own words — applies to all ",
                &format!("{}:", app.candidates().len()),
                width,
                palette::dim(),
                palette::dim(),
            ));
        }
        ModalKind::ChangeThisCard => {
            lines.extend(card_preview(app, width));
            lines.push(side_text_line(
                "     tell me what to change:",
                "",
                width,
                palette::dim(),
                palette::base(),
            ));
        }
    }
    lines.push(side_line(&blank_inner));
    lines.push(side_styled_line(
        Line::from(vec![Span::styled(
            format!("     {}", "─".repeat(inner.saturating_sub(12))),
            palette::dim(),
        )]),
        width,
    ));
    lines.extend(textarea_lines(app, width));
    lines.push(side_styled_line(
        Line::from(vec![Span::styled(
            format!("     {}", "─".repeat(inner.saturating_sub(12))),
            palette::dim(),
        )]),
        width,
    ));
    lines.push(side_line(&blank_inner));
    lines.push(footer_line(width));
    lines.push(side_line(&blank_inner));
    lines.push(Line::from(Span::styled(bottom, palette::base())));
    Paragraph::new(lines).style(palette::base())
}

fn double_edge_line(title: &str, width: u16) -> String {
    let inner = width as usize - 2;
    let adorned = format!("═ {title} ");
    let adorned_len = adorned.chars().count();
    let fill = inner.saturating_sub(adorned_len);
    format!("╔{adorned}{}╗", "═".repeat(fill))
}

fn side_line(content: &str) -> Line<'static> {
    Line::from(Span::styled(String::from(content), palette::base()))
}

fn side_styled_line(line: Line<'_>, width: u16) -> Line<'_> {
    let used: usize = line
        .spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum();
    let pad = (width as usize).saturating_sub(2).saturating_sub(used);
    let mut spans: Vec<Span<'_>> = Vec::new();
    spans.push(Span::styled("║", palette::base()));
    spans.extend(line.spans);
    spans.push(Span::styled(" ".repeat(pad), palette::base()));
    spans.push(Span::styled("║", palette::base()));
    Line::from(spans)
}

fn side_text_line<'a>(
    prefix: &str,
    suffix: &str,
    width: u16,
    prefix_style: Style,
    suffix_style: Style,
) -> Line<'a> {
    let visible = prefix.chars().count() + suffix.chars().count();
    let pad = (width as usize).saturating_sub(2).saturating_sub(visible);
    Line::from(vec![
        Span::styled("║", palette::base()),
        Span::styled(String::from(prefix), prefix_style),
        Span::styled(String::from(suffix), suffix_style),
        Span::styled(" ".repeat(pad), palette::base()),
        Span::styled("║", palette::base()),
    ])
}

fn card_preview<'a>(app: &'a App, width: u16) -> Vec<Line<'a>> {
    let inner = (width as usize).saturating_sub(2);
    let separator = format!("     {}", "─".repeat(inner.saturating_sub(10)));
    let focused = app.cards().get(app.card_selected());
    let title = focused
        .map(|draft| format!("     card #{} · {}", app.card_selected() + 1, draft.term()))
        .unwrap_or_else(|| String::from("     card preview"));
    let example = focused
        .map(|draft| format!("      {}", draft.payload().front()))
        .unwrap_or_else(|| String::from("      (nothing selected)"));
    vec![
        side_text_line(&title, "", width, palette::dim(), palette::base()),
        side_styled_line(
            Line::from(vec![Span::styled(separator.clone(), palette::dim())]),
            width,
        ),
        side_text_line(&example, "", width, palette::base(), palette::base()),
        side_styled_line(
            Line::from(vec![Span::styled(separator, palette::dim())]),
            width,
        ),
        side_text_line("", "", width, palette::base(), palette::base()),
    ]
}

fn textarea_lines<'a>(app: &'a App, width: u16) -> Vec<Line<'a>> {
    let inner = (width as usize).saturating_sub(2);
    let buffer = app.modal_buffer();
    let mut rows: Vec<String> = Vec::new();
    if buffer.is_empty() {
        rows.push(String::from("      "));
    } else {
        for line in buffer.split('\n') {
            rows.push(format!("      {line}"));
        }
    }
    let mut lines: Vec<Line<'a>> = Vec::new();
    let last = rows.len() - 1;
    for (index, text) in rows.into_iter().enumerate() {
        let mut visible = text.chars().count();
        let show_cursor = index == last;
        let cursor_width = if show_cursor { 1 } else { 0 };
        let pad = inner.saturating_sub(visible).saturating_sub(cursor_width);
        visible += cursor_width + pad;
        let _ = visible;
        let mut spans: Vec<Span<'a>> = Vec::new();
        spans.push(Span::styled("║", palette::base()));
        spans.push(Span::styled(text, palette::base()));
        if show_cursor {
            spans.push(Span::styled(
                " ",
                Style::default()
                    .bg(palette::FG)
                    .fg(palette::FG)
                    .add_modifier(Modifier::SLOW_BLINK),
            ));
        }
        spans.push(Span::styled(" ".repeat(pad), palette::base()));
        spans.push(Span::styled("║", palette::base()));
        lines.push(Line::from(spans));
    }
    lines
}

fn footer_line(width: u16) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    let text_left_key = "[Esc]";
    let text_left_label = " cancel";
    let gap = "    ";
    let text_right_key = "[Enter]";
    let text_right_label = " send";
    let visible = text_left_key.chars().count()
        + text_left_label.chars().count()
        + gap.chars().count()
        + text_right_key.chars().count()
        + text_right_label.chars().count();
    let pad_left = inner.saturating_sub(visible) / 2;
    let pad_right = inner.saturating_sub(visible).saturating_sub(pad_left);
    Line::from(vec![
        Span::styled("║", palette::base()),
        Span::styled(" ".repeat(pad_left), palette::base()),
        Span::styled(String::from(text_left_key), palette::key()),
        Span::styled(String::from(text_left_label), palette::base()),
        Span::raw(gap),
        Span::styled(String::from(text_right_key), palette::key()),
        Span::styled(String::from(text_right_label), palette::base()),
        Span::styled(" ".repeat(pad_right), palette::base()),
        Span::styled("║", palette::base()),
    ])
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let clamped_width = width.min(area.width);
    let clamped_height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(clamped_width) / 2;
    let y = area.y + area.height.saturating_sub(clamped_height) / 2;
    Rect {
        x,
        y,
        width: clamped_width,
        height: clamped_height,
    }
}
