//! Sticky outputs panel shared by `your cards` and `done`.
//!
//! The panel lists the produced artifacts — APKG (deck), PDF (report) and
//! FOLDER (output directory) — one per row. Each row shows the underlined
//! label followed by the path in dim grey, so the user can read where the
//! artifact landed without leaving the TUI. The hit tester in `tui::links`
//! mirrors the column maths laid out here.

use std::path::Path;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

/// Labels shown in the panel, in render order.
pub const LABELS: [&str; 3] = ["APKG", "PDF", "FOLDER"];

/// Lead glyph repeated before every label.
pub const GLYPH: &str = "↓ ";

/// Visible characters of the leading `"│ "` indent.
pub const INDENT_WIDTH: usize = 2;

/// Visible characters between the longest label and the path column.
pub const LABEL_PAD: usize = 6;

/// Visible characters separating the padded label from the path.
pub const PATH_GAP: usize = 2;

/// Number of rows of *artifact text* the panel occupies — one row per
/// artifact, no trailing blank. Callers leave their own breathing rows
/// around the panel.
pub fn height(app: &App) -> u16 {
    u16::try_from(entries(app).len()).unwrap_or(u16::MAX)
}

/// Number of rows the panel is lifted upward into the header-gap slot so it
/// sits closer to the title.
pub const LIFT: u16 = 1;

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

/// Return the basename of a path — the file name on its own.
pub fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from(path))
}

/// Render the panel widget. Caller is responsible for rendering it into a
/// `height(app)`-row sub-rect at the top of the body area. The deck and
/// report rows show only the file name; the folder row shows the full
/// directory path so the file rows above don't need to repeat it.
pub fn widget(app: &App) -> Paragraph<'static> {
    let entries = entries(app);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(entries.len());
    for (label, path) in &entries {
        let padding = LABEL_PAD.saturating_sub(label.chars().count()) + PATH_GAP;
        let display = if *label == "FOLDER" {
            String::from(*path)
        } else {
            basename(path)
        };
        let spans: Vec<Span<'static>> = vec![
            Span::styled("│ ", palette::base()),
            Span::styled(GLYPH, palette::dim()),
            Span::styled(String::from(*label), palette::link()),
            Span::styled(" ".repeat(padding), palette::base()),
            Span::styled(display, palette::dim()),
        ];
        lines.push(Line::from(spans));
    }
    Paragraph::new(lines).style(palette::base())
}
