//! Shared chrome for the Kamishibai TUI screens.
//!
//! These primitives keep every fullscreen screen anchored on the same design
//! grid as the HTML mockup (`kamishibai-simple/project/styles.css`). All
//! tokens come from the static palette — no accent hue is allowed.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::tui::app::App;
use crate::tui::palette;

/// Horizontal breathing room applied to every screen's content column.
pub const GUTTER: u16 = 4;
/// Breathing room above the title row so it doesn't touch the top edge.
pub const TOP_MARGIN: u16 = 1;
/// Breathing room between the header and the body content.
pub const HEADER_GAP: u16 = 1;

/// Describes the rows of a fullscreen screen: top margin, header, rule lines,
/// body, dashed status separator, status bar.
pub struct ScreenFrame {
    pub header: Rect,
    pub body: Rect,
    pub status_rule: Rect,
    pub status: Rect,
}

/// Split the available area into the common screen frame.
///
/// Order top-down: top margin, header, breathing row, body, dashed rule above
/// status, status bar pinned to the last row.
pub fn frame_rects(area: Rect) -> ScreenFrame {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TOP_MARGIN),
            Constraint::Length(1),
            Constraint::Length(HEADER_GAP),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    ScreenFrame {
        header: inset_horizontal(rows[1], GUTTER),
        body: inset_horizontal(rows[3], GUTTER),
        status_rule: rows[4],
        status: inset_horizontal(rows[5], GUTTER),
    }
}

/// Render the chrome rule above the status bar. The header rule slot has
/// zero height now, so nothing is drawn at the top of the body.
pub fn paint_rules(frame: &mut Frame, rects: &ScreenFrame) {
    frame.render_widget(dashed_rule(rects.status_rule.width), rects.status_rule);
}

/// Render a dashed full-width rule line in `--rule` color.
pub fn dashed_rule(width: u16) -> Paragraph<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(width as usize);
    let dash_style = palette::rule().add_modifier(Modifier::CROSSED_OUT);
    for column in 0..width as usize {
        if column % 2 == 0 {
            spans.push(Span::styled(String::from(" "), dash_style));
        } else {
            spans.push(Span::styled(String::from(" "), palette::base()));
        }
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
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

/// Render the header row: inverted-block title, dim contextual tagline pinned
/// immediately to the right of the title, and the language chip pinned to the
/// right edge of the row.
///
/// The chip is provided as styled spans so the renderer can mark only the
/// learning language with the bright inverted block — the user's own
/// language stays dim, mirroring the "active label" treatment of the title.
///
/// Layout: `[ TITLE ]  hint                                 support → target`.
/// Two plain spaces separate the inverted title block from the dim hint —
/// enough breathing room after the bright background, no extra punctuation.
/// The flexible gap sits between the hint and the chip so the chip stays
/// anchored to the right edge.
pub fn header(
    title: &str,
    hint: &str,
    lang_chip: Option<Vec<Span<'static>>>,
    width: u16,
) -> Paragraph<'static> {
    let title = String::from(title);
    let hint = String::from(hint);
    let title_block = format!(" {title} ");
    let title_visible = title_block.chars().count();
    let hint_visible = hint.chars().count();
    let hint_lead = if hint.is_empty() { 0 } else { 2 };
    let chip = lang_chip.unwrap_or_default();
    let chip_visible: usize = chip.iter().map(|span| span.content.chars().count()).sum();
    let chip_lead = if chip.is_empty() { 0 } else { 2 };
    let used = title_visible + hint_lead + hint_visible + chip_visible + chip_lead;
    let gap = (width as usize).saturating_sub(used);
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        title_block,
        palette::invert().add_modifier(Modifier::BOLD),
    ));
    if !hint.is_empty() {
        spans.push(Span::styled("  ", palette::base()));
        spans.push(Span::styled(hint, palette::dim()));
    }
    spans.push(Span::styled(" ".repeat(gap), palette::base()));
    if !chip.is_empty() {
        spans.push(Span::styled("  ", palette::base()));
        spans.extend(chip);
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
}

/// Build the language chip — bold bright `support → target`.
///
/// Reading order is `support → target` so the user reads "from your language
/// into the language i'm learning". The whole chip — both languages and the
/// arrow between them — is rendered in bold bright `palette::base()`,
/// matching the title block on the opposite side of the header.
pub fn language_chip(app: &App) -> Vec<Span<'static>> {
    let support = app.pair().support().to_uppercase();
    let target_text = if app.target_pending() {
        String::from("…")
    } else {
        app.pair().target().to_uppercase()
    };
    let style = palette::base().add_modifier(Modifier::BOLD);
    vec![
        Span::styled(support, style),
        Span::styled(" → ", style),
        Span::styled(target_text, style),
    ]
}

/// Render the bottom status bar — left segment, right segment, separated by a flexible gap.
pub fn status_bar(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: u16,
) -> Paragraph<'static> {
    let left_visible: usize = left.iter().map(|span| span.content.chars().count()).sum();
    let right_visible: usize = right.iter().map(|span| span.content.chars().count()).sum();
    let gap = (width as usize).saturating_sub(left_visible + right_visible);
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.extend(left);
    spans.push(Span::styled(" ".repeat(gap), palette::base()));
    spans.extend(right);
    Paragraph::new(Line::from(spans)).style(palette::base())
}

/// Compose a `[KEY] label` pair suitable for a status bar key hint.
pub fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!("[{key}]"),
            palette::base().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}"), palette::dim()),
    ]
}

/// Compose the dim · separator between status bar segments.
pub fn status_sep() -> Span<'static> {
    Span::styled(" · ", palette::dim2())
}

/// Append the global Ctrl+C quit hint to a status-bar segment.
///
/// The hint is always present so the user never has to remember the chord;
/// the label switches from `quit` to a bold `again` once the first Ctrl+C
/// has been received and the next press will actually exit the process.
pub fn append_quit(segment: &mut Vec<Span<'static>>, pending: bool) {
    if !segment.is_empty() {
        segment.push(status_sep());
    }
    segment.push(Span::styled(
        "[Ctrl+C]",
        palette::base().add_modifier(Modifier::BOLD),
    ));
    if pending {
        segment.push(Span::styled(
            " again",
            palette::base().add_modifier(Modifier::BOLD),
        ));
    } else {
        segment.push(Span::styled(" quit", palette::dim()));
    }
}

/// One and only entry point for drawing a fullscreen screen.
///
/// Paints the background, computes the standard chrome layout, draws the
/// header and dashed status rule, hands the inner body rectangle to
/// `view.body`, and finishes by drawing the screen's footer pinned to the
/// bottom row. Screens cannot bypass this function — `render::draw` only
/// dispatches through it, and the dispatcher hands `view.body` the body Rect
/// only, so a screen has no handle on the chrome regions.
pub fn render_screen(frame: &mut Frame, area: Rect, app: &App, view: &dyn ScreenView) {
    paint_background(frame, area);
    let rects = frame_rects(area);
    let title = view.title(app);
    let hint = view.hint(app);
    frame.render_widget(
        header(
            title.as_ref(),
            hint.as_ref(),
            view.lang_chip(app),
            rects.header.width,
        ),
        rects.header,
    );
    paint_rules(frame, &rects);
    view.body(frame, rects.body, app);
    frame.render_widget(view.footer(app, rects.status.width), rects.status);
}

/// Clear a rectangle with the terminal-dark background so no stray paper bleeds through.
pub fn paint_background(frame: &mut Frame, area: Rect) {
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

/// Number of body-rect rows actually available for scrolling content on the
/// current screen, given the full terminal area. Equals the body rect height
/// minus the rows reserved for the sticky outputs banner (when it is shown).
/// Used by the wheel-scroll clamp so the user can never push content past the
/// bottom edge of the viewport.
pub fn scroll_viewport(app: &App, terminal_area: Rect) -> u16 {
    let body_height = frame_rects(terminal_area).body.height;
    let banner_rows = if banner_visible(app) {
        super::banner::height(app)
    } else {
        0
    };
    body_height.saturating_sub(banner_rows)
}

/// Body-rect width in chars for the current `terminal_area`. Used by callers
/// that need to feed the layout calc — `Your cards` wraps the meta sentence on
/// the head row, so scroll clamp and click hit-test must agree on the width.
pub fn scroll_body_width(terminal_area: Rect) -> u16 {
    frame_rects(terminal_area).body.width
}

fn banner_visible(app: &App) -> bool {
    if !super::banner::has_entries(app) {
        return false;
    }
    match app.screen() {
        crate::tui::screen::Screen::Done => true,
        crate::tui::screen::Screen::YourCards => {
            app.cards()
                .iter()
                .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
                && !app.cards().is_empty()
        }
        _ => false,
    }
}

/// Pad a string to the requested character width.
pub fn pad_right(value: &str, width: usize) -> String {
    let mut text = String::from(value);
    let gap = width.saturating_sub(value.chars().count());
    text.push_str(" ".repeat(gap).as_str());
    text
}
