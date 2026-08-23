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

/// `ScreenView` handle for the post-generation summary screen.
pub struct Done;

impl ScreenView for Done {
    fn title(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HEADLINE)
    }

    /// Silent when cards were lost — the outcome strip states that once, in
    /// the one place bright enough to be seen.
    fn hint(&self, app: &App) -> Cow<'static, str> {
        let copy = if super::banner::losses(app) > 0 {
            ""
        } else {
            HINT_OK
        };
        Cow::Borrowed(copy)
    }

    fn status(&self, app: &App) -> Vec<Span<'static>> {
        status(app)
    }

    fn hints(&self, app: &App) -> Vec<super::common::FooterHint> {
        hints(app)
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
        frame.render_widget(super::banner::widget(app, area.width), split[0]);
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
            palette::Ink::Detail.on(false),
        )));
    } else {
        for (index, draft) in app.cards().iter().enumerate() {
            // One question, asked once. This screen only exists for a batch
            // that has already been published, so a card either made the deck
            // or gave up — there is no third state to distinguish. Asking
            // `all_ready()` for the ink was asking about artifacts the reopen
            // path never loads: `DraftRecord` stores identity and spend, not
            // slots, so `CardDraft::new` hands back a default set and every
            // real row came out at the dimmest rank while its own `✓` said the
            // card had shipped. Now the glyph and the ink answer together.
            let failed = draft.artifacts().has_failed();
            let glyph = if failed { "✗" } else { "✓" };
            let glyph_style = if failed {
                palette::Ink::Subject.on(false)
            } else {
                palette::Ink::Aside.on(false)
            };
            let term_style = if failed {
                palette::Ink::Aside.on(false)
            } else {
                palette::Ink::Subject.on(false)
            };
            let mut spans = vec![
                Span::styled(format!(" {glyph} "), glyph_style),
                Span::styled(
                    format!("{:0>2}  ", index + 1),
                    palette::Ink::Aside.on(false),
                ),
                Span::styled(String::from(draft.term()), term_style),
            ];
            if let Some(cost) = super::your_cards::card_cost(draft) {
                spans.push(Span::styled(
                    format!("  {}", cost.dollars()),
                    palette::Ink::Aside.on(false),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    Paragraph::new(lines).style(palette::base())
}

/// Return whether this screen is reading a published record rather than the
/// live batch it was reopened from.
fn published(app: &App) -> bool {
    let done = app.done_artifacts();
    !done.deck.is_empty() && done.cards.saturating_add(done.failed) > 0
}

fn status(app: &App) -> Vec<Span<'static>> {
    let done = app.done_artifacts();
    let published = published(app);
    let ready = if published {
        done.cards
    } else {
        app.cards_ready()
    };
    let total = if published {
        done.cards.saturating_add(done.failed)
    } else {
        app.cards().len()
    };
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 3/3", palette::Ink::Aside.on(false)));
    left.push(super::common::status_sep());
    left.push(Span::styled(
        ready.to_string(),
        palette::Ink::Subject.on(false),
    ));
    left.push(Span::styled(
        format!("/{total} ready"),
        palette::Ink::Detail.on(false),
    ));
    if let Some(cost) = super::your_cards::total_cost(app) {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            cost.dollars_cents(),
            palette::Ink::Subject.on(false),
        ));
    }
    left
}

fn hints(app: &App) -> Vec<super::common::FooterHint> {
    if app.new_batch_pending() {
        return vec![
            super::common::new_batch_hint(true),
            super::common::quit_hint(app.quit_pending()),
        ];
    }
    let mut hints: Vec<super::common::FooterHint> = Vec::new();
    if super::banner::losses(app) > 0 {
        hints.push(super::common::FooterHint::primary("Ctrl+G", "regenerate"));
    }
    if !app.cards().is_empty() {
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
    }
    if app.can_start_new_batch() {
        hints.push(super::common::new_batch_hint(app.new_batch_pending()));
    }
    hints.push(super::common::quit_hint(app.quit_pending()));
    hints
}
