//! Renderer for the `Done` screen.
//!
//! Mirrors state 04 of the design mockup (`states.js` · "Done").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "Done";
const TAGLINE: &str = "here's what I made";
const HINT_KEYS: &str = "[n] new batch · [q] quit";

/// Draw the `Done` screen for the current `App`.
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
    frame.render_widget(artifacts_panel(app), frame_rects.body);
    frame.render_widget(
        super::common::dashed_divider(frame_rects.footer_rule.width),
        frame_rects.footer_rule,
    );
    frame.render_widget(
        super::common::footer(HINT_KEYS, frame_rects.footer.width),
        frame_rects.footer,
    );
}

fn artifacts_panel(app: &App) -> Paragraph<'_> {
    let done = app.done_artifacts();
    let deck = if done.deck.is_empty() {
        "—"
    } else {
        done.deck.as_str()
    };
    let report = if done.report.is_empty() {
        "—"
    } else {
        done.report.as_str()
    };
    let output = if done.output.is_empty() {
        "—"
    } else {
        done.output.as_str()
    };
    let row = |label: &str, value: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled(String::from("✓ "), palette::ok()),
            Span::styled(format!("{label:<11}"), palette::base()),
            Span::raw("  "),
            Span::styled(String::from(value), palette::link()),
        ])
    };
    let lines = vec![
        Line::from(""),
        Line::from(""),
        row("Anki deck:", deck),
        Line::from(""),
        row("Report:", report),
        Line::from(""),
        row("Output:", output),
    ];
    Paragraph::new(lines).style(palette::base())
}
