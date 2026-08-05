//! Renderer for the post-generation `your cards · done` screen.
//!
//! The new design folds the old standalone Done screen into the same canvas
//! as `your_cards` once every card has either finished or given up. Renders
//! the sticky outputs banner with the apkg/pdf/folder links and reuses the
//! card-block rendering from `your_cards` for visual continuity.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
        let banner_rows = super::banner::height(app);
        if banner_rows == 0 {
            frame.render_widget(card_summary(app).scroll((app.body_scroll(), 0)), area);
            return;
        }
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(banner_rows), Constraint::Min(0)])
            .split(area);
        frame.render_widget(super::banner::widget(app), split[0]);
        frame.render_widget(card_summary(app).scroll((app.body_scroll(), 0)), split[1]);
    }
}

/// Total number of lines `card_summary` will produce — one per card, or one
/// placeholder row when the batch is empty. Used by the scroll clamp in
/// `tui::app` so the wheel cannot push content past the bottom edge.
pub(crate) fn content_height(app: &App) -> u16 {
    let lines = if app.cards().is_empty() {
        1
    } else {
        app.cards().len()
    };
    u16::try_from(lines).unwrap_or(u16::MAX)
}

fn card_summary(app: &App) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
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
            let mut spans = vec![
                Span::styled(format!(" {glyph} "), palette::base()),
                Span::styled(format!("{:0>2}  ", index + 1), palette::dim2()),
                Span::styled(String::from(draft.term()), palette::base()),
            ];
            if let Some(cost) = super::your_cards::card_cost(draft) {
                spans.push(Span::styled(
                    format!("  {}", cost.dollars()),
                    palette::dim2(),
                ));
            }
            lines.push(Line::from(spans));
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
    if let Some(cost) = super::your_cards::total_cost(app) {
        left.push(super::common::status_sep());
        left.push(Span::styled(cost.dollars_cents(), palette::dim()));
    }
    let mut hints: Vec<super::common::FooterHint> = Vec::new();
    if app.cards_failed() > 0 {
        hints.push(super::common::FooterHint::primary("Ctrl+G", "Regenerate"));
    }
    if app.can_start_new_batch() {
        hints.push(super::common::new_batch_hint(app.new_batch_pending()));
    }
    hints.push(super::common::quit_hint(app.quit_pending()));
    super::common::footer_bar(left, hints, width)
}
