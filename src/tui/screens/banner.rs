//! Sticky outputs banner shared by `your cards` and `done`.
//!
//! The banner shows up to three clickable labels — APKG (deck), PDF (report)
//! and FOLDER (output directory) — and is drawn into a fixed top sub-area of
//! the body so it does not move when the user scrolls the card list. The hit
//! tester in `tui::links` mirrors the column maths laid out here.

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

/// Number of rows the banner occupies inside the body rect (the link row plus
/// one breathing line).
pub const HEIGHT: u16 = 2;

/// Labels shown in the banner, in render order.
pub const LABELS: [&str; 3] = ["APKG", "PDF", "FOLDER"];

/// Lead glyph repeated before every label.
pub const GLYPH: &str = "↓ ";

/// Visible characters between two consecutive labels.
pub const SEPARATOR_WIDTH: usize = 4;

/// Visible characters of the leading `"│ "` indent.
pub const INDENT_WIDTH: usize = 2;

/// Return `true` if at least one of the three artifact paths is available.
pub fn has_entries(app: &App) -> bool {
    !entries(app).is_empty()
}

/// Return the (label, path) pairs that should be rendered, in order.
pub fn entries(app: &App) -> Vec<(&'static str, &str)> {
    let done = app.done_artifacts();
    let paths = [
        done.deck.as_str(),
        done.report.as_str(),
        done.output.as_str(),
    ];
    LABELS
        .iter()
        .zip(paths.iter())
        .filter_map(|(label, path)| {
            if path.is_empty() {
                None
            } else {
                Some((*label, *path))
            }
        })
        .collect()
}

/// Render the banner widget. Caller is responsible for rendering it into a
/// `HEIGHT`-row sub-rect at the top of the body area.
pub fn widget(app: &App) -> Paragraph<'static> {
    let entries = entries(app);
    let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", palette::base())];
    for (idx, (label, _)) in entries.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" ".repeat(SEPARATOR_WIDTH), palette::base()));
        }
        spans.push(Span::styled(GLYPH, palette::dim()));
        spans.push(Span::styled(String::from(*label), palette::link()));
    }
    Paragraph::new(vec![Line::from(spans), Line::from("")]).style(palette::base())
}
