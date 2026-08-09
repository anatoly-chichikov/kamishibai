//! Sentence-label tags and in-card editor rendering.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::session::{SentenceAxis, SentenceBatchSettings, SentenceLabelSelection, SentenceLabels};
use crate::tui::palette;
use crate::tui::sentence_editor::{BatchSettingsRow, LabelEditorRow, SentenceLabelsEditor};
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

/// One clickable control inside the batch sentence-settings editor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BatchEditorControl {
    Chip(BatchSettingsRow, usize),
    Advance(BatchSettingsRow, bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CarouselControl<Row> {
    Chip(Row, usize),
    Advance(Row, bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChipRegion<Row> {
    row: usize,
    start: usize,
    end: usize,
    control: CarouselControl<Row>,
}

struct EditorLayout {
    lines: Vec<Line<'static>>,
    chips: Vec<ChipRegion<LabelEditorRow>>,
    note_row: usize,
    note_start: usize,
    cursor: Option<(usize, usize)>,
}

struct BatchEditorLayout {
    lines: Vec<Line<'static>>,
    chips: Vec<ChipRegion<BatchSettingsRow>>,
    ranges: Vec<(BatchSettingsRow, usize, usize)>,
}

struct CarouselAxis<Row> {
    row: Row,
    label: &'static str,
    focused: bool,
    selected: Option<usize>,
}

struct CarouselGeometry {
    track_width: usize,
    question_width: usize,
    width: usize,
    origin: (usize, usize),
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
            CarouselControl::Chip(row, _) if row == editor.row() => Some(region.row + 1),
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
        .map(|region| match region.control {
            CarouselControl::Chip(row, index) => EditorControl::Chip(row, index),
            CarouselControl::Advance(row, forward) => EditorControl::Advance(row, forward),
        })
}

/// Render both rows of the batch sentence-settings editor.
#[must_use]
pub(crate) fn batch_editor_lines(
    settings: SentenceBatchSettings,
    focused: BatchSettingsRow,
    width: usize,
) -> Vec<Line<'static>> {
    batch_editor_layout(settings, focused, width).lines
}

/// Return the focused batch editor row relative to its first rendered line.
#[must_use]
pub(crate) fn batch_editor_focus_range(
    settings: SentenceBatchSettings,
    focused: BatchSettingsRow,
    width: usize,
) -> (usize, usize) {
    batch_editor_layout(settings, focused, width)
        .ranges
        .into_iter()
        .find_map(|(row, start, end)| (row == focused).then_some((start, end - start)))
        .unwrap_or((0, 1))
}

/// Return the batch sentence-settings control occupying one editor cell.
#[must_use]
pub(crate) fn batch_editor_control_at(
    settings: SentenceBatchSettings,
    focused: BatchSettingsRow,
    width: usize,
    column: usize,
    row: usize,
) -> Option<BatchEditorControl> {
    batch_editor_layout(settings, focused, width)
        .chips
        .into_iter()
        .find(|region| row == region.row && column >= region.start && column < region.end)
        .map(|region| match region.control {
            CarouselControl::Chip(row, index) => BatchEditorControl::Chip(row, index),
            CarouselControl::Advance(row, forward) => BatchEditorControl::Advance(row, forward),
        })
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
        question_column(),
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
        row_selector_width(
            count,
            (0..count).map(|index| {
                editor
                    .selection()
                    .choice_token(axis, index)
                    .expect("invariant: every sentence-label axis must expose its declared choices")
            }),
        )
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
) -> (Vec<Line<'static>>, Vec<ChipRegion<LabelEditorRow>>) {
    let axis = row
        .axis()
        .expect("invariant: every rendered label row must own an axis");
    let count = editor.selection().choice_count(axis);
    let selected = editor.selection().token(axis).and_then(|token| {
        (0..count).find(|index| editor.selection().choice_token(axis, *index) == Some(token))
    });
    carousel_lines(
        CarouselAxis {
            row,
            label: row_label(row),
            focused: editor.row() == row,
            selected,
        },
        count,
        |index| editor.selection().choice_token(axis, index),
        CarouselGeometry {
            track_width: selector_width(editor),
            question_width: question_column(),
            width,
            origin: (first_row, indent),
        },
    )
}

fn carousel_lines<Row: Copy>(
    axis: CarouselAxis<Row>,
    count: usize,
    choice_token: impl Fn(usize) -> Option<&'static str>,
    geometry: CarouselGeometry,
) -> (Vec<Line<'static>>, Vec<ChipRegion<Row>>) {
    let CarouselAxis {
        row,
        label,
        focused,
        selected,
    } = axis;
    let CarouselGeometry {
        track_width,
        question_width,
        width,
        origin: (first_row, indent),
    } = geometry;
    let mut lines = Vec::new();
    let mut regions = Vec::new();
    let mut spans = row_prefix(label, focused, indent, question_width);
    let mut used = indent + question_width;
    let mut screen_row = first_row;
    let selected_text = selected.and_then(&choice_token).unwrap_or("—");
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
        let token = choice_token(current)
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
            control: CarouselControl::Chip(row, current),
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

fn row_selector_width(count: usize, tokens: impl Iterator<Item = &'static str>) -> usize {
    let chip = tokens
        .map(|token| super::common::display_width(token) + 2)
        .max()
        .expect("invariant: sentence carousel must expose at least one choice");
    chip.saturating_add(count.saturating_sub(1).saturating_mul(MARKER_WIDTH))
        .saturating_add(2usize.saturating_mul(CHEVRON_WIDTH))
}

fn batch_editor_layout(
    settings: SentenceBatchSettings,
    focused: BatchSettingsRow,
    width: usize,
) -> BatchEditorLayout {
    let mut lines = Vec::new();
    let mut chips = Vec::new();
    let mut ranges = Vec::new();
    for row in [BatchSettingsRow::Level, BatchSettingsRow::Types] {
        let start = lines.len();
        let count = row.choice_count();
        let (mut rendered, mut regions) = carousel_lines(
            CarouselAxis {
                row,
                label: batch_row_label(row),
                focused: row == focused,
                selected: Some(row.selected(settings)),
            },
            count,
            |index| row.choice_token(index),
            CarouselGeometry {
                track_width: batch_selector_width(),
                question_width: batch_question_column(),
                width,
                origin: (start, 0),
            },
        );
        lines.append(&mut rendered);
        chips.append(&mut regions);
        ranges.push((row, start, lines.len()));
    }
    BatchEditorLayout {
        lines,
        chips,
        ranges,
    }
}

fn batch_selector_width() -> usize {
    [BatchSettingsRow::Level, BatchSettingsRow::Types]
        .into_iter()
        .map(|row| {
            let count = row.choice_count();
            row_selector_width(
                count,
                (0..count).map(|index| {
                    row.choice_token(index)
                        .expect("invariant: every batch sentence row must expose its choices")
                }),
            )
        })
        .max()
        .expect("invariant: batch sentence editor must expose at least one row")
}

fn batch_question_column() -> usize {
    [BatchSettingsRow::Level, BatchSettingsRow::Types]
        .into_iter()
        .map(batch_row_label)
        .map(super::common::display_width)
        .max()
        .expect("invariant: batch sentence editor must expose at least one question")
        .saturating_add(2)
}

fn batch_row_label(row: BatchSettingsRow) -> &'static str {
    match row {
        BatchSettingsRow::Level => "what's the desired level?",
        BatchSettingsRow::Types => "how to mix the types?",
    }
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

fn chevron<Row: Copy>(
    row: Row,
    screen_row: usize,
    forward: bool,
    column: usize,
    focused: bool,
) -> (Span<'static>, ChipRegion<Row>) {
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
            control: CarouselControl::Advance(row, forward),
        },
    )
}

fn marker<Row: Copy>(
    row: Row,
    screen_row: usize,
    index: usize,
    distance: usize,
    column: usize,
    width: usize,
) -> (Span<'static>, ChipRegion<Row>) {
    (
        Span::styled(" ".repeat(width), marker_style(distance)),
        ChipRegion {
            row: screen_row,
            start: column,
            end: column + width,
            control: CarouselControl::Chip(row, index),
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

fn row_prefix(
    label: &str,
    focused: bool,
    indent: usize,
    question_width: usize,
) -> Vec<Span<'static>> {
    let style = if focused {
        palette::base().add_modifier(Modifier::BOLD)
    } else {
        palette::dim2()
    };
    vec![
        Span::styled(" ".repeat(indent), palette::base()),
        Span::styled(super::common::pad_right(label, question_width), style),
    ]
}

fn continuation_prefix(indent: usize) -> Vec<Span<'static>> {
    vec![Span::styled(" ".repeat(indent), palette::base())]
}
