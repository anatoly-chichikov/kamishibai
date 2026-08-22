//! Sticky outputs panel shared by `your cards` and `done`.
//!
//! The panel lists the produced artifacts — APKG (deck), PDF (report) and
//! FOLDER (output directory) — one per row. Each row shows an underlined
//! placeholder label followed by the path in dim grey. The hit tester in
//! `tui::links` mirrors the column maths laid out here.

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

/// Number of rows the panel occupies inside the body rect (the artifact
/// rows plus one trailing blank line for breathing). Returns zero when no
/// artifact is ready yet.
pub fn height(app: &App) -> u16 {
    let count = entries(app).len();
    if count == 0 {
        0
    } else {
        u16::try_from(count + 1).unwrap_or(u16::MAX)
    }
}

/// Return `true` if at least one of the three artifact paths is available.
pub fn has_entries(app: &App) -> bool {
    !entries(app).is_empty()
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

/// Render the panel widget. Caller is responsible for rendering it into a
/// `height(app)`-row sub-rect at the top of the body area. The deck and
/// report rows show only the file name; the folder row shows the full
/// directory path so the file rows above don't need to repeat it.
pub fn widget(app: &App) -> Paragraph<'static> {
    let entries = entries(app);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(entries.len() + 1);
    for (label, path) in &entries {
        let padding = LABEL_PAD.saturating_sub(super::common::display_width(label)) + PATH_GAP;
        let display = display(label, path);
        let spans: Vec<Span<'static>> = vec![
            Span::styled("│ ", palette::base()),
            Span::styled(GLYPH, palette::Ink::Detail.on(false)),
            Span::styled(String::from(*label), palette::Ink::Subject.link(false)),
            Span::styled(" ".repeat(padding), palette::base()),
            Span::styled(display, palette::Ink::Detail.on(false)),
        ];
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    Paragraph::new(lines).style(palette::base())
}
