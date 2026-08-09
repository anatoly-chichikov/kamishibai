//! Hit-testing for clickable elements in the TUI.
//!
//! ratatui draws into a cell buffer and the host terminal does not
//! auto-detect links inside that buffer the way it would for raw stdout.
//! The shell captures mouse clicks via crossterm and consults `link_at`
//! to decide whether the click landed on a path that should be opened
//! with the system handler.

use ratatui::layout::Rect;

use super::App;
use super::event::AppEvent;
use super::screen::{Screen, WelcomeFocus, WelcomeStage};
use super::screens::banner;
use super::screens::common::{CARD_DETAIL_COLUMN, GUTTER, TOP_MARGIN, frame_rects, language_chip};
use super::screens::sentence_labels::EditorControl;
use super::screens::welcome;
use super::screens::what_i_understood::{SentenceSettingsControl, sentence_settings_control_at};
use super::screens::your_cards::{
    artifact_file_label, card_range_at, detail_pane_height, head_rows_for, rejected_attempts,
    rejected_link_columns, rejected_rows_offset, sentence_editor_control_at,
    sentence_label_extra_rows, sentence_tag_hit_at, sentence_tags_visible, step_rows_for,
};
use super::sentence_editor::LabelEditorRow;

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

/// Return which Welcome key-step control one cell lands on, if any. The key
/// input, the `submit` chip, and (when env offers a key) the `load from env`
/// chip are the click targets.
pub fn welcome_control_at(
    app: &App,
    terminal: Rect,
    click_x: u16,
    click_y: u16,
) -> Option<WelcomeFocus> {
    if app.screen() != Screen::Welcome || app.welcome().stage != WelcomeStage::EnterKey {
        return None;
    }
    welcome::control_at(app, terminal, click_x, click_y)
}

/// Return the inline sentence-label action at one terminal cell.
pub fn sentence_label_event_at(
    app: &App,
    terminal: Rect,
    click_x: u16,
    click_y: u16,
) -> Option<AppEvent> {
    if app.screen() != Screen::YourCards {
        return None;
    }
    let frame = frame_rects(terminal);
    let banner_rows = if banner_visible(app) {
        banner::height(app)
    } else {
        0
    };
    let cards_y = frame.body.y.saturating_add(banner_rows);
    let cards_height = frame.body.height.saturating_sub(banner_rows);
    if click_x < frame.body.x
        || click_x >= frame.body.x.saturating_add(frame.body.width)
        || click_y < cards_y
        || click_y >= cards_y.saturating_add(cards_height)
    {
        return None;
    }
    let width = usize::from(frame.body.width);
    let column = usize::from(click_x.saturating_sub(frame.body.x));
    let visible_row = usize::from(click_y.saturating_sub(cards_y));
    let content_row = usize::from(app.body_scroll()).saturating_add(visible_row);
    let (card, card_start, _) = card_range_at(app, width, content_row)?;
    let draft = app.cards().get(card)?;
    let labels = draft
        .meta()
        .and_then(crate::session::CardMeta::sentence_labels);
    let staged = draft.staged_rewrite().map(|rewrite| rewrite.selection());
    let selected = card == app.card_selected();
    let editor = if selected {
        app.sentence_editor()
    } else {
        None
    };
    let head_start = card_start;
    let expanded = selected && app.card_expanded();
    let head_height = head_rows_for(draft, width);
    let head_end = head_start.saturating_add(head_height);
    let running = app
        .cards_running_target()
        .and_then(|(running_card, artifact)| (running_card == card).then_some(artifact));
    let steps = step_rows_for(draft, running);
    let meta_row = head_end.saturating_add(
        steps
            .iter()
            .position(|artifact| *artifact == crate::session::Artifact::Meta)?,
    );
    let attributed = labels.is_some()
        || staged.is_some_and(crate::session::SentenceLabelSelection::attributed)
        || editor.is_some_and(|editor| editor.selection().attributed());
    if !expanded
        && (!attributed
            || !steps.contains(&crate::session::Artifact::Sound)
            || !sentence_tags_visible(draft, running, width))
        && app.card_tunable_at(card)
        && content_row >= head_start
        && content_row < head_end
    {
        return Some(AppEvent::SentenceLabelOpen(card, LabelEditorRow::Register));
    }
    if !expanded {
        let tag_row = content_row.checked_sub(meta_row)?;
        if sentence_tag_hit_at(draft, running, width, tag_row, column) {
            return Some(AppEvent::SentenceLabelOpen(card, LabelEditorRow::Register));
        }
        return None;
    }
    let editor = editor?;
    let editor_row = content_row.checked_sub(meta_row)?;
    match sentence_editor_control_at(draft, running, editor, width, editor_row, column)? {
        EditorControl::Chip(row, index) => Some(AppEvent::SentenceLabelChoose(row, index)),
        EditorControl::Advance(row, forward) => Some(AppEvent::SentenceLabelAdvance(row, forward)),
        EditorControl::Note => Some(AppEvent::SentenceLabelFocus(LabelEditorRow::Note)),
    }
}

/// Return the batch sentence-settings action at one terminal cell.
pub fn sentence_settings_event_at(
    app: &App,
    terminal: Rect,
    click_x: u16,
    click_y: u16,
) -> Option<AppEvent> {
    if app.screen() != Screen::WhatIUnderstood
        || app.modal().is_some()
        || app.busy().is_some()
        || app.error().is_some()
    {
        return None;
    }
    let frame = frame_rects(terminal);
    if click_x < frame.body.x
        || click_x >= frame.body.x.saturating_add(frame.body.width)
        || click_y < frame.body.y
        || click_y >= frame.body.y.saturating_add(frame.body.height)
    {
        return None;
    }
    let width = usize::from(frame.body.width);
    let column = usize::from(click_x.saturating_sub(frame.body.x));
    let visible_row = usize::from(click_y.saturating_sub(frame.body.y));
    let content_row = usize::from(app.body_scroll()).saturating_add(visible_row);
    match sentence_settings_control_at(app, width, column, content_row)? {
        SentenceSettingsControl::Open => Some(AppEvent::SentenceSettingsOpen),
        SentenceSettingsControl::Editor(
            super::screens::sentence_labels::BatchEditorControl::Chip(row, index),
        ) => Some(AppEvent::SentenceSettingsChoose(row, index)),
        SentenceSettingsControl::Editor(
            super::screens::sentence_labels::BatchEditorControl::Advance(row, forward),
        ) => Some(AppEvent::SentenceSettingsAdvance(row, forward)),
    }
}

fn link_regions(app: &App, terminal: Rect) -> Vec<LinkRegion> {
    let mut links = Vec::new();
    if !matches!(app.screen(), Screen::YourCards | Screen::Done) {
        return links;
    }
    let frame = frame_rects(terminal);
    let body_y = frame.body.y;
    let body_x = frame.body.x;
    let body_width = frame.body.width;
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
    let body_height = frame.body.height.saturating_sub(banner_rows);
    let mut content_row = 0usize;
    let width = usize::from(body_width);
    for (idx, draft) in app.cards().iter().enumerate() {
        let running = app
            .cards_running_target()
            .and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let expanded = idx == app.card_selected() && app.card_expanded();
        let editor = if idx == app.card_selected() {
            app.sentence_editor()
        } else {
            None
        };
        let head_height = head_rows_for(draft, width);
        let steps = step_rows_for(draft, running);
        let detail = if expanded {
            detail_pane_height(draft, width)
        } else {
            0
        };
        let labels = sentence_label_extra_rows(draft, running, editor, expanded, width);
        let trailing = usize::from(!steps.is_empty() || detail > 0 || labels > 0);
        let card_total = head_height + steps.len() + labels + detail + trailing;
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
            let label = artifact_file_label(*artifact, file);
            let label_start = body_x
                + u16::try_from(CARD_DETAIL_COLUMN)
                    .expect("invariant: card detail column must fit in u16");
            let label_end = label_start
                .saturating_add(u16::try_from(label.chars().count()).unwrap_or(u16::MAX));
            links.push(LinkRegion {
                row: screen_row,
                hit_start: label_start,
                hit_end: label_end,
                target: file.path().to_string_lossy().into_owned(),
            });
        }
        if detail > 0 {
            links.extend(rejected_regions(
                draft,
                body_x,
                body_y + banner_rows,
                body_height,
                content_row + head_height + steps.len() + labels,
                app.body_scroll(),
                width,
            ));
        }
        content_row += card_total;
    }
    links
}

fn rejected_regions(
    draft: &crate::session::CardDraft,
    body_x: u16,
    body_start: u16,
    body_height: u16,
    pane_row: usize,
    scroll: u16,
    width: usize,
) -> Vec<LinkRegion> {
    let Some(offset) = rejected_rows_offset(draft, width) else {
        return Vec::new();
    };
    rejected_attempts(draft)
        .into_iter()
        .enumerate()
        .filter_map(|(row, attempt)| {
            let absolute = pane_row + offset + row;
            let screen_row = visible_content_row(body_start, body_height, absolute, scroll)?;
            Some(rejected_link_columns(attempt, width).into_iter().map(
                move |(start, end, target)| LinkRegion {
                    row: screen_row,
                    hit_start: body_x.saturating_add(start),
                    hit_end: body_x.saturating_add(end),
                    target: target.to_string_lossy().into_owned(),
                },
            ))
        })
        .flatten()
        .collect()
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
