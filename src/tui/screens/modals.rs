//! Centered modals.
//!
//! Two visual patterns share the same centred surround (solid border, padded
//! content, action row):
//!
//! 1. The text modal (`ChangeSomething`) — a single text-input field with an
//!    `[Esc] cancel · [Enter] send` row for the missing-sense flow.
//! 2. The language pair picker (`PickLanguages`) — two side-by-side vertical
//!    lists, one per half of the pair, each row naming a language by code and
//!    by the name its own speakers use. A ruled heading carries the arrow that
//!    says which way the pair reads. Above both lists sits one pinned row: the
//!    learning column's `auto`, which hands the choice back to detection, and
//!    a blank opposite it — so both columns scroll exactly the catalog and the
//!    same language lands on the same row. Each column keeps its pick inverted
//!    so the pair reads whole; the focused column's heading and pick are
//!    bright. A column longer than the window scrolls, showing a thumb against
//!    the right edge. Action row is `[↑ ↓] pick · [← →] column · [Enter]
//!    confirm · [Esc] cancel`. No text input, no cursor.
//!
//! All modals are rendered last in the frame so they sit on top of the
//! fullscreen screen beneath them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::picker::{PickerCursor, PickerSection};
use crate::tui::screen::ModalKind;
use crate::tui::text_field::TextField;

/// The halves of the pair in render order.
const SECTIONS: [PickerSection; 2] = [PickerSection::Known, PickerSection::Learning];

const TEXT_MODAL_WIDTH: u16 = 64;
const TEXT_MODAL_HEIGHT: u16 = 7;
const HORIZONTAL_PADDING: u16 = 2;
const INPUT_LINE_OFFSET: u16 = 1;

/// Draw the modal of the requested kind.
pub fn draw(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    match kind {
        ModalKind::PickLanguages => draw_picker(frame, area, app),
        ModalKind::ChangeSomething => draw_text_modal(frame, area, kind, app),
    }
}

fn draw_text_modal(frame: &mut Frame, area: Rect, kind: ModalKind, app: &App) {
    let inset = super::common::overlay_rect(area, TEXT_MODAL_WIDTH, TEXT_MODAL_HEIGHT);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = surround();
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let content = padded(inner);
    frame.render_widget(text_panel(kind, app, content.width as usize), content);
    paint_title(frame, inset, text_title(kind));
    if content.width == 0 || content.height == 0 {
        return;
    }
    let buffer_width = text_field(kind, app).cursor_offset();
    let cursor_x = (content.x + buffer_width).min(content.x + content.width.saturating_sub(1));
    let cursor_y = content.y + INPUT_LINE_OFFSET.min(content.height.saturating_sub(1));
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn draw_picker(frame: &mut Frame, area: Rect, app: &App) {
    let inset = picker_inset(area);
    super::common::paint_background(frame, inset);
    frame.render_widget(Clear, inset);
    let block = surround();
    let inner = block.inner(inset);
    frame.render_widget(block, inset);
    let content = padded(inner);
    frame.render_widget(picker_panel(app, picker_rows(area)), content);
    paint_title(frame, inset, "languages");
}

fn surround() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::FG).bg(palette::BG))
        .style(palette::base())
}

fn paint_title(frame: &mut Frame, inset: Rect, label: &str) {
    let title = Span::styled(format!(" {label} "), palette::base());
    let title_rect = Rect {
        x: inset.x + 2,
        y: inset.y,
        width: title.content.chars().count() as u16,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(title)).style(palette::base()),
        title_rect,
    );
}

fn padded(inner: Rect) -> Rect {
    Rect {
        x: inner.x + HORIZONTAL_PADDING,
        y: inner.y,
        width: inner.width.saturating_sub(HORIZONTAL_PADDING * 2),
        height: inner.height,
    }
}

fn text_title(kind: ModalKind) -> &'static str {
    match kind {
        ModalKind::ChangeSomething => "what meanings did we miss?",
        ModalKind::PickLanguages => "languages",
    }
}

fn text_field<'a>(kind: ModalKind, app: &'a App) -> TextField<'a> {
    TextField::new(app.modal_buffer(), text_placeholder(kind))
}

fn text_placeholder(kind: ModalKind) -> &'static str {
    match kind {
        ModalKind::ChangeSomething => "write the missing meaning however you want",
        ModalKind::PickLanguages => "",
    }
}

fn text_panel(kind: ModalKind, app: &App, width: usize) -> Paragraph<'static> {
    let dashes = "─".repeat(width);
    let mut action_spans = super::common::FooterHint::ghost("Esc", "cancel").spans();
    action_spans.push(Span::styled(String::from("    "), palette::base()));
    action_spans.extend(super::common::FooterHint::primary("Enter", "send").spans());
    let actions = Line::from(action_spans);
    let lines = vec![
        Line::from(""),
        text_field(kind, app).line(),
        Line::from(Span::styled(dashes, palette::rule())),
        Line::from(""),
        actions,
    ];
    Paragraph::new(lines).style(palette::base())
}

/// Rows of chrome the modal spends on everything that is not a scrolling row:
/// the border, one blank line, the headings, the rule under them, the pinned
/// row, one blank line, and the action row.
const PICKER_CHROME: usize = 8;
/// Cells between the two columns, wide enough to centre the arrow.
const COLUMN_GAP: usize = 5;
/// Cells the code cell occupies, sized for the widest label (`auto`).
const CODE_WIDTH: usize = 4;
/// Cells the action row needs: `[↑ ↓] pick  [← →] column  [Enter] confirm  [Esc] cancel`.
const ACTION_ROW_WIDTH: usize = 63;
/// Cells each column spends to the right of its text: one blank, then the
/// scrollbar. The blank is what keeps a full-width row off the bar.
const SCROLLBAR_GUTTER: usize = 2;
/// Rows between the modal's top border and the pinned row: blank, headings, rule.
const PINNED_ROW: u16 = 4;

fn picker_panel(app: &App, visible: usize) -> Paragraph<'static> {
    let cursor = app.picker_cursor();
    let mut lines = vec![
        Line::from(""),
        Line::from(headings(cursor.section())),
        Line::from(rules()),
        Line::from(across(gutter(), |section| pinned_cell(section, cursor))),
    ];
    for row in 0..visible {
        lines.push(Line::from(across(gutter(), |section| {
            scrolling_cell(section, cursor, row, visible)
        })));
    }
    let mut actions = super::common::FooterHint::secondary("↑ ↓", "pick").spans();
    actions.push(Span::styled(String::from("  "), palette::base()));
    actions.extend(super::common::FooterHint::secondary("← →", "column").spans());
    actions.push(Span::styled(String::from("  "), palette::base()));
    actions.extend(super::common::FooterHint::primary("Enter", "confirm").spans());
    actions.push(Span::styled(String::from("  "), palette::base()));
    actions.extend(super::common::FooterHint::ghost("Esc", "cancel").spans());
    lines.push(Line::from(""));
    lines.push(Line::from(actions));
    Paragraph::new(lines).style(palette::base())
}

/// Lay one row across both columns, separated by the gutter.
///
/// Every row spends the same cells on a column — its text plus the scrollbar
/// gutter — whether or not it draws anything there. That is what keeps the
/// headings, the rule and the list rows all starting the second column at the
/// same x, so the mouse geometry can describe a column with one span.
fn across(
    gutter: Span<'static>,
    cell: impl Fn(PickerSection) -> Vec<Span<'static>>,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for section in SECTIONS {
        if section == PickerSection::Learning {
            spans.push(gutter.clone());
        }
        let cells = cell(section);
        let filled: usize = cells
            .iter()
            .map(|span| super::common::display_width(span.content.as_ref()))
            .sum();
        let owed = (column_width(section) + SCROLLBAR_GUTTER).saturating_sub(filled);
        spans.extend(cells);
        spans.push(Span::styled(" ".repeat(owed), palette::base()));
    }
    spans
}

/// Build the heading row: both column titles with the arrow between them, the
/// focused title bright.
///
/// The arrow sits beside the names of the two sides rather than beside any two
/// languages, so it says which way the pair reads without pretending to link
/// whichever rows happen to be level with it.
fn headings(focused: PickerSection) -> Vec<Span<'static>> {
    across(arrow(), |section| {
        let style = if section == focused {
            palette::base().add_modifier(Modifier::BOLD)
        } else {
            palette::dim2()
        };
        vec![Span::styled(String::from(section.heading()), style)]
    })
}

/// The pair arrow, centred inside the gutter.
fn arrow() -> Span<'static> {
    let lead = (COLUMN_GAP - 1) / 2;
    Span::styled(
        format!("{}→{}", " ".repeat(lead), " ".repeat(COLUMN_GAP - 1 - lead)),
        palette::dim(),
    )
}

/// The plain gutter every row but the headings uses.
fn gutter() -> Span<'static> {
    Span::styled(" ".repeat(COLUMN_GAP), palette::base())
}

/// Build the rule that separates each heading from its own list. Ruling the
/// columns separately, rather than the whole modal, also shows how wide each
/// one is.
fn rules() -> Vec<Span<'static>> {
    across(gutter(), |section| {
        vec![Span::styled(
            "─".repeat(column_width(section)),
            palette::rule(),
        )]
    })
}

/// Build the row pinned above both lists. Only the learning column fills it,
/// with `auto`; the blank left of it is what lines the two catalogs up.
fn pinned_cell(section: PickerSection, cursor: PickerCursor) -> Vec<Span<'static>> {
    let pinned = (0..section.scrolling_first()).next();
    let text = pinned
        .map(|index| row_text(section, index))
        .unwrap_or_default();
    let selected = pinned.is_some_and(|index| index == cursor.index(section));
    vec![Span::styled(
        super::common::pad_right(text.as_str(), column_width(section)),
        row_style(selected, cursor.section() == section),
    )]
}

/// Build one column's cell on one scrolling row: `CODE  Endonym`, padded to the
/// column width, followed by that column's scrollbar cell.
fn scrolling_cell(
    section: PickerSection,
    cursor: PickerCursor,
    row: usize,
    visible: usize,
) -> Vec<Span<'static>> {
    let first = section.scrolling_first();
    let total = section.scrolling();
    let offset = window(total, cursor.index(section).saturating_sub(first), visible);
    let index = first + offset + row;
    let text = if offset + row < total {
        row_text(section, index)
    } else {
        String::new()
    };
    let selected = offset + row < total && index == cursor.index(section);
    vec![
        Span::styled(
            super::common::pad_right(text.as_str(), column_width(section)),
            row_style(selected, cursor.section() == section),
        ),
        Span::styled(" ", palette::base()),
        scrollbar_cell(total, offset, row, visible),
    ]
}

/// Render one row as its two cells. The `auto` row goes through here too, so
/// it lines up with the languages instead of being a special case.
fn row_text(section: PickerSection, index: usize) -> String {
    format!(
        "{}  {}",
        super::common::pad_right(section.label_at(index).as_str(), CODE_WIDTH),
        section.name_at(index)
    )
}

/// Paint one row. The pick of each column stays inverted so the pair reads at
/// a glance; only the focused column's pick is also bold.
fn row_style(selected: bool, focused: bool) -> Style {
    match (selected, focused) {
        (true, true) => palette::invert().add_modifier(Modifier::BOLD),
        (true, false) => palette::invert(),
        (false, _) => palette::dim(),
    }
}

/// Return the scrollbar cell for one visible row, blank while the whole column
/// fits. The thumb covers the slice of the track the window is looking at.
fn scrollbar_cell(total: usize, offset: usize, row: usize, visible: usize) -> Span<'static> {
    if total <= visible || visible == 0 {
        return Span::styled(" ", palette::base());
    }
    let first = offset * visible / total;
    let height = (visible * visible).div_ceil(total).max(1);
    if row >= first && row < first + height {
        return Span::styled("┃", palette::dim());
    }
    Span::styled("│", palette::rule())
}

/// Return the first list index one column shows, keeping the pick in view.
///
/// The offset is derived from the pick rather than stored, so the renderer and
/// the mouse hit-test cannot drift apart and the selected row is visible by
/// construction.
#[must_use]
pub fn window(total: usize, selected: usize, visible: usize) -> usize {
    selected
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible))
}

/// Return the width one column's text occupies, before its scrollbar cell.
fn column_width(section: PickerSection) -> usize {
    let widest = (0..section.chips())
        .map(|index| super::common::display_width(row_text(section, index).as_str()))
        .max()
        .unwrap_or(0);
    widest.max(super::common::display_width(section.heading()))
}

/// Return how many list rows the modal shows inside `area`.
///
/// Both the renderer and the geometry call this, so the clamping `overlay_rect`
/// applies on a short terminal is accounted for once, in one place.
#[must_use]
pub fn picker_rows(area: Rect) -> usize {
    let ceiling = usize::from(
        super::common::frame_rects(area)
            .disclaimer
            .y
            .saturating_sub(area.y),
    );
    let longest = SECTIONS
        .into_iter()
        .map(PickerSection::scrolling)
        .max()
        .unwrap_or(0);
    ceiling.saturating_sub(PICKER_CHROME).clamp(1, longest)
}

fn picker_inset(area: Rect) -> Rect {
    let height = u16::try_from(picker_rows(area) + PICKER_CHROME).unwrap_or(u16::MAX);
    super::common::overlay_rect(area, picker_width(), height)
}

/// Return the modal width: whatever the two columns and the action row need.
fn picker_width() -> u16 {
    let columns = SECTIONS
        .into_iter()
        .map(|section| column_width(section) + SCROLLBAR_GUTTER)
        .sum::<usize>()
        + COLUMN_GAP;
    let content = columns.max(ACTION_ROW_WIDTH);
    u16::try_from(content + usize::from(2 + HORIZONTAL_PADDING * 2)).unwrap_or(u16::MAX)
}

/// Geometry helpers exported so the input layer can mouse-hit-test the rows
/// inside the language pair modal.
pub mod picker_geometry {
    use super::{
        COLUMN_GAP, HORIZONTAL_PADDING, PINNED_ROW, SCROLLBAR_GUTTER, SECTIONS, column_width,
        picker_inset, picker_rows, window,
    };
    use crate::tui::picker::{PickerCursor, PickerSection};
    use ratatui::layout::Rect;

    /// Return the column and list index that landed under `(x, y)` inside
    /// `area`, or `None` if the click missed every row.
    pub fn row_at(
        area: Rect,
        cursor: PickerCursor,
        x: u16,
        y: u16,
    ) -> Option<(PickerSection, usize)> {
        SECTIONS.into_iter().find_map(|section| {
            (0..section.chips()).find_map(|index| {
                row_rect(area, cursor, section, index)
                    .filter(|rect| y == rect.y && x >= rect.x && x < rect.x + rect.width)
                    .map(|_| (section, index))
            })
        })
    }

    /// Return the column column `x` falls in, so the wheel scrolls the list
    /// under the pointer. A pointer between or beside the columns keeps
    /// whichever column already has focus.
    pub fn column_at(area: Rect, x: u16, focused: PickerSection) -> PickerSection {
        SECTIONS
            .into_iter()
            .find(|section| {
                let (start, width) = column_span(area, *section);
                x >= start && x < start.saturating_add(width)
            })
            .unwrap_or(focused)
    }

    /// Return the rendered rectangle for one row, or `None` when the column is
    /// scrolled past it — an undrawn row must not answer a click.
    ///
    /// The pinned row sits above the list and never scrolls, so it is always
    /// drawn and always clickable.
    pub fn row_rect(
        area: Rect,
        cursor: PickerCursor,
        section: PickerSection,
        index: usize,
    ) -> Option<Rect> {
        let inset = picker_inset(area);
        let (x, width) = column_span(area, section);
        let seat = |row: u16| {
            let y = inset.y + PINNED_ROW + row;
            (y < inset.y + inset.height.saturating_sub(1)).then_some(Rect {
                x,
                y,
                width,
                height: 1,
            })
        };
        let first = section.scrolling_first();
        let Some(scrolling) = index.checked_sub(first) else {
            return seat(0);
        };
        let visible = picker_rows(area);
        let offset = window(
            section.scrolling(),
            cursor.index(section).saturating_sub(first),
            visible,
        );
        let row = scrolling.checked_sub(offset)?;
        if row >= visible {
            return None;
        }
        seat(1 + u16::try_from(row).unwrap_or(u16::MAX))
    }

    /// Return where one column starts and how wide its text cells are.
    fn column_span(area: Rect, section: PickerSection) -> (u16, u16) {
        let inset = picker_inset(area);
        let left = inset.x + 1 + HORIZONTAL_PADDING;
        let width = u16::try_from(column_width(section)).unwrap_or(u16::MAX);
        match section {
            PickerSection::Known => (left, width),
            PickerSection::Learning => {
                let known = u16::try_from(
                    column_width(PickerSection::Known) + SCROLLBAR_GUTTER + COLUMN_GAP,
                )
                .unwrap_or(u16::MAX);
                (left.saturating_add(known), width)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use crate::tui::picker::{PickerCursor, PickerSection};

    use super::{SECTIONS, column_width, picker_geometry, picker_rows, window};

    /// A terminal tall enough to show every language without scrolling.
    fn tall() -> Rect {
        Rect::new(0, 0, 96, 40)
    }

    /// A terminal that forces both columns to scroll.
    fn short() -> Rect {
        Rect::new(0, 0, 96, 20)
    }

    /// The same language sits on the same row in both columns while neither is
    /// scrolled — the whole point of pinning `auto` above the lists instead of
    /// leaving it as the learning column's first entry.
    #[test]
    fn the_same_language_shares_a_row_across_both_columns() {
        let area = tall();
        let seat = cursor(0, 1, PickerSection::Known);
        let misaligned = (0..PickerSection::Known.scrolling())
            .filter(|offset| {
                let known = picker_geometry::row_rect(
                    area,
                    seat,
                    PickerSection::Known,
                    PickerSection::Known.scrolling_first() + offset,
                );
                let learning = picker_geometry::row_rect(
                    area,
                    seat,
                    PickerSection::Learning,
                    PickerSection::Learning.scrolling_first() + offset,
                );
                known.map(|rect| rect.y) != learning.map(|rect| rect.y)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            misaligned,
            Vec::<usize>::new(),
            "a language stopped sharing its row with the same language in the other column"
        );
    }

    /// The pinned row never scrolls away, so `auto` is always one click off.
    #[test]
    fn the_pinned_row_stays_put_however_far_the_column_scrolls() {
        let area = short();
        let last = PickerSection::Learning.chips() - 1;
        let scrolled = cursor(0, last, PickerSection::Learning);
        assert_eq!(
            picker_geometry::row_rect(area, scrolled, PickerSection::Learning, 0)
                .map(|rect| rect.y),
            picker_geometry::row_rect(
                area,
                cursor(0, 0, PickerSection::Learning),
                PickerSection::Learning,
                0
            )
            .map(|rect| rect.y),
            "the pinned auto row moved when the list beneath it scrolled"
        );
    }

    fn cursor(known: usize, learning: usize, section: PickerSection) -> PickerCursor {
        PickerCursor::new(known, learning, section)
    }

    /// The window is derived from the pick, so the pick is visible whatever it
    /// is — this is the invariant that lets the modal keep no scroll state.
    #[test]
    fn the_window_always_contains_the_pick() {
        let visible = picker_rows(short());
        let escaped = SECTIONS
            .into_iter()
            .flat_map(|section| {
                let total = section.chips();
                [0, total / 2, total - 1].map(move |selected| (section, total, selected))
            })
            .filter(|(_, total, selected)| {
                let offset = window(*total, *selected, visible);
                *selected < offset || *selected >= offset + visible
            })
            .map(|(section, _, selected)| (section, selected))
            .collect::<Vec<_>>();
        assert_eq!(
            escaped,
            Vec::<(PickerSection, usize)>::new(),
            "a pick fell outside the window the modal draws"
        );
    }

    /// Every row a tall terminal draws answers a click at its own rectangle.
    #[test]
    fn every_drawn_row_round_trips_through_its_hit_region() {
        let area = tall();
        let seat = cursor(0, 0, PickerSection::Known);
        let broken = SECTIONS
            .into_iter()
            .flat_map(|section| (0..section.chips()).map(move |index| (section, index)))
            .filter(|(section, index)| {
                let Some(rect) = picker_geometry::row_rect(area, seat, *section, *index) else {
                    return true;
                };
                picker_geometry::row_at(area, seat, rect.x + 1, rect.y) != Some((*section, *index))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            broken,
            Vec::<(PickerSection, usize)>::new(),
            "a drawn row lost or misreported its mouse hit region"
        );
    }

    /// A row the column has scrolled past is not drawn, so it answers nothing.
    #[test]
    fn a_row_outside_the_window_reports_no_hit_region() {
        let area = short();
        let last = PickerSection::Learning.chips() - 1;
        assert_eq!(
            picker_geometry::row_rect(
                area,
                cursor(0, 0, PickerSection::Known),
                PickerSection::Learning,
                last
            ),
            None,
            "a row scrolled out of view must not answer a click"
        );
    }

    /// Scrolling to the end brings the last row into view — nothing is
    /// unreachable, which is what makes a scrolling list honest.
    #[test]
    fn scrolling_to_the_end_reveals_the_last_row() {
        let area = short();
        let last = PickerSection::Learning.chips() - 1;
        let seat = cursor(0, last, PickerSection::Learning);
        assert!(
            picker_geometry::row_rect(area, seat, PickerSection::Learning, last).is_some(),
            "the last language stayed unreachable however far the column scrolled"
        );
    }

    /// The columns share every row, so they must not share a single column.
    #[test]
    fn the_columns_never_share_a_cell() {
        let area = tall();
        let seat = cursor(0, 0, PickerSection::Known);
        let known = picker_geometry::row_rect(area, seat, PickerSection::Known, 0)
            .expect("the known column must draw its first row");
        let learning = picker_geometry::row_rect(area, seat, PickerSection::Learning, 0)
            .expect("the learning column must draw its first row");
        assert!(
            known.x + known.width <= learning.x,
            "the learning column overlapped the known column"
        );
    }

    /// A wheel tick lands in the column under the pointer, and keeps the
    /// focused one when the pointer sits between them.
    #[test]
    fn the_pointer_picks_which_column_the_wheel_scrolls() {
        let area = tall();
        let learning = picker_geometry::row_rect(
            area,
            cursor(0, 0, PickerSection::Known),
            PickerSection::Learning,
            0,
        )
        .expect("the learning column must draw its first row");
        assert_eq!(
            (
                picker_geometry::column_at(area, learning.x + 1, PickerSection::Known),
                picker_geometry::column_at(area, 0, PickerSection::Learning),
            ),
            (PickerSection::Learning, PickerSection::Learning),
            "the wheel scrolled a column the pointer was not over"
        );
    }

    /// A column is sized by its own widest row, so a longer language name
    /// widens the modal instead of being clipped.
    #[test]
    fn a_column_is_wide_enough_for_its_widest_row() {
        let narrow = SECTIONS
            .into_iter()
            .filter(|section| {
                column_width(*section) < super::super::common::display_width(section.heading())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            narrow,
            Vec::<PickerSection>::new(),
            "a column is narrower than its own heading"
        );
    }
}
