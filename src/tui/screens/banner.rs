//! Sticky outcome strip shared by `your cards` and `done`.
//!
//! One block, two halves: on the left the produced artifacts — APKG (deck),
//! PDF (report) and FOLDER (output directory), one per row, each an underlined
//! label followed by its path in dim grey; on the right, level with the first
//! row, a bright tag counting the cards that never made the deck. What you got
//! and what you lost, read in one glance.
//!
//! A dashed rule closes the block off from the card list below, the same rule
//! that closes the body off from the footer.
//!
//! The hit tester in `tui::links` mirrors the column maths laid out here.

use std::path::Path;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::runtime::locations::compact_path;
use crate::tui::app::App;
use crate::tui::palette;

/// Labels shown in the panel, in render order. The folder row leads so the
/// file rows below it inherit its directory context without repeating it.
pub const LABELS: [&str; 3] = ["FOLDER", "APKG", "PDF"];

/// Lead glyph repeated before every label.
pub const GLYPH: &str = "↓ ";

/// Visible characters of the leading `"│ "` indent.
pub const INDENT_WIDTH: usize = 2;

/// Visible characters between the longest label and the path column.
pub const LABEL_PAD: usize = 6;

/// Visible characters separating the padded label from the path.
pub const PATH_GAP: usize = 2;

/// Visible characters bracketing the loss tag, one space either side.
const TAG_PAD: usize = 2;

/// Number of rows the strip occupies inside the body rect: its content rows,
/// the dashed rule closing them off, and one blank before the cards. Returns
/// zero when there is nothing to report.
///
/// The rule hugs the last content row the way the status rule hugs the
/// disclaimer above it — no blank between a block and its own bottom edge.
pub fn height(app: &App) -> u16 {
    let count = content_rows(app);
    if count == 0 {
        0
    } else {
        u16::try_from(count + 2).unwrap_or(u16::MAX)
    }
}

/// Return whether the strip has anything to say — files, losses, or both.
///
/// Losses count on their own because a batch where every card gave up
/// publishes no deck at all, and that is exactly the run whose outcome the
/// learner most needs stated.
pub fn reports(app: &App) -> bool {
    content_rows(app) > 0
}

fn content_rows(app: &App) -> usize {
    let count = entries(app).len();
    if count > 0 {
        count
    } else {
        usize::from(losses(app) > 0)
    }
}

/// Return `true` if at least one of the three artifact paths is available.
pub fn has_entries(app: &App) -> bool {
    !entries(app).is_empty()
}

/// Return how many cards never made the deck.
///
/// A published record carries the durable tally and a live batch carries the
/// census. They answer the same question, but only the published one survives
/// a reopen, so it wins wherever it exists.
pub fn losses(app: &App) -> usize {
    let done = app.done_artifacts();
    if !done.deck.is_empty() && done.cards.saturating_add(done.failed) > 0 {
        return done.failed;
    }
    app.cards_failed()
}

/// Return the loss tag, or `None` when nothing was lost.
fn tag(app: &App) -> Option<String> {
    let lost = losses(app);
    if lost == 0 {
        return None;
    }
    Some(format!(" {lost} gave up "))
}

/// Return the (label, path) pairs that should be rendered, in order.
pub fn entries(app: &App) -> Vec<(&'static str, &str)> {
    let done = app.done_artifacts();
    LABELS
        .iter()
        .filter_map(|label| {
            let path = match *label {
                "FOLDER" => done.output.as_str(),
                "APKG" => done.deck.as_str(),
                "PDF" => done.report.as_str(),
                _ => "",
            };
            if path.is_empty() {
                None
            } else {
                Some((*label, path))
            }
        })
        .collect()
}

/// Return the basename of a path — the file name on its own.
pub fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from(path))
}

/// Return the visible file/path display for one banner row.
#[must_use]
pub fn display(label: &str, path: &str) -> String {
    if label == "FOLDER" {
        return compact_path(Path::new(path));
    }
    basename(path)
}

/// Render the strip. Caller is responsible for rendering it into a
/// `height(app)`-row sub-rect at the top of the body area, `width` cells wide.
/// The deck and report rows show only the file name; the folder row shows the
/// full directory path so the file rows above don't need to repeat it.
pub fn widget(app: &App, width: u16) -> Paragraph<'static> {
    let entries = entries(app);
    let width = usize::from(width);
    let tag = tag(app);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(entries.len() + 3);
    for (index, (label, path)) in entries.iter().enumerate() {
        let padding = LABEL_PAD.saturating_sub(super::common::display_width(label)) + PATH_GAP;
        let display = display(label, path);
        let mut spans: Vec<Span<'static>> = vec![
            Span::styled("│ ", palette::base()),
            Span::styled(GLYPH, palette::Ink::Detail.on(false)),
            Span::styled(String::from(*label), palette::Ink::Subject.link(false)),
            Span::styled(" ".repeat(padding), palette::base()),
            Span::styled(display, palette::Ink::Detail.on(false)),
        ];
        if index == 0
            && let Some(tag) = tag.as_deref()
        {
            push_tag(&mut spans, tag, width);
        }
        lines.push(Line::from(spans));
    }
    if entries.is_empty()
        && let Some(tag) = tag.as_deref()
    {
        // No gutter bar here: it exists to bind the file rows into one block,
        // and with no files beside it a lone tick would be marking nothing.
        let mut spans = Vec::new();
        push_tag(&mut spans, tag, width);
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        return Paragraph::new(Vec::<Line<'static>>::new()).style(palette::base());
    }
    lines.push(super::common::dashed_line(0, width));
    lines.push(Line::from(""));
    Paragraph::new(lines).style(palette::base())
}

/// Push the right-aligned loss tag onto one already-built row.
///
/// The tag keeps its full width and the gap absorbs the difference, so on a
/// terminal too narrow for both it is the path that runs off the edge — paths
/// already do, and the count is the part that cannot afford to.
fn push_tag(spans: &mut Vec<Span<'static>>, tag: &str, width: usize) {
    let used: usize = spans
        .iter()
        .map(|span| super::common::display_width(span.content.as_ref()))
        .sum();
    let gap = width
        .saturating_sub(used + super::common::display_width(tag))
        .max(TAG_PAD);
    spans.push(Span::styled(" ".repeat(gap), palette::base()));
    spans.push(Span::styled(
        String::from(tag),
        super::sentence_labels::tag_style(true),
    ));
}
