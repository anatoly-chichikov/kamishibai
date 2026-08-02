//! Sentence-label tags and in-card editor rendering.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::session::{SentenceAxis, SentenceLabelSelection, SentenceLabels};
use crate::tui::palette;
use crate::tui::sentence_editor::{LabelEditorRow, SentenceLabelsEditor};
use crate::tui::text_field::TextField;

const MARKER_WIDTH: usize = 2;
const CHEVRON_WIDTH: usize = 2;
const NOTE_PLACEHOLDER: &str = "say what should change";

enum TagSource<'a> {
    Generated(&'a SentenceLabels),
    Editing(&'a SentenceLabelSelection),
}

impl TagSource<'_> {
    fn attributed(&self) -> bool {
        match self {
            Self::Generated(_) => true,
            Self::Editing(selection) => selection.attributed(),
        }
    }

    fn token(&self, axis: SentenceAxis) -> Option<&'static str> {
        match self {
            Self::Generated(labels) => labels.token(axis),
            Self::Editing(selection) => selection.token(axis),
        }
    }

    fn pinned(&self, axis: SentenceAxis) -> bool {
        match self {
            Self::Generated(labels) => labels.pinned().contains(axis),
            Self::Editing(selection) => selection.pinned().contains(axis),
        }
    }

    fn approximate(&self, axis: SentenceAxis) -> bool {
        match self {
            Self::Generated(labels) => labels.approx().contains(axis),
            Self::Editing(selection) => selection.approx().contains(axis),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HeadTag {
    row: usize,
    start: usize,
    end: usize,
    span: Span<'static>,
}

/// Atomic sentence-label tags positioned beside the artifact rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeadTagsLayout {
    tags: Vec<HeadTag>,
}

impl HeadTagsLayout {
    /// Return the width required by the widest indivisible tag.
    #[must_use]
    pub(crate) fn minimum_width(&self) -> usize {
        self.tags
            .iter()
            .map(|tag| tag.end.saturating_sub(tag.start))
            .max()
            .unwrap_or(0)
    }

    /// Return the exclusive column occupied by one rendered tag row.
    #[must_use]
    pub(crate) fn row_width(&self, row: usize) -> usize {
        self.tags
            .iter()
            .filter(|tag| tag.row == row)
            .map(|tag| tag.end)
            .max()
            .unwrap_or(0)
    }

    /// Render one tag row from an existing prefix column.
    #[must_use]
    pub(crate) fn spans_from(
        &self,
        row: usize,
        start: usize,
        gap_style: Style,
    ) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut column = start;
        for tag in self.tags.iter().filter(|tag| tag.row == row) {
            if tag.start > column {
                spans.push(Span::styled(" ".repeat(tag.start - column), gap_style));
            }
            spans.push(tag.span.clone());
            column = tag.end;
        }
        spans
    }

    /// Return whether one cell belongs to a real tag rather than a gap.
    #[must_use]
    pub(crate) fn hit_at(&self, row: usize, column: usize) -> bool {
        self.tags
            .iter()
            .any(|tag| tag.row == row && column >= tag.start && column < tag.end)
    }

    /// Return whether the layout places at least one tag on this row.
    #[must_use]
    pub(crate) fn occupies(&self, row: usize) -> bool {
        self.tags.iter().any(|tag| tag.row == row)
    }
}

/// One clickable control inside the in-card sentence-label editor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EditorControl {
    Chip(LabelEditorRow, usize),
    Advance(LabelEditorRow, bool),
    Note,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChipRegion {
    row: usize,
    start: usize,
    end: usize,
    control: EditorControl,
}

struct EditorLayout {
    lines: Vec<Line<'static>>,
    chips: Vec<ChipRegion>,
    note_row: usize,
    note_start: usize,
    cursor: Option<(usize, usize)>,
}

/// Lay out the three generated or staged tags in one wrapping sequence.
#[must_use]
pub(crate) fn head_tags_layout(
    labels: Option<&SentenceLabels>,
    staged: Option<&SentenceLabelSelection>,
    editor: Option<&SentenceLabelsEditor>,
    width: usize,
) -> Option<HeadTagsLayout> {
    let source = match editor {
        Some(editor) => TagSource::Editing(editor.selection()),
        None => match staged {
            Some(selection) => TagSource::Editing(selection),
            None => TagSource::Generated(labels?),
        },
    };
    if !source.attributed() {
        return None;
    }
    let mut tags = Vec::new();
    let mut row = 0usize;
    let mut used = 0usize;
    for axis in [
        SentenceAxis::Register,
        SentenceAxis::Type,
        SentenceAxis::Level,
    ] {
        let prefix = if source.approximate(axis) { "≈" } else { "" };
        let token = source.token(axis).unwrap_or("—");
        let text = format!(" {prefix}{token} ");
        let tag_width = super::common::display_width(text.as_str());
        let gap = usize::from(used > 0);
        if used > 0 && used.saturating_add(gap).saturating_add(tag_width) > width {
            row = row.saturating_add(1);
            used = 0;
        }
        let start = used.saturating_add(usize::from(used > 0));
        let end = start.saturating_add(tag_width);
        tags.push(HeadTag {
            row,
            start,
            end,
            span: Span::styled(text, tag_style(source.pinned(axis))),
        });
        used = end;
    }
    Some(HeadTagsLayout { tags })
}

pub(crate) fn editor_lines(
    editor: &SentenceLabelsEditor,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> Vec<Line<'static>> {
    editor_layout(editor, width, first_start, fallback_start).lines
}

/// Return the exclusive editor row needed to keep the focused control visible.
#[must_use]
pub(crate) fn editor_focus_end(
    editor: &SentenceLabelsEditor,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> usize {
    let layout = editor_layout(editor, width, first_start, fallback_start);
    if editor.row() == LabelEditorRow::Note {
        return layout.note_row + 1;
    }
    layout
        .chips
        .iter()
        .filter_map(|region| match region.control {
            EditorControl::Chip(row, _) if row == editor.row() => Some(region.row + 1),
            _ => None,
        })
        .max()
        .unwrap_or(1)
}

pub(crate) fn editor_cursor(
    editor: &SentenceLabelsEditor,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> Option<(usize, usize)> {
    editor_layout(editor, width, first_start, fallback_start).cursor
}

pub(crate) fn editor_control_at(
    editor: &SentenceLabelsEditor,
    width: usize,
    column: usize,
    row: usize,
    first_start: usize,
    fallback_start: usize,
) -> Option<EditorControl> {
    let layout = editor_layout(editor, width, first_start, fallback_start);
    if row == layout.note_row && column >= layout.note_start && column < width {
        return Some(EditorControl::Note);
    }
    layout
        .chips
        .into_iter()
        .find(|region| row == region.row && column >= region.start && column < region.end)
        .map(|region| region.control)
}

fn tag_style(pinned: bool) -> Style {
    if pinned {
        palette::invert()
    } else {
        Style::default().bg(palette::DIM).fg(palette::BG)
    }
}

fn editor_layout(
    editor: &SentenceLabelsEditor,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> EditorLayout {
    let wrapped = first_start != fallback_start
        && first_start.saturating_add(widest_selector_width(editor)) > width;
    let indent = if wrapped { fallback_start } else { first_start };
    let mut lines = if wrapped {
        vec![Line::from("")]
    } else {
        Vec::new()
    };
    let mut chips = Vec::new();
    for row in [
        LabelEditorRow::Register,
        LabelEditorRow::Type,
        LabelEditorRow::Level,
    ] {
        let (mut rendered, mut regions) = axis_lines(editor, row, width, lines.len(), indent);
        lines.append(&mut rendered);
        chips.append(&mut regions);
    }
    let note_row = lines.len();
    let note_start = indent + question_column();
    let mut note = row_prefix(
        row_label(LabelEditorRow::Note),
        editor.row() == LabelEditorRow::Note,
        indent,
    );
    note.extend(TextField::new(editor.note().value(), NOTE_PLACEHOLDER).spans());
    lines.push(Line::from(note));
    let cursor = if editor.row() == LabelEditorRow::Note {
        Some((
            note_start + super::common::display_width(editor.note().before_cursor()),
            note_row,
        ))
    } else {
        None
    };
    EditorLayout {
        lines,
        chips,
        note_row,
        note_start,
        cursor,
    }
}

fn widest_selector_width(editor: &SentenceLabelsEditor) -> usize {
    question_column() + selector_width(editor)
}

fn selector_width(editor: &SentenceLabelsEditor) -> usize {
    [
        SentenceAxis::Register,
        SentenceAxis::Type,
        SentenceAxis::Level,
    ]
    .into_iter()
    .map(|axis| {
        let count = editor.selection().choice_count(axis);
        let chip = (0..count)
            .map(|index| {
                editor
                    .selection()
                    .choice_token(axis, index)
                    .expect("invariant: every sentence-label axis must expose its declared choices")
            })
            .map(|token| super::common::display_width(token) + 2)
            .max()
            .expect("invariant: sentence-label axis must expose at least one choice");
        chip.saturating_add(count.saturating_sub(1).saturating_mul(MARKER_WIDTH))
            .saturating_add(2usize.saturating_mul(CHEVRON_WIDTH))
    })
    .max()
    .expect("invariant: sentence-label editor must expose at least one axis")
}

fn axis_lines(
    editor: &SentenceLabelsEditor,
    row: LabelEditorRow,
    width: usize,
    first_row: usize,
    indent: usize,
) -> (Vec<Line<'static>>, Vec<ChipRegion>) {
    let axis = row
        .axis()
        .expect("invariant: every rendered label row must own an axis");
    let count = editor.selection().choice_count(axis);
    let selected = editor.selection().token(axis).and_then(|token| {
        (0..count).find(|index| editor.selection().choice_token(axis, *index) == Some(token))
    });
    let mut lines = Vec::new();
    let mut regions = Vec::new();
    let mut spans = row_prefix(row_label(row), editor.row() == row, indent);
    let mut used = indent + question_column();
    let mut screen_row = first_row;
    let track_width = selector_width(editor);
    let selected_text = selected
        .and_then(|index| editor.selection().choice_token(axis, index))
        .unwrap_or("—");
    let chip_width = super::common::display_width(selected_text) + 2;
    let rail_width = track_width
        .saturating_sub(chip_width)
        .saturating_sub(2usize.saturating_mul(CHEVRON_WIDTH));
    if used.saturating_add(track_width) > width {
        lines.push(Line::from(spans));
        spans = continuation_prefix(indent);
        used = indent;
        screen_row += 1;
    }
    let focused = editor.row() == row;
    let (left, left_region) = chevron(row, screen_row, false, used, focused);
    spans.push(left);
    regions.push(left_region);
    used += CHEVRON_WIDTH;
    if let Some(current) = selected {
        let hidden = count.saturating_sub(1);
        let left_width = proportional_width(rail_width, hidden, current);
        let right_width = rail_width.saturating_sub(left_width);
        for index in 0..current {
            let distance = current - index;
            let width = distributed_marker_width(left_width, current, distance);
            let (span, region) = marker(row, screen_row, index, distance, used, width);
            spans.push(span);
            regions.push(region);
            used += width;
        }
        let token = editor
            .selection()
            .choice_token(axis, current)
            .expect("invariant: selected sentence-label choice must have a token");
        let chip = format!(" {token} ");
        let start = used;
        spans.push(Span::styled(
            chip,
            palette::invert().add_modifier(Modifier::BOLD),
        ));
        used += super::common::display_width(token) + 2;
        regions.push(ChipRegion {
            row: screen_row,
            start,
            end: used,
            control: EditorControl::Chip(row, current),
        });
        let remaining = count.saturating_sub(current + 1);
        for index in current + 1..count {
            let distance = index - current;
            let width = distributed_marker_width(right_width, remaining, distance);
            let (span, region) = marker(row, screen_row, index, distance, used, width);
            spans.push(span);
            regions.push(region);
            used += width;
        }
    } else {
        let left_width = rail_width / 2;
        let right_width = rail_width.saturating_sub(left_width);
        let left_gap = left_width.saturating_sub(MARKER_WIDTH);
        spans.push(empty_marker(left_gap));
        used += left_gap;
        let (span, region) = marker(
            row,
            screen_row,
            count.saturating_sub(1),
            1,
            used,
            MARKER_WIDTH,
        );
        spans.push(span);
        regions.push(region);
        used += MARKER_WIDTH;
        spans.push(Span::styled(" — ", palette::dim2()));
        used += 3;
        let (span, region) = marker(row, screen_row, 0, 1, used, MARKER_WIDTH);
        spans.push(span);
        regions.push(region);
        used += MARKER_WIDTH;
        let right_gap = right_width.saturating_sub(MARKER_WIDTH);
        spans.push(empty_marker(right_gap));
        used += right_gap;
    }
    let (right, right_region) = chevron(row, screen_row, true, used, focused);
    spans.push(right);
    regions.push(right_region);
    lines.push(Line::from(spans));
    (lines, regions)
}

fn proportional_width(total: usize, slots: usize, occupied: usize) -> usize {
    assert!(
        slots > 0 && occupied <= slots,
        "invariant: carousel progress requires a valid selected slot"
    );
    let shared = total / slots;
    let remainder = total % slots;
    shared
        .saturating_mul(occupied)
        .saturating_add(occupied.min(remainder))
}

fn chevron(
    row: LabelEditorRow,
    screen_row: usize,
    forward: bool,
    column: usize,
    focused: bool,
) -> (Span<'static>, ChipRegion) {
    let text = if forward { " >" } else { "< " };
    let style = if focused {
        palette::base()
    } else {
        palette::dim()
    };
    (
        Span::styled(text, style),
        ChipRegion {
            row: screen_row,
            start: column,
            end: column + CHEVRON_WIDTH,
            control: EditorControl::Advance(row, forward),
        },
    )
}

fn marker(
    row: LabelEditorRow,
    screen_row: usize,
    index: usize,
    distance: usize,
    column: usize,
    width: usize,
) -> (Span<'static>, ChipRegion) {
    (
        Span::styled(" ".repeat(width), marker_style(distance)),
        ChipRegion {
            row: screen_row,
            start: column,
            end: column + width,
            control: EditorControl::Chip(row, index),
        },
    )
}

fn distributed_marker_width(total: usize, count: usize, distance: usize) -> usize {
    let shared = total / count;
    let remainder = total % count;
    shared + usize::from(distance <= remainder)
}

fn empty_marker(width: usize) -> Span<'static> {
    Span::styled(" ".repeat(width), marker_style(usize::MAX))
}

fn marker_style(distance: usize) -> Style {
    let background = match distance {
        1 => palette::DIM2,
        2 => palette::RULE,
        _ => palette::HL,
    };
    Style::default().bg(background).fg(palette::BG)
}

fn row_label(row: LabelEditorRow) -> &'static str {
    match row {
        LabelEditorRow::Register => "how should it sound?",
        LabelEditorRow::Type => "what kind of phrase?",
        LabelEditorRow::Level => "what's the desired level?",
        LabelEditorRow::Note => "one more thing",
    }
}

fn question_column() -> usize {
    [
        LabelEditorRow::Register,
        LabelEditorRow::Type,
        LabelEditorRow::Level,
    ]
    .into_iter()
    .map(row_label)
    .map(super::common::display_width)
    .max()
    .expect("invariant: sentence-label editor must expose at least one question")
    .saturating_add(2)
}

fn row_prefix(label: &str, focused: bool, indent: usize) -> Vec<Span<'static>> {
    let style = if focused {
        palette::base().add_modifier(Modifier::BOLD)
    } else {
        palette::dim2()
    };
    vec![
        Span::styled(" ".repeat(indent), palette::base()),
        Span::styled(super::common::pad_right(label, question_column()), style),
    ]
}

fn continuation_prefix(indent: usize) -> Vec<Span<'static>> {
    vec![Span::styled(" ".repeat(indent), palette::base())]
}
