//! Renderer for the post-generation `your cards · done` screen.
//!
//! The new design folds the old standalone Done screen into the same canvas
//! as `your_cards` once every card has either finished or given up. Renders
//! the sticky outputs banner with the apkg/pdf/folder links and reuses the
//! card-block rendering from `your_cards` for visual continuity.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "your cards";
const HINT_OK: &str = "all done";
const HINT_FAIL: &str = "some cards didn't make it";

/// `ScreenView` handle for the post-generation summary screen.
pub struct Done;

impl ScreenView for Done {
    fn title(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HEADLINE)
    }

    fn hint(&self, app: &App) -> Cow<'static, str> {
        let copy = if app.cards_failed() > 0 {
            HINT_FAIL
        } else {
            HINT_OK
        };
        Cow::Borrowed(copy)
    }

    fn footer(&self, app: &App, width: u16) -> Paragraph<'static> {
        footer(app, width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        frame.render_widget(body(app).scroll((app.body_scroll(), 0)), area);
    }
}

fn body(app: &App) -> Paragraph<'_> {
    let done = app.done_artifacts();
    let mut lines: Vec<Line<'_>> = Vec::new();
    let entries: Vec<(&str, &str)> = [("APKG", done.deck.as_str()), ("PDF", done.report.as_str())]
        .into_iter()
        .filter(|(_, path)| !path.is_empty())
        .collect();
    if !entries.is_empty() {
        let mut top: Vec<Span<'_>> = vec![Span::styled("│ ", palette::base())];
        for (idx, (label, _)) in entries.iter().enumerate() {
            if idx > 0 {
                top.push(Span::styled("    ", palette::base()));
            }
            top.push(Span::styled("↓ ", palette::dim()));
            top.push(Span::styled(String::from(*label), palette::link()));
        }
        lines.push(Line::from(top));
    }
    lines.push(Line::from(""));
    if app.cards().is_empty() {
        lines.push(Line::from(Span::styled(
            "no cards in this batch",
            palette::dim(),
        )));
    } else {
        for (index, draft) in app.cards().iter().enumerate() {
            let glyph = if draft.artifacts().has_failed() {
                "✗"
            } else {
                "✓"
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {glyph} "), palette::base()),
                Span::styled(format!("{:0>2}  ", index + 1), palette::dim2()),
                Span::styled(String::from(draft.term()), palette::base()),
            ]));
        }
    }
    Paragraph::new(lines).style(palette::base())
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 3/3", palette::dim2()));
    left.push(super::common::status_sep());
    left.push(Span::styled(
        format!("{}/{} ready", app.cards_ready(), app.cards().len()),
        palette::dim(),
    ));
    if app.cards_failed() > 0 {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            format!("{} gave up", app.cards_failed()),
            palette::dim(),
        ));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    right.extend(super::common::key_hint("n", "new batch"));
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}
