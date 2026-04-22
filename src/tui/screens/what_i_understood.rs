//! Renderer for the `What I understood` screen.
//!
//! Mirrors state 02 of the design mockup (`states.js` · "What I understood").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "What I understood";
const TAGLINE: &str = "a quick look before making the cards";
const HINT_KEYS: &str = "[↑↓] nav · [d] drop · [R] change something · [Enter] make cards";
const PENDING: &str = "understanding your words…";
const EMPTY_AFTER_DROP: &str = "nothing left to review — add more words or go back";

/// Draw the `What I understood` screen for the current `App`.
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
    frame.render_widget(body_panel(app, frame_rects.body.width), frame_rects.body);
    frame.render_widget(
        super::common::dashed_divider(frame_rects.footer_rule.width),
        frame_rects.footer_rule,
    );
    frame.render_widget(
        super::common::footer(HINT_KEYS, frame_rects.footer.width),
        frame_rects.footer,
    );
}

fn body_panel(app: &App, width: u16) -> Paragraph<'_> {
    if app.candidates().is_empty() {
        let message = if app.target_pending() {
            PENDING
        } else {
            EMPTY_AFTER_DROP
        };
        let mut lines: Vec<Line<'_>> = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(message, palette::dim())));
        lines.push(Line::from(""));
        lines.extend(confirm_prompts());
        return Paragraph::new(lines).style(palette::base());
    }
    let rule_width = (width as usize).min(80);
    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(rule_width),
        palette::dim(),
    )));
    lines.push(Line::from(""));
    for (index, candidate) in app.candidates().iter().enumerate() {
        let is_focus = index == app.selected();
        let term_style = if is_focus {
            Style::default()
                .bg(palette::BG)
                .fg(palette::FG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(palette::BG).fg(palette::FG)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:>2}.", index + 1), palette::base()),
            Span::raw("   "),
            Span::styled(format!("{:<12}", candidate.term()), term_style),
            Span::raw("   "),
            Span::styled(format!("{:<22}", candidate.kind().label()), palette::dim()),
            Span::raw("  "),
            Span::styled(String::from(candidate.preview()), palette::base()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(rule_width),
        palette::dim(),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.extend(confirm_prompts());
    Paragraph::new(lines).style(palette::base())
}

fn confirm_prompts() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("looks right?  ", palette::base()),
            Span::styled("[Enter]", palette::key()),
            Span::raw(" "),
            Span::styled("make cards", palette::dim()),
        ]),
        Line::from(vec![
            Span::styled("not quite?    ", palette::base()),
            Span::styled("[R]", palette::key()),
            Span::raw(" "),
            Span::styled("change something", palette::dim()),
        ]),
    ]
}
