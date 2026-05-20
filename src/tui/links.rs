//! Hit-testing for clickable elements in the TUI.
//!
//! ratatui draws into a cell buffer and the host terminal does not
//! auto-detect links inside that buffer the way it would for raw stdout.
//! The shell captures mouse clicks via crossterm and consults `link_at`
//! to decide whether the click landed on a path that should be opened
//! with the system handler.

use ratatui::layout::Rect;

use super::App;
use super::screen::Screen;
use super::screens::banner;
use super::screens::common::{GUTTER, HEADER_GAP, TOP_MARGIN, language_chip};
use super::screens::your_cards::{detail_pane_height, head_rows_for, step_rows_for};

const STEP_FILE_LABEL_START: u16 = 15;

#[derive(Clone, Debug, Eq, PartialEq)]
struct LinkRegion {
    row: u16,
    hit_start: u16,
    hit_end: u16,
    target: String,
}

impl LinkRegion {
    fn contains(&self, column: u16, row: u16) -> bool {
        row == self.row && column >= self.hit_start && column < self.hit_end
    }
}

/// Return `true` if the click landed on the language chip in the header row
/// AND the active screen actually allows the user to change `my` language.
///
/// The chip lives at `(area.y + TOP_MARGIN)` row, anchored to the right edge
/// at `(area.x + area.width - GUTTER)` minus the chip width. Welcome doesn't
/// render a chip at all; YourCards and Done do render one but the batch pair
/// is frozen, so clicks there are inert (the same rule the keyboard path
/// follows in `transit`).
pub fn language_chip_at(app: &App, terminal: Rect, click_x: u16, click_y: u16) -> bool {
    if !matches!(app.screen(), Screen::YourWords | Screen::WhatIUnderstood) {
        return false;
    }
    let header_y = terminal.y + TOP_MARGIN;
    if click_y != header_y {
        return false;
    }
    let chip_width: u16 = language_chip(app)
        .iter()
        .map(|span| span.content.chars().count() as u16)
        .sum();
    if chip_width == 0 {
        return false;
    }
    let right_edge = terminal.x + terminal.width.saturating_sub(GUTTER);
    let start = right_edge.saturating_sub(chip_width);
    click_x >= start && click_x < right_edge
}

/// Return the path that the click landed on, if any. Two clickable surfaces
/// live on `Your cards`:
/// 1. The sticky outputs banner (.apkg, .pdf, output folder) shown when all
///    cards finished — pinned to the top of the body rect and unaffected by
///    body scroll.
/// 2. Per-card step rows whose artifact is `ready` — clickable on the
///    rendered file label so the user can jump straight to that artifact.
pub fn link_at(app: &App, terminal: Rect, click_x: u16, click_y: u16) -> Option<String> {
    link_regions(app, terminal)
        .into_iter()
        .find(|region| region.contains(click_x, click_y))
        .map(|region| region.target)
}

fn link_regions(app: &App, terminal: Rect) -> Vec<LinkRegion> {
    let mut links = Vec::new();
    if !matches!(app.screen(), Screen::YourCards | Screen::Done) {
        return links;
    }
    let body_y = terminal.y + TOP_MARGIN + 1 + HEADER_GAP;
    let body_x = terminal.x + GUTTER;
    let body_width = terminal.width.saturating_sub(GUTTER * 2);
    let banner_rows = if banner_visible(app) {
        banner::height(app)
    } else {
        0
    };
    if banner_rows > 0 {
        links.extend(banner_regions(app, body_x, body_y));
    }
    if app.screen() != Screen::YourCards {
        return links;
    }
    let body_height = terminal
        .height
        .saturating_sub(TOP_MARGIN + 1 + HEADER_GAP)
        .saturating_sub(2)
        .saturating_sub(banner_rows);
    let mut content_row = 0usize;
    let width = usize::from(body_width);
    for (idx, draft) in app.cards().iter().enumerate() {
        let running = app
            .cards_running_target()
            .and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let head_height = head_rows_for(draft, width);
        let steps = step_rows_for(draft, running);
        let detail = if idx == app.card_selected() && app.card_expanded() {
            detail_pane_height(draft, width)
        } else {
            0
        };
        let trailing = usize::from(!steps.is_empty() || detail > 0);
        let card_total = head_height + steps.len() + detail + trailing;
        for (step_idx, artifact) in steps.iter().enumerate() {
            let absolute = content_row + head_height + step_idx;
            let Some(screen_row) = visible_content_row(
                body_y + banner_rows,
                body_height,
                absolute,
                app.body_scroll(),
            ) else {
                continue;
            };
            let slot = match artifact {
                crate::session::Artifact::Meta => draft.artifacts().meta(),
                crate::session::Artifact::Sound => draft.artifacts().sound(),
                crate::session::Artifact::Scene => draft.artifacts().scene(),
                crate::session::Artifact::Picture => draft.artifacts().picture(),
            };
            let Some(file) = slot.file() else {
                continue;
            };
            let label_start = body_x + STEP_FILE_LABEL_START;
            let label_end = label_start
                .saturating_add(u16::try_from(file.name().chars().count()).unwrap_or(u16::MAX));
            links.push(LinkRegion {
                row: screen_row,
                hit_start: label_start,
                hit_end: label_end,
                target: file.path().to_string_lossy().into_owned(),
            });
        }
        content_row += card_total;
    }
    links
}

fn banner_visible(app: &App) -> bool {
    if !banner::has_entries(app) {
        return false;
    }
    match app.screen() {
        Screen::Done => true,
        Screen::YourCards => all_finished(app),
        _ => false,
    }
}

fn banner_regions(app: &App, body_x: u16, body_y: u16) -> Vec<LinkRegion> {
    let entries = banner::entries(app);
    entries
        .into_iter()
        .enumerate()
        .map(|(row, (label, path))| {
            let prefix = u16::try_from(banner::INDENT_WIDTH).unwrap_or(u16::MAX);
            let label_start = body_x
                .saturating_add(prefix)
                .saturating_add(u16::try_from(banner::GLYPH.chars().count()).unwrap_or(u16::MAX));
            let label_end = label_start
                .saturating_add(u16::try_from(label.chars().count()).unwrap_or(u16::MAX));
            LinkRegion {
                row: body_y.saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                hit_start: label_start,
                hit_end: label_end,
                target: String::from(path),
            }
        })
        .collect()
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}

fn visible_content_row(
    body_start: u16,
    body_height: u16,
    absolute: usize,
    scroll: u16,
) -> Option<u16> {
    let absolute = u16::try_from(absolute).ok()?;
    if absolute < scroll {
        return None;
    }
    let row = absolute - scroll;
    if row >= body_height {
        return None;
    }
    Some(body_start + row)
}
