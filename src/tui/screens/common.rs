//! Shared widgets for the Kamishibai TUI screens.
//!
//! These primitives (language badge, header, dividers, footer) keep every
//! fullscreen screen anchored on the same design grid. Colors come from the
//! grayscale ink palette; text content comes from the design mockup.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::palette;

/// Horizontal breathing room applied to every screen's content column.
pub const GUTTER: u16 = 4;
/// Breathing room above the language badge so it doesn't touch the top edge.
pub const TOP_MARGIN: u16 = 1;
/// Breathing room between the language badge and the title row.
pub const BADGE_GAP: u16 = 1;
/// Breathing room above the dashed footer separator so the body doesn't hug it.
pub const FOOTER_GAP: u16 = 1;

/// Describes the rows of a fullscreen screen: outer gutters, badge, header,
/// body, and footer.
pub struct ScreenFrame {
    pub badge: Rect,
    pub header: Rect,
    pub body: Rect,
    pub footer_rule: Rect,
    pub footer: Rect,
}

/// Split the available area into the common screen frame: top-margin,
/// badge, gap, header, body (with horizontal gutter), spacer, footer.
pub fn frame(area: Rect) -> ScreenFrame {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TOP_MARGIN),
            Constraint::Length(1),
            Constraint::Length(BADGE_GAP),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(FOOTER_GAP),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let body = inset_horizontal(rows[4], GUTTER);
    ScreenFrame {
        badge: inset_horizontal(rows[1], GUTTER),
        header: inset_horizontal(rows[3], GUTTER),
        body,
        footer_rule: rows[6],
        footer: inset_horizontal(rows[7], GUTTER),
    }
}

/// Return the rectangle shrunk by `gutter` columns on each side.
pub fn inset_horizontal(area: Rect, gutter: u16) -> Rect {
    let clamp = gutter.min(area.width / 2);
    Rect {
        x: area.x + clamp,
        y: area.y,
        width: area.width.saturating_sub(clamp * 2),
        height: area.height,
    }
}

/// Width-hint display length of the compact badge (the top-left "kamishibai · X → Y" line).
pub fn language_badge(app: &App) -> Paragraph<'_> {
    let target = if app.target_pending() {
        String::from("detecting…")
    } else {
        app.pair().target().to_uppercase()
    };
    let support = app.pair().support().to_uppercase();
    let text = format!("kamishibai · {target} → {support}");
    Paragraph::new(Line::from(Span::styled(text, palette::dim())))
}

/// Return the header line with a bold title on the left and a dim tagline on the right.
pub fn header(title: &str, tagline: &str, width: u16) -> Paragraph<'static> {
    let title = String::from(title);
    let tagline = String::from(tagline);
    let left_chars = title.chars().count();
    let right_chars = tagline.chars().count();
    let gap = (width as usize).saturating_sub(left_chars + right_chars);
    Paragraph::new(Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .bg(palette::BG)
                .fg(palette::FG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(gap), palette::base()),
        Span::styled(tagline, palette::dim()),
    ]))
    .style(palette::base())
}

/// Return a full-width dashed divider (dim) — the thin rule between body and footer.
pub fn dashed_divider(width: u16) -> Paragraph<'static> {
    let cells = width as usize;
    let mut pattern = String::with_capacity(cells);
    for column in 0..cells {
        pattern.push(if column % 2 == 0 { '─' } else { ' ' });
    }
    Paragraph::new(Line::from(Span::styled(pattern, palette::dim()))).style(palette::base())
}

/// Return a labelled section divider — `label ──────────────────...` — dim.
pub fn section_divider(label: &str, width: u16) -> Paragraph<'static> {
    let prefix = format!("{label} ");
    let prefix_len = prefix.chars().count();
    let remaining = (width as usize).saturating_sub(prefix_len);
    Paragraph::new(Line::from(vec![Span::styled(
        format!("{prefix}{}", "─".repeat(remaining)),
        palette::dim(),
    )]))
    .style(palette::base())
}

/// Return the bottom hints bar — right-aligned keyboard hints only.
///
/// The design mockup's left-hand "why" blurb is design commentary, not product
/// UI, so it is not rendered here.
pub fn footer(keys: &str, width: u16) -> Paragraph<'static> {
    let text = String::from(keys);
    let pad = (width as usize).saturating_sub(text.chars().count());
    Paragraph::new(Line::from(vec![
        Span::styled(" ".repeat(pad), palette::base()),
        Span::styled(text, palette::key()),
    ]))
    .style(palette::base())
}

/// Clear a rectangle with the terminal-dark background so no stray paper bleeds through.
pub fn paint_background(frame: &mut ratatui::Frame, area: Rect) {
    let filler = " ".repeat(area.width as usize);
    for row in 0..area.height {
        let strip = Rect {
            x: area.x,
            y: area.y + row,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(filler.clone(), palette::base()))),
            strip,
        );
    }
}
