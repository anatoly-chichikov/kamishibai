//! Sentence-label tags and in-card editor rendering.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::session::{SentenceAxis, SentenceBatchSettings, SentenceLabelSelection, SentenceLabels};
use crate::tui::palette;
use crate::tui::sentence_editor::{BatchSettingsRow, LabelEditorRow, SentenceLabelsEditor};
use crate::tui::text_field::TextField;

const MARKER_WIDTH: usize = 2;
const CHEVRON_WIDTH: usize = 2;
const NOTE_PLACEHOLDER: &str = "say what should change";

struct TagSource<'a> {
    generated: Option<&'a SentenceLabels>,
    working: Option<&'a SentenceLabelSelection>,
}

impl TagSource<'_> {
    fn attributed(&self) -> bool {
        self.generated.is_some() || self.working.is_some_and(SentenceLabelSelection::attributed)
    }

    fn actual(&self, axis: SentenceAxis) -> Option<&'static str> {
        let labels = self.generated?;
        if labels.approx().contains(axis) && labels.recorded_request_token(axis).is_none() {
            return None;
        }
        labels.token(axis)
    }

    fn requested(&self, axis: SentenceAxis) -> Option<&'static str> {
        if !self.pinned(axis) {
            return None;
        }
        self.working
            .and_then(|selection| selection.token(axis))
            .or_else(|| {
                self.generated
                    .and_then(|labels| labels.requested_token(axis))
            })
    }

    fn pinned(&self, axis: SentenceAxis) -> bool {
        self.working
            .map(|selection| selection.pinned().contains(axis))
            .unwrap_or_else(|| {
                self.generated
                    .is_some_and(|labels| labels.pinned().contains(axis))
            })
    }

    fn fallback(&self, axis: SentenceAxis) -> Option<&'static str> {
        self.actual(axis)
            .or_else(|| self.working.and_then(|selection| selection.token(axis)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HeadTag {
    row: usize,
    start: usize,
    end: usize,
    spans: Vec<Span<'static>>,
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
            spans.extend(tag.spans.clone());
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

/// One clickable control inside the generation-guidance editor.
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
    row_ends: Vec<(LabelEditorRow, usize)>,
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
    let source = TagSource {
        generated: labels,
        working: editor.map(SentenceLabelsEditor::selection).or(staged),
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
        let (spans, tag_width) = tag_spans(&source, axis);
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
            spans,
        });
        used = end;
    }
    Some(HeadTagsLayout { tags })
}

/// Render the four tune rows of one card. `focused` is what separates the live
/// editor from the same rows merely on display under an open card: unfocused,
/// no question is lit, no chevron is bright, and the note owns no cursor.
pub(crate) fn editor_lines(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    focused: bool,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> Vec<Line<'static>> {
    editor_layout(editor, current, focused, width, first_start, fallback_start).lines
}

/// Return the exclusive editor row needed to keep the focused control visible.
#[must_use]
pub(crate) fn editor_focus_end(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> usize {
    let layout = editor_layout(editor, current, true, width, first_start, fallback_start);
    if editor.row() == LabelEditorRow::Note {
        return layout.note_row + 1;
    }
    layout
        .row_ends
        .iter()
        .find_map(|(row, end)| (*row == editor.row()).then_some(*end))
        .unwrap_or(1)
}

pub(crate) fn editor_cursor(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    width: usize,
    first_start: usize,
    fallback_start: usize,
) -> Option<(usize, usize)> {
    editor_layout(editor, current, true, width, first_start, fallback_start).cursor
}

pub(crate) fn editor_control_at(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    width: usize,
    column: usize,
    row: usize,
    first_start: usize,
    fallback_start: usize,
) -> Option<EditorControl> {
    let layout = editor_layout(editor, current, true, width, first_start, fallback_start);
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

/// Render both rows of the generation-guidance editor.
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

/// Return the shared compact-label style used before and after generation.
pub(crate) fn tag_style(pinned: bool) -> Style {
    if pinned {
        palette::invert()
    } else {
        Style::default().bg(palette::DIM).fg(palette::BG)
    }
}

fn tag_spans(source: &TagSource<'_>, axis: SentenceAxis) -> (Vec<Span<'static>>, usize) {
    let actual = source.actual(axis);
    let requested = source.requested(axis);
    let spans = match (actual, requested) {
        (Some(actual), Some(requested)) if actual != requested => vec![
            label_tag(actual, false),
            Span::styled(" · aimed for ", palette::Ink::Aside.on(false)),
            label_tag(requested, true),
        ],
        (None, Some(requested)) => vec![
            Span::styled("aimed for ", palette::Ink::Aside.on(false)),
            label_tag(requested, true),
        ],
        (Some(actual), _) => vec![label_tag(actual, source.pinned(axis))],
        (None, None) => vec![label_tag(source.fallback(axis).unwrap_or("—"), false)],
    };
    let width = spans
        .iter()
        .map(|span| super::common::display_width(span.content.as_ref()))
        .sum();
    (spans, width)
}

fn label_tag(token: &str, pinned: bool) -> Span<'static> {
    Span::styled(format!(" {token} "), tag_style(pinned))
}

fn editor_layout(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    focused: bool,
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
    let mut row_ends = Vec::new();
    for row in [
        LabelEditorRow::Register,
        LabelEditorRow::Type,
        LabelEditorRow::Level,
    ] {
        let (mut rendered, mut regions) =
            axis_lines(editor, current, focused, row, width, lines.len(), indent);
        lines.append(&mut rendered);
        chips.append(&mut regions);
        row_ends.push((row, lines.len()));
    }
    let note_row = lines.len();
    let note_start = indent + question_column();
    let mut note = row_prefix(
        row_label(LabelEditorRow::Note),
        focused && editor.row() == LabelEditorRow::Note,
        indent,
        question_column(),
    );
    note.extend(TextField::new(editor.note().value(), NOTE_PLACEHOLDER).spans());
    lines.push(Line::from(note));
    let cursor = if focused && editor.row() == LabelEditorRow::Note {
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
        row_ends,
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

#[allow(clippy::too_many_arguments)]
fn axis_lines(
    editor: &SentenceLabelsEditor,
    current: Option<&SentenceLabels>,
    focused: bool,
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
    let (mut lines, regions) = carousel_lines(
        CarouselAxis {
            row,
            label: row_label(row),
            focused: focused && editor.row() == row,
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
    );
    append_current(&mut lines, current, editor.selection(), axis, width, indent);
    (lines, regions)
}

fn append_current(
    lines: &mut Vec<Line<'static>>,
    current: Option<&SentenceLabels>,
    selection: &SentenceLabelSelection,
    axis: SentenceAxis,
    width: usize,
    indent: usize,
) {
    let Some(actual) = current.and_then(|labels| known_current(labels, axis)) else {
        return;
    };
    if selection.token(axis) == Some(actual) {
        return;
    }
    let label = "current  ";
    let same_line_gap = "   ";
    let status_width = super::common::display_width(same_line_gap)
        .saturating_add(super::common::display_width(label))
        .saturating_add(super::common::display_width(actual));
    let line_width = lines.last().map(Line::width).unwrap_or(0);
    if line_width.saturating_add(status_width) <= width {
        let line = lines
            .last_mut()
            .expect("invariant: every sentence-label axis must render a carousel line");
        line.spans
            .push(Span::styled(same_line_gap, palette::base()));
        line.spans
            .push(Span::styled(label, palette::Ink::Aside.on(false)));
        line.spans.push(Span::styled(actual, palette::base()));
        return;
    }
    let start = if lines.len() > 1 {
        indent
    } else {
        indent.saturating_add(question_column())
    };
    lines.push(Line::from(vec![
        Span::styled(" ".repeat(start), palette::base()),
        Span::styled(label, palette::Ink::Aside.on(false)),
        Span::styled(actual, palette::base()),
    ]));
}

fn known_current(labels: &SentenceLabels, axis: SentenceAxis) -> Option<&'static str> {
    if labels.approx().contains(axis) && labels.recorded_request_token(axis).is_none() {
        return None;
    }
    labels.token(axis)
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
        spans.push(Span::styled(chip, palette::invert()));
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
        spans.push(Span::styled(" — ", palette::Ink::Aside.on(false)));
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
        BatchSettingsRow::Types => "what kinds of phrases?",
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
        palette::Ink::Detail.on(false)
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

/// Paint one rail segment by how far its choice sits from the selected chip.
///
/// The rail fades out rather than stopping: the nearest hidden choice is the
/// brightest segment, the next one is a rule line, and everything beyond it is
/// the page itself. The far segment is deliberately the background and not the
/// cursor highlight — the rail must not brighten when the row highlight does.
fn marker_style(distance: usize) -> Style {
    let background = match distance {
        1 => palette::DIM2,
        2 => palette::RULE,
        _ => palette::BG,
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
        palette::Ink::Subject.on(false)
    } else {
        palette::Ink::Aside.on(false)
    };
    vec![
        Span::styled(" ".repeat(indent), palette::base()),
        Span::styled(super::common::pad_right(label, question_width), style),
    ]
}

fn continuation_prefix(indent: usize) -> Vec<Span<'static>> {
    vec![Span::styled(" ".repeat(indent), palette::base())]
}
