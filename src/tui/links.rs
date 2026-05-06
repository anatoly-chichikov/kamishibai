//! Hit-testing for clickable elements in the TUI.
//!
//! ratatui draws into a cell buffer and the host terminal does not
//! auto-detect links inside that buffer the way it would for raw stdout.
//! The shell captures mouse clicks via crossterm and consults `link_at`
//! to decide whether the click landed on a path that should be opened
//! with the system handler.

use ratatui::layout::Rect;

use crate::session::Artifact;

use super::App;
use super::screen::Screen;
use super::screens::banner;
use super::screens::common::{GUTTER, HEADER_GAP, TOP_MARGIN, language_chip};
use super::screens::your_cards::{detail_pane_height, head_rows_for};

const STEP_ARTIFACT_ORDER: [Artifact; 4] = [
    Artifact::Body,
    Artifact::Sound,
    Artifact::Scene,
    Artifact::Picture,
];

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
    if !matches!(app.screen(), Screen::YourCards | Screen::Done) {
        return None;
    }
    let body_y = TOP_MARGIN + 1 + HEADER_GAP;
    let body_x = GUTTER;
    let body_width = terminal.width.saturating_sub(GUTTER * 2);
    if click_x < body_x || click_x >= body_x + body_width {
        return None;
    }
    let banner_rows = if banner_visible(app) {
        banner::height(app)
    } else {
        0
    };
    let banner_top = body_y.saturating_sub(banner::LIFT);
    if banner_rows > 0 && click_y >= banner_top && click_y < banner_top + banner_rows {
        return banner_label_hit(app, body_x, click_x, click_y - banner_top);
    }
    if click_y < body_y {
        return None;
    }
    if app.screen() != Screen::YourCards {
        return None;
    }
    let row_in_body = click_y - body_y;
    if row_in_body < banner_rows {
        return None;
    }
    let mut row = (row_in_body - banner_rows) as usize + app.body_scroll() as usize;
    let width = usize::from(body_width);
    for (idx, draft) in app.cards().iter().enumerate() {
        let head_height = head_rows_for(draft, width);
        let steps_height = STEP_ARTIFACT_ORDER.len();
        let detail = if idx == app.card_selected() && app.card_expanded() {
            detail_pane_height(draft)
        } else {
            0
        };
        let card_total = head_height + steps_height + detail + 1; // + trailing blank
        if row < head_height {
            return None;
        }
        if row < head_height + steps_height {
            let step_idx = row - head_height;
            let artifact = STEP_ARTIFACT_ORDER[step_idx];
            let slot = match artifact {
                Artifact::Body => draft.artifacts().body(),
                Artifact::Sound => draft.artifacts().sound(),
                Artifact::Scene => draft.artifacts().scene(),
                Artifact::Picture => draft.artifacts().picture(),
            };
            let file = slot.file()?;
            return Some(file.path().to_string_lossy().into_owned());
        }
        if row < card_total {
            return None;
        }
        row -= card_total;
    }
    None
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

fn banner_label_hit(app: &App, body_x: u16, click_x: u16, row: u16) -> Option<String> {
    let entries = banner::entries(app);
    let (label, path) = entries.get(row as usize)?;
    let prefix = banner::INDENT_WIDTH as u16 + banner::GLYPH.chars().count() as u16;
    let label_start = body_x + prefix;
    let label_len = label.chars().count() as u16;
    let label_end = label_start.saturating_add(label_len);
    if click_x >= label_start && click_x < label_end {
        return Some(String::from(*path));
    }
    None
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}
