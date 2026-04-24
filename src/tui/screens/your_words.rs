//! Renderer for the `Your words` screen.
//!
//! Mirrors state 01 of the design mockup
//! (`kamishibai/project/states.js` · "Your words").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

const PLACEHOLDER: &str = "paste one per line, or comma-separated, or a messy blob:";
const HEADLINE: &str = "Your words";
const TAGLINE: &str = "paste anything — I figure out the rest";
const HINT_KEYS: &str = "[paste/type] words · [L] my language · [Enter] continue";

/// Draw the `Your words` screen into the given area for the current `App`.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let frame_rects = super::common::frame(area);
    frame.render_widget(
        super::common::language_badge(app).style(palette::base()),
        frame_rects.badge,
    );
    frame.render_widget(
        super::common::header(HEADLINE, TAGLINE, frame_rects.header.width),
        frame_rects.header,
    );
    frame.render_widget(input_panel(app), frame_rects.body);
    frame.render_widget(
        super::common::dashed_divider(frame_rects.footer_rule.width),
        frame_rects.footer_rule,
    );
    frame.render_widget(
        super::common::footer(HINT_KEYS, frame_rects.footer.width),
        frame_rects.footer,
    );
}

fn input_panel(app: &App) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(PLACEHOLDER, palette::dim())));
    lines.push(Line::from(""));
    let typed = app.blob();
    if typed.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("▍", Style::default().bg(palette::BG).fg(palette::FG)),
            Span::styled(
                " ",
                Style::default()
                    .bg(palette::FG)
                    .fg(palette::FG)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]));
    } else {
        let total = typed.split('\n').count();
        for (row, raw) in typed.split('\n').enumerate() {
            let mut spans: Vec<Span<'_>> = Vec::new();
            if row == 0 {
                spans.push(Span::styled(
                    "▍",
                    Style::default().bg(palette::BG).fg(palette::FG),
                ));
            } else {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                String::from(raw),
                Style::default().bg(palette::BG).fg(palette::FG),
            ));
            if row + 1 == total {
                spans.push(Span::styled(
                    " ",
                    Style::default()
                        .bg(palette::FG)
                        .fg(palette::FG)
                        .add_modifier(Modifier::SLOW_BLINK),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    Paragraph::new(lines).style(palette::base())
}
