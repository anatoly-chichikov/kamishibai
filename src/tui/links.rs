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
use super::screens::common::{GUTTER, HEADER_GAP, TOP_MARGIN, language_chip};

const STEP_ARTIFACT_ORDER: [Artifact; 4] = [
    Artifact::Body,
    Artifact::Sound,
    Artifact::Scene,
    Artifact::Picture,
];

/// Mirror of the outputs banner labels in `your_cards::outputs_banner`.
const BANNER_LABELS: [(&str, &str); 2] = [("↓", "APKG"), ("↓", "PDF")];

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
/// 1. The outputs banner (.apkg, .pdf, output folder) shown when all cards
///    finished — clickable on the short label row at the top, and on each
///    full path row beneath it.
/// 2. Per-card step rows whose artifact is `ready` — clickable on the
///    rendered file label so the user can jump straight to that artifact.
pub fn link_at(app: &App, terminal: Rect, click_x: u16, click_y: u16) -> Option<String> {
    if app.screen() != Screen::YourCards {
        return None;
    }
    let body_y = TOP_MARGIN + 1 + HEADER_GAP;
    let body_x = GUTTER;
    let body_width = terminal.width.saturating_sub(GUTTER * 2);
    if click_y < body_y {
        return None;
    }
    if click_x < body_x || click_x >= body_x + body_width {
        return None;
    }
    let mut row = (click_y - body_y) as usize + app.body_scroll() as usize;
    if all_finished(app) {
        let banner = banner_entries(app);
        if !banner.is_empty() {
            if row == 0 {
                return label_row_hit(&banner, body_x, click_x);
            }
            row = row.checked_sub(2)?; // skip label row + trailing blank
        }
    }
    for (idx, draft) in app.cards().iter().enumerate() {
        let head_height = 1usize;
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

struct BannerEntry<'a> {
    glyph: &'a str,
    label: &'a str,
    path: &'a str,
}

fn banner_entries(app: &App) -> Vec<BannerEntry<'_>> {
    let done = app.done_artifacts();
    let paths = [done.deck.as_str(), done.report.as_str()];
    BANNER_LABELS
        .iter()
        .zip(paths.iter())
        .filter_map(|((glyph, label), path)| {
            if path.is_empty() {
                None
            } else {
                Some(BannerEntry { glyph, label, path })
            }
        })
        .collect()
}

fn label_row_hit(entries: &[BannerEntry<'_>], body_x: u16, click_x: u16) -> Option<String> {
    let mut pos = 2u16; // skip "│ "
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            pos = pos.saturating_add(4); // separator
        }
        pos = pos.saturating_add(entry.glyph.chars().count() as u16 + 1); // glyph + space
        let label_start = body_x + pos;
        let label_len = entry.label.chars().count() as u16;
        let label_end = label_start.saturating_add(label_len);
        if click_x >= label_start && click_x < label_end {
            return Some(String::from(entry.path));
        }
        pos = pos.saturating_add(label_len);
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

/// Approximate height of the body preview detail pane to keep step click
/// hit-tests aligned even when a card is expanded. The shape is a verbatim
/// mirror of `your_cards::detail_pane` / `body_preview`.
fn detail_pane_height(draft: &crate::session::CardDraft) -> usize {
    let mut h = 1; // initial blank
    let Some(body) = draft.body() else {
        return h + 1; // "body not generated yet" placeholder
    };
    h += 2; // target label + value
    h += 1 + 2; // blank + source label + value
    h += 1 + 2; // blank + hint label + value
    h += 1 + 2; // blank + meaning label + value
    if !body.source_context().trim().is_empty() {
        h += 1 + 1; // blank + context label
        h += body.source_context().lines().count();
    }
    h
}
