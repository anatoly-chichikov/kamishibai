//! Renderer for the `your cards` / `building your cards` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-2.jsx` (StepGenerating). One block
//! per card: head row plus four step lines (meta · scene · audio · picture).
//! "meta" is the rich card metadata produced by the Gemini card pass and is the first
//! real step in the pipeline. When a card is selected and finished the row
//! expands into a meta preview + artifact pane.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::markdown::{parse_markdown, to_ratatui};
use crate::session::{
    Artifact, ArtifactFile, ArtifactSlot, AttemptFault, CardArtifacts, CardDraft, CardMeta,
    GenerationCost,
};
use crate::tui::app::App;
use crate::tui::disclosure::DisclosureControls;
use crate::tui::palette;
use crate::tui::sentence_editor::SentenceLabelsEditor;

const HEADLINE_WORKING: &str = "building your cards";
const HEADLINE_DONE: &str = "your cards";
const HINT_WORKING: &str = "drawing each card one by one";
const HINT_DONE: &str = "all done";
const HINT_DONE_FAILED: &str = "some cards didn't make it";
const SPINNER_FRAME_MILLIS: u128 = 250;
const STEP_LABEL_COL_CHARS: usize = 14;
const STEP_DETAIL_COL_CHARS: usize = 9;
const STEP_AUX_COL_CHARS: usize = 8;
const SENTENCE_TAG_GAP: usize = 3;
const SENTENCE_TAG_START: usize = super::common::CARD_DETAIL_COLUMN
    + STEP_LABEL_COL_CHARS
    + STEP_DETAIL_COL_CHARS
    + STEP_AUX_COL_CHARS
    + SENTENCE_TAG_GAP;
const SECTION_GAP_ROWS: usize = 1;
const FACT_LABEL_COLUMN: usize = 29;
const STEPS: [(&str, Artifact); 4] = [
    ("meta", Artifact::Meta),
    ("audio", Artifact::Sound),
    ("scene", Artifact::Scene),
    ("picture", Artifact::Picture),
];
const SPINNER_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

struct CollapsedLabels {
    tags: super::sentence_labels::HeadTagsLayout,
    start: usize,
}

struct ArtifactLine<'a> {
    core: Vec<Span<'a>>,
    tail: Vec<Span<'a>>,
}

impl<'a> ArtifactLine<'a> {
    fn core_width(&self) -> usize {
        spans_width(self.core.as_slice())
    }

    fn tail_width(&self) -> usize {
        spans_width(self.tail.as_slice())
    }

    fn into_line(mut self) -> Line<'a> {
        self.core.append(&mut self.tail);
        Line::from(self.core)
    }

    fn muted(mut self) -> Self {
        for span in self.core.iter_mut().chain(self.tail.iter_mut()) {
            span.style = span.style.fg(palette::DIM);
        }
        self
    }
}

struct StepState<'a> {
    glyph: String,
    status_style: Style,
    label_style: Style,
    line: ArtifactLine<'a>,
}

/// `ScreenView` handle for the `your cards` / generating screen. Title and
/// hint switch from the `building` copy to the `done` copy once every card
/// has either succeeded or terminally failed.
pub struct YourCards;

impl ScreenView for YourCards {
    fn title(&self, app: &App) -> Cow<'static, str> {
        let copy = if all_finished(app) {
            HEADLINE_DONE
        } else {
            HEADLINE_WORKING
        };
        Cow::Borrowed(copy)
    }

    fn hint(&self, app: &App) -> Cow<'static, str> {
        let copy = if !all_finished(app) {
            HINT_WORKING
        } else if app.cards_failed() > 0 {
            HINT_DONE_FAILED
        } else {
            HINT_DONE
        };
        Cow::Borrowed(copy)
    }

    fn footer(&self, app: &App, width: u16) -> Paragraph<'static> {
        footer(app, width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        let finished = all_finished(app);
        let banner_rows = if finished {
            super::banner::height(app)
        } else {
            0
        };
        let cards_area = if banner_rows == 0 {
            area
        } else {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(banner_rows), Constraint::Min(0)])
                .split(area);
            frame.render_widget(super::banner::widget(app), split[0]);
            split[1]
        };
        frame.render_widget(
            cards_paragraph(app, area.width as usize).scroll((app.body_scroll(), 0)),
            cards_area,
        );
        paint_sentence_editor_cursor(frame, cards_area, app);
    }
}

fn cards_paragraph(app: &App, width: usize) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    if app.cards().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("preparing cards…", palette::dim())));
        return Paragraph::new(lines).style(palette::base());
    }
    let spinner_frame =
        (app.elapsed().as_millis() / SPINNER_FRAME_MILLIS) as usize % SPINNER_FRAMES.len();
    let running_target = app.cards_running_target();
    for (index, draft) in app.cards().iter().enumerate() {
        let focused = index == app.card_selected();
        let expanded = focused && app.card_expanded();
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == index { Some(kind) } else { None });
        lines.extend(card_block(
            draft,
            index,
            focused,
            expanded,
            if focused { app.sentence_editor() } else { None },
            width,
            running_for_card,
            spinner_frame,
        ));
    }
    Paragraph::new(lines).style(palette::base())
}

#[allow(clippy::too_many_arguments)]
fn card_block<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    editor: Option<&SentenceLabelsEditor>,
    width: usize,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Vec<Line<'a>> {
    let artifacts = draft.artifacts();
    let steps = step_rows_for(draft, running);
    let progressed = !steps.is_empty();
    let pending = draft.staged_rewrite().is_some();
    let mut lines: Vec<Line<'a>> = Vec::new();
    lines.extend(card_head(
        draft, idx, focused, expanded, progressed, pending, width,
    ));
    let step_lines = steps
        .iter()
        .map(|kind| {
            let slot = slot_for(artifacts, *kind);
            let line = artifact_line(
                *kind,
                slot,
                running,
                spinner_frame,
                *kind == Artifact::Meta && draft.meta().is_some(),
            );
            (*kind, if pending { line.muted() } else { line })
        })
        .collect::<Vec<_>>();
    if expanded {
        lines.extend(step_lines.into_iter().map(|(_, line)| line.into_line()));
        if let Some(editor) = editor {
            lines.push(Line::from(""));
            lines.extend(super::sentence_labels::editor_lines(
                editor,
                width,
                super::common::CARD_DETAIL_COLUMN,
                super::common::CARD_DETAIL_COLUMN,
            ));
        }
    } else if let Some(labels) = collapsed_labels(draft, running, width) {
        lines.extend(collapsed_step_lines(step_lines, labels));
    } else {
        lines.extend(step_lines.into_iter().map(|(_, line)| line.into_line()));
    }
    if expanded {
        lines.extend(detail_pane(draft, width, pending).lines);
    }
    if progressed || expanded {
        lines.push(Line::from(""));
    }
    lines
}

fn collapsed_labels(
    draft: &CardDraft,
    running: Option<Artifact>,
    width: usize,
) -> Option<CollapsedLabels> {
    let steps = step_rows_for(draft, running);
    if !steps.contains(&Artifact::Sound) {
        return None;
    }
    let sound = artifact_line(
        Artifact::Sound,
        draft.artifacts().sound(),
        running,
        0,
        false,
    );
    let start = SENTENCE_TAG_START;
    if sound.core_width() > start {
        return None;
    }
    let unwrapped = summary_tags_layout(draft, usize::MAX)?;
    if sound.tail_width() > 0 {
        if start
            .saturating_add(unwrapped.row_width(0))
            .saturating_add(sound.tail_width())
            > width
        {
            return None;
        }
        return Some(CollapsedLabels {
            tags: unwrapped,
            start,
        });
    }
    let content_width = width.saturating_sub(start);
    if unwrapped.minimum_width() > content_width {
        return None;
    }
    let tags = summary_tags_layout(draft, content_width)?;
    if [Artifact::Sound, Artifact::Scene, Artifact::Picture]
        .into_iter()
        .enumerate()
        .any(|(row, artifact)| {
            tags.occupies(row)
                && (!steps.contains(&artifact)
                    || artifact_line_width(draft, running, artifact) > start)
        })
    {
        return None;
    }
    Some(CollapsedLabels { tags, start })
}

fn collapsed_step_lines<'a>(
    steps: Vec<(Artifact, ArtifactLine<'a>)>,
    labels: CollapsedLabels,
) -> Vec<Line<'a>> {
    steps
        .into_iter()
        .map(|(artifact, line)| {
            let Some(row) = sentence_tag_row(artifact) else {
                return line.into_line();
            };
            if !labels.tags.occupies(row) {
                return line.into_line();
            }
            if artifact == Artifact::Sound {
                let mut spans = line.core;
                spans.push(Span::styled(
                    " ".repeat(labels.start.saturating_sub(spans_width(spans.as_slice()))),
                    palette::base(),
                ));
                spans.extend(labels.tags.spans_from(row, 0, palette::base()));
                spans.extend(line.tail);
                return Line::from(spans);
            }
            let mut full = line.into_line();
            full.spans.push(Span::styled(
                " ".repeat(labels.start.saturating_sub(full.width())),
                palette::base(),
            ));
            full.spans
                .extend(labels.tags.spans_from(row, 0, palette::base()));
            full
        })
        .collect()
}

fn paint_sentence_editor_cursor(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some((column, row)) = sentence_editor_cursor_for(app, usize::from(area.width)) else {
        return;
    };
    let scroll_row = usize::from(app.body_scroll());
    if row < scroll_row {
        return;
    }
    let visible_row = row - scroll_row;
    if visible_row >= usize::from(area.height) {
        return;
    }
    let column = column.min(usize::from(area.width.saturating_sub(1)));
    let x = area
        .x
        .saturating_add(u16::try_from(column).unwrap_or(u16::MAX));
    let y = area
        .y
        .saturating_add(u16::try_from(visible_row).unwrap_or(u16::MAX));
    frame.set_cursor_position((x, y));
}

const HEAD_PREFIX_CHARS: usize = 7;
const HEAD_ARROW: &str = " → ";
const HEAD_ARROW_CHARS: usize = 3;

fn card_head<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    progressed: bool,
    pending: bool,
    width: usize,
) -> Vec<Line<'a>> {
    let row_style = if focused {
        palette::highlight()
    } else {
        palette::base()
    };
    let glyph = if expanded {
        "▾"
    } else if card_finished(draft) {
        "▸"
    } else if progressed {
        "·"
    } else {
        " "
    };
    let glyph_style = if focused && !pending {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else if focused {
        palette::highlight_dim()
    } else {
        palette::dim2()
    };
    let num_style = if focused && !pending {
        palette::highlight()
    } else if focused {
        palette::highlight_dim()
    } else {
        palette::dim2()
    };
    let term_style = if pending {
        if focused {
            palette::highlight_dim()
        } else {
            palette::dim()
        }
    } else {
        match (progressed, focused) {
            (true, true) => palette::highlight().add_modifier(Modifier::BOLD),
            (true, false) => palette::base(),
            (false, true) => palette::highlight_dim(),
            (false, false) => palette::dim2(),
        }
    };
    let sentence_base = if focused {
        palette::highlight_dim()
    } else {
        palette::dim()
    };
    let sentence_style = if pending {
        sentence_base.add_modifier(Modifier::CROSSED_OUT)
    } else {
        sentence_base
    };
    let term_width = super::common::display_width(draft.term());
    let head_used = HEAD_PREFIX_CHARS + term_width;
    let mut head_spans: Vec<Span<'a>> = Vec::new();
    head_spans.push(Span::styled(format!(" {glyph} "), glyph_style));
    head_spans.push(Span::styled(format!("{:0>2}  ", idx + 1), num_style));
    head_spans.push(Span::styled(String::from(draft.term()), term_style));
    let suffix_style = if focused {
        palette::highlight_dim()
    } else {
        palette::dim2()
    };
    let Some(meta) = draft.meta() else {
        let suffix = visible_card_head_suffix(draft, head_used, width);
        let suffix_width = suffix
            .as_ref()
            .map(|label| super::common::display_width(label))
            .unwrap_or(0);
        if let Some(label) = suffix {
            head_spans.push(Span::styled(label, suffix_style));
        }
        let pad = width.saturating_sub(head_used.saturating_add(suffix_width));
        if pad > 0 {
            head_spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        return vec![Line::from(head_spans)];
    };
    let sentence_start = head_used + HEAD_ARROW_CHARS;
    let suffix = visible_card_head_suffix(draft, sentence_start, width);
    let suffix_width = suffix
        .as_ref()
        .map(|label| super::common::display_width(label))
        .unwrap_or(0);
    let chunks = sentence_chunks(meta, sentence_start, suffix_width, width);
    let mut chunks = chunks.into_iter();
    let first = chunks.next().unwrap_or_default();
    let first_width = super::common::display_width(first.as_str());
    head_spans.push(Span::styled(HEAD_ARROW, sentence_base));
    head_spans.push(Span::styled(first, sentence_style));
    let mut first_used = sentence_start + first_width;
    if let Some(label) = suffix.as_ref() {
        head_spans.push(Span::styled(label.clone(), suffix_style));
        first_used += suffix_width;
    }
    let pad = width.saturating_sub(first_used);
    if pad > 0 {
        head_spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    let mut lines: Vec<Line<'a>> = vec![Line::from(head_spans)];
    for chunk in chunks {
        let chunk_width = super::common::display_width(chunk.as_str());
        let mut spans: Vec<Span<'a>> = vec![Span::styled(" ".repeat(sentence_start), row_style)];
        spans.push(Span::styled(chunk, sentence_style));
        let pad = width.saturating_sub(sentence_start + chunk_width);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn wrap_sentence(sentence: &str, first_avail: usize, cont_avail: usize) -> Vec<String> {
    super::common::wrap_words(sentence, first_avail, cont_avail)
}

fn summary_tags_layout(
    draft: &CardDraft,
    width: usize,
) -> Option<super::sentence_labels::HeadTagsLayout> {
    let labels = draft.meta().and_then(CardMeta::sentence_labels);
    let staged = draft.staged_rewrite().map(|rewrite| rewrite.selection());
    super::sentence_labels::head_tags_layout(labels, staged, None, width)
}

fn sentence_tag_row(artifact: Artifact) -> Option<usize> {
    match artifact {
        Artifact::Meta => None,
        Artifact::Sound => Some(0),
        Artifact::Scene => Some(1),
        Artifact::Picture => Some(2),
    }
}

fn artifact_line_width(draft: &CardDraft, running: Option<Artifact>, kind: Artifact) -> usize {
    let artifacts = draft.artifacts();
    artifact_line(
        kind,
        slot_for(artifacts, kind),
        running,
        0,
        kind == Artifact::Meta && draft.meta().is_some(),
    )
    .into_line()
    .width()
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| super::common::display_width(span.content.as_ref()))
        .sum()
}

fn sentence_chunks(
    meta: &CardMeta,
    sentence_start: usize,
    cost_width: usize,
    width: usize,
) -> Vec<String> {
    let first = width.saturating_sub(sentence_start + cost_width);
    let continuation = width.saturating_sub(sentence_start);
    wrap_sentence(meta.target_sentence(), first, continuation)
}

fn head_rows(draft: &CardDraft, width: usize) -> usize {
    let Some(meta) = draft.meta() else {
        return 1;
    };
    let sentence_start =
        HEAD_PREFIX_CHARS + super::common::display_width(draft.term()) + HEAD_ARROW_CHARS;
    let suffix_width = visible_card_head_suffix(draft, sentence_start, width)
        .as_ref()
        .map(|label| super::common::display_width(label))
        .unwrap_or(0);
    sentence_chunks(meta, sentence_start, suffix_width, width)
        .len()
        .max(1)
}

fn visible_card_head_suffix(draft: &CardDraft, row_used: usize, width: usize) -> Option<String> {
    let cost = card_cost(draft).map(|cost| cost.dollars());
    let retries = card_retry_count(draft);
    let label = match (cost, retries) {
        (Some(cost), 0) => format!("  {cost}"),
        (Some(cost), retries) => format!("  {cost}  ↻{retries}"),
        (None, retries) if retries > 0 => format!("  ↻{retries}"),
        (None, _) => return None,
    };
    let label_width = super::common::display_width(label.as_str());
    let sentence_breathing = if draft.meta().is_some() { 8 } else { 0 };
    if width.saturating_sub(row_used) < label_width.saturating_add(sentence_breathing) {
        return None;
    }
    Some(label)
}

fn card_retry_count(draft: &CardDraft) -> u16 {
    let artifacts = draft.artifacts();
    [
        artifacts.meta(),
        artifacts.sound(),
        artifacts.scene(),
        artifacts.picture(),
    ]
    .into_iter()
    .map(|slot| u16::from(slot.tally().done().min(slot.tally().retries())))
    .sum()
}

fn card_finished(draft: &CardDraft) -> bool {
    let artifacts = draft.artifacts();
    artifacts.all_ready() || artifacts.has_failed()
}

fn slot_for(artifacts: &CardArtifacts, kind: Artifact) -> &ArtifactSlot {
    match kind {
        Artifact::Meta => artifacts.meta(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    }
}

fn slot_visible(slot: &ArtifactSlot, kind: Artifact, running: Option<Artifact>) -> bool {
    slot.ready()
        || slot.discarded()
        || slot.failed_terminally()
        || slot.tally().done() > 0
        || running == Some(kind)
}

fn artifact_line<'a>(
    kind: Artifact,
    slot: &'a ArtifactSlot,
    running: Option<Artifact>,
    spinner_frame: usize,
    stored: bool,
) -> ArtifactLine<'a> {
    let active = running == Some(kind);
    let label = step_label(kind, slot);
    let state = step_state(slot, active, spinner_frame, stored);
    let mut core: Vec<Span<'a>> = Vec::new();
    core.push(Span::styled(
        " ".repeat(super::common::CARD_DETAIL_COLUMN.saturating_sub(2)),
        palette::base(),
    ));
    core.push(Span::styled(
        format!("{} ", state.glyph),
        state.status_style,
    ));
    core.push(Span::styled(label.clone(), state.label_style));
    core.push(Span::styled(" ".repeat(label_gap(&label)), palette::dim()));
    core.extend(state.line.core);
    ArtifactLine {
        core,
        tail: state.line.tail,
    }
}

fn muted_line(mut line: Line<'_>) -> Line<'_> {
    for span in &mut line.spans {
        span.style = span.style.fg(palette::DIM);
    }
    line
}

fn step_label(kind: Artifact, slot: &ArtifactSlot) -> String {
    match slot.file() {
        Some(file) => artifact_file_label(kind, file),
        None => String::from(step_name(kind)),
    }
}

fn step_state<'a>(
    slot: &'a ArtifactSlot,
    active: bool,
    spinner_frame: usize,
    stored: bool,
) -> StepState<'a> {
    let row_dim = palette::dim();
    let row_dim2 = palette::dim2();
    let row_fg = palette::base();
    if slot.ready() || stored {
        let mut core: Vec<Span<'a>> = Vec::new();
        let mut tail: Vec<Span<'a>> = Vec::new();
        if let Some(file) = slot.file() {
            core.push(Span::styled(
                pad_left(file.size(), STEP_DETAIL_COL_CHARS),
                palette::dim(),
            ));
        }
        push_slot_cost(&mut core, slot);
        if let Some(file) = slot.file()
            && file.cached()
        {
            if slot.cost().is_some() {
                push_cached(&mut tail);
            } else {
                push_cached(&mut core);
            }
        }
        return StepState {
            glyph: String::from("✓"),
            status_style: row_fg,
            label_style: palette::link(),
            line: ArtifactLine { core, tail },
        };
    }
    if slot.discarded() {
        return StepState {
            glyph: String::from("⊘"),
            status_style: row_dim,
            label_style: row_dim,
            line: ArtifactLine {
                core: vec![Span::styled(String::from("discarded"), palette::dim())],
                tail: Vec::new(),
            },
        };
    }
    if slot.failed_terminally() {
        let mut core = vec![Span::styled(String::from("gave up"), palette::dim())];
        push_slot_cost(&mut core, slot);
        return StepState {
            glyph: String::from("✗"),
            status_style: row_fg,
            label_style: row_fg,
            line: ArtifactLine {
                core,
                tail: Vec::new(),
            },
        };
    }
    if active {
        return StepState {
            glyph: String::from(SPINNER_FRAMES[spinner_frame]),
            status_style: row_fg,
            label_style: row_fg,
            line: ArtifactLine {
                core: vec![Span::styled(String::from("ai is working…"), palette::dim())],
                tail: Vec::new(),
            },
        };
    }
    if slot.tally().retry().is_some() {
        let mut core = Vec::new();
        push_slot_cost(&mut core, slot);
        return StepState {
            glyph: String::from("·"),
            status_style: row_fg,
            label_style: row_fg,
            line: ArtifactLine {
                core,
                tail: Vec::new(),
            },
        };
    }
    StepState {
        glyph: String::from("○"),
        status_style: row_dim2,
        label_style: row_dim2,
        line: ArtifactLine {
            core: vec![Span::styled(String::from("queued"), palette::dim())],
            tail: Vec::new(),
        },
    }
}

fn push_cached(note: &mut Vec<Span<'_>>) {
    note.push(Span::styled("  ", palette::dim()));
    note.push(Span::styled("cached", palette::dim2()));
}

fn push_slot_cost<'a>(note: &mut Vec<Span<'a>>, slot: &ArtifactSlot) {
    if let Some(cost) = slot.cost() {
        note.push(Span::styled("  ", palette::dim()));
        note.push(Span::styled(cost.dollars(), palette::dim2()));
    }
}

fn step_name(kind: Artifact) -> &'static str {
    match kind {
        Artifact::Meta => "meta",
        Artifact::Sound => "audio",
        Artifact::Scene => "scene",
        Artifact::Picture => "picture",
    }
}

pub(crate) fn artifact_file_label(kind: Artifact, file: &ArtifactFile) -> String {
    match file_extension(file.name()) {
        Some(extension) => format!("{}.{}", step_name(kind), extension),
        None => String::from(step_name(kind)),
    }
}

fn file_extension(filename: &str) -> Option<&str> {
    filename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
}

fn label_gap(label: &str) -> usize {
    STEP_LABEL_COL_CHARS
        .saturating_sub(label.chars().count())
        .max(2)
}

fn pad_left(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return String::from(text);
    }
    format!("{}{}", " ".repeat(width - len), text)
}

/// The expanded card body together with the row its rejected block starts on.
struct DetailPane<'a> {
    lines: Vec<Line<'a>>,
    rejected_start: Option<usize>,
}

/// Render the expanded card: the meta preview first, then — below the card and
/// behind a dashed rule — the attempts that were rejected on the way to it.
fn detail_pane(draft: &CardDraft, width: usize, pending: bool) -> DetailPane<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    let indent = "      ";
    lines.push(Line::from(""));
    if let Some(meta) = draft.meta() {
        lines.extend(meta_preview(meta, indent, width, pending));
    } else {
        lines.push(Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled("meta not generated yet", palette::dim2()),
        ]));
    }
    let attempts = rejected_attempts(draft);
    let rejected_start = if attempts.is_empty() {
        None
    } else {
        lines.push(Line::from(""));
        lines.push(super::common::dashed_line(
            super::common::display_width(indent),
            width.saturating_sub(super::common::display_width(indent)),
        ));
        lines.push(Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled("rejected attempts", palette::dim2()),
        ]));
        let start = lines.len();
        lines.extend(
            attempts
                .into_iter()
                .map(|attempt| rejected_row(attempt, width).line),
        );
        Some(start)
    };
    if pending {
        lines = lines.into_iter().map(muted_line).collect();
    }
    DetailPane {
        lines,
        rejected_start,
    }
}

const REJECTED_INDENT: &str = "       ";
const REJECTED_STEP_COL_CHARS: usize = 12;
const REJECTED_FILE_COL_CHARS: usize = 22;

/// One rejected attempt of one artifact, as listed in the expanded card.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RejectedAttempt<'a> {
    artifact: Artifact,
    index: usize,
    fault: &'a AttemptFault,
}

impl<'a> RejectedAttempt<'a> {
    fn new(artifact: Artifact, index: usize, fault: &'a AttemptFault) -> Self {
        Self {
            artifact,
            index,
            fault,
        }
    }

    /// Return what this rejected attempt produced, when anything survived it.
    pub(crate) fn artifact(&self) -> Option<&'a Path> {
        self.fault.artifact()
    }
}

/// Return every rejected attempt of one card, in artifact and attempt order.
pub(crate) fn rejected_attempts(draft: &CardDraft) -> Vec<RejectedAttempt<'_>> {
    STEPS
        .iter()
        .flat_map(|&(_, kind)| {
            slot_for(draft.artifacts(), kind)
                .faults()
                .iter()
                .enumerate()
                .map(move |(position, fault)| RejectedAttempt::new(kind, position + 1, fault))
        })
        .collect()
}

/// One rendered rejected-attempt row together with the click targets on it.
pub(crate) struct RejectedRow<'a> {
    line: Line<'a>,
    links: Vec<(u16, u16, PathBuf)>,
}

/// Render one rejected attempt: which try it was, whatever that try produced
/// before it was thrown away, and the gate that rejected it. A try that never
/// produced anything leaves the middle column blank — the reason already says
/// what happened. The archived file is the click target, so the row is built
/// once and reused by the hit-tester instead of being measured twice.
pub(crate) fn rejected_row<'a>(attempt: RejectedAttempt<'_>, width: usize) -> RejectedRow<'a> {
    let step = format!("{} {}", step_name(attempt.artifact), attempt.index);
    let used = REJECTED_INDENT.chars().count()
        + 2
        + REJECTED_STEP_COL_CHARS.max(step.chars().count() + 2)
        + REJECTED_FILE_COL_CHARS;
    let reason = clip(
        format!("{} · {}", attempt.fault.category(), attempt.fault.reason()).as_str(),
        width.saturating_sub(used).max(12),
    );
    let mut spans = vec![
        Span::styled(REJECTED_INDENT, palette::base()),
        Span::styled("✗ ", palette::dim()),
        Span::styled(
            pad_right(step.as_str(), REJECTED_STEP_COL_CHARS),
            palette::dim(),
        ),
    ];
    let mut links: Vec<(u16, u16, PathBuf)> = Vec::new();
    let files_start = spans
        .iter()
        .map(|span| super::common::display_width(span.content.as_ref()))
        .sum::<usize>();
    match archive_label(attempt.artifact()) {
        Some(name) => {
            let column = push_link(
                &mut spans,
                &mut links,
                files_start,
                name,
                attempt.artifact(),
            );
            spans.push(Span::styled(
                column_gap(column, files_start + REJECTED_FILE_COL_CHARS),
                palette::dim(),
            ));
        }
        None => spans.push(Span::styled(
            " ".repeat(REJECTED_FILE_COL_CHARS),
            palette::dim(),
        )),
    }
    spans.push(Span::styled(reason, palette::dim()));
    RejectedRow {
        line: Line::from(spans),
        links,
    }
}

fn archive_label(path: Option<&Path>) -> Option<String> {
    path.and_then(|path| path.file_name().and_then(|name| name.to_str()))
        .map(String::from)
}

fn push_link<'a>(
    spans: &mut Vec<Span<'a>>,
    links: &mut Vec<(u16, u16, PathBuf)>,
    column: usize,
    label: String,
    target: Option<&Path>,
) -> usize {
    let width = super::common::display_width(label.as_str());
    spans.push(Span::styled(
        label,
        palette::dim2().add_modifier(Modifier::UNDERLINED),
    ));
    if let Some(target) = target {
        links.push((
            u16::try_from(column).unwrap_or(u16::MAX),
            u16::try_from(column + width).unwrap_or(u16::MAX),
            target.to_path_buf(),
        ));
    }
    column + width
}

/// Return the plain spacing that pads one label out to its column, so an
/// underline stops at the label instead of trailing through the gap.
fn column_gap(column: usize, next_column: usize) -> String {
    " ".repeat(next_column.saturating_sub(column).max(2))
}

fn pad_right(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return format!("{text} ");
    }
    format!("{text}{}", " ".repeat(width - len))
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return String::from(text);
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn meta_preview<'a>(
    meta: &'a CardMeta,
    indent: &'static str,
    width: usize,
    pending: bool,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let label = |text: &'static str| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::dim2()),
        ])
    };
    lines.push(label("the phrase"));
    lines.extend(value_lines(
        meta.target_sentence(),
        indent,
        width,
        if pending {
            palette::base().add_modifier(Modifier::CROSSED_OUT)
        } else {
            palette::base()
        },
    ));
    lines.push(Line::from(""));
    lines.push(label("in your language"));
    lines.extend(highlight_lines(meta, indent, width));
    lines.push(Line::from(""));
    lines.push(label("a visual clue"));
    lines.extend(value_lines(
        meta.source_hint(),
        indent,
        width,
        palette::base(),
    ));
    lines.push(Line::from(""));
    lines.extend(fact_lines(
        "word meaning",
        String::from(meta.meaning()),
        indent,
        width,
    ));
    lines.extend(fact_lines(
        "word pronunciation",
        format!("/{}/", meta.pronunciation()),
        indent,
        width,
    ));
    lines.extend(fact_lines(
        "phrase pronunciation",
        format!("/{}/", meta.transcription()),
        indent,
        width,
    ));
    lines.extend(fact_lines(
        "worth learning",
        format!("{}/10", meta.importance()),
        indent,
        width,
    ));
    if !meta.source_context().trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(label("the right context"));
        let indent_w = super::common::display_width(indent);
        let inner = width.saturating_sub(indent_w).max(20);
        for line in to_ratatui(&parse_markdown(meta.source_context())) {
            for wrapped in softwrap_line(line, inner) {
                lines.push(restyle_with_indent(wrapped, indent));
            }
        }
    }
    lines
}

fn fact_lines(
    label: &'static str,
    value: String,
    indent: &'static str,
    width: usize,
) -> Vec<Line<'static>> {
    let indent_width = super::common::display_width(indent);
    let inline = width >= indent_width + FACT_LABEL_COLUMN + 12;
    let value_width = if inline {
        width.saturating_sub(indent_width + FACT_LABEL_COLUMN)
    } else {
        width.saturating_sub(indent_width)
    }
    .max(1);
    let values = super::common::wrap_words(value.as_str(), value_width, value_width);
    if !inline {
        let mut lines = vec![Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(label, palette::dim2()),
        ])];
        lines.extend(values.into_iter().map(|value| {
            Line::from(vec![
                Span::styled(indent, palette::base()),
                Span::styled(value, palette::base()),
            ])
        }));
        return lines;
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let label = if index == 0 {
                super::common::pad_right(label, FACT_LABEL_COLUMN)
            } else {
                " ".repeat(FACT_LABEL_COLUMN)
            };
            Line::from(vec![
                Span::styled(indent, palette::base()),
                Span::styled(label, palette::dim2()),
                Span::styled(value, palette::base()),
            ])
        })
        .collect()
}

fn value_lines<'a>(
    text: &str,
    indent: &'static str,
    width: usize,
    style: ratatui::style::Style,
) -> Vec<Line<'a>> {
    let indent_w = super::common::display_width(indent);
    let inner = width.saturating_sub(indent_w).max(20);
    super::common::wrap_words(text, inner, inner)
        .into_iter()
        .map(|chunk| {
            Line::from(vec![
                Span::styled(indent, palette::base()),
                Span::styled(chunk, style),
            ])
        })
        .collect()
}

/// Soft-wrap one markdown-rendered line into chunks that each fit `width`
/// display columns. Preserves per-span style + modifiers across wrap points.
/// CJK code points count as two cells, others as one — close enough to the
/// real terminal width without pulling in `unicode-width`.
fn softwrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;
    let mut pending_space: Option<ratatui::style::Style> = None;
    for span in line.spans {
        for (token, whitespace) in styled_tokens(span.content.as_ref()) {
            if whitespace {
                if current_width > 0 {
                    pending_space = Some(span.style);
                }
                continue;
            }
            let token_width = super::common::display_width(token.as_str());
            let gap = usize::from(current_width > 0 && pending_space.is_some());
            if current_width + gap + token_width <= width {
                if let Some(style) = pending_space.take() {
                    current_spans.push(Span::styled(String::from(" "), style));
                    current_width += 1;
                }
                current_spans.push(Span::styled(token, span.style));
                current_width += token_width;
                continue;
            }
            if !current_spans.is_empty() {
                out.push(Line::from(std::mem::take(&mut current_spans)));
                current_width = 0;
                pending_space = None;
            }
            if token_width <= width {
                current_spans.push(Span::styled(token, span.style));
                current_width = token_width;
                continue;
            }
            let parts = split_token(token.as_str(), width);
            let last = parts.len().saturating_sub(1);
            for (index, part) in parts.into_iter().enumerate() {
                current_spans.push(Span::styled(part.clone(), span.style));
                current_width = super::common::display_width(part.as_str());
                if index < last {
                    out.push(Line::from(std::mem::take(&mut current_spans)));
                    current_width = 0;
                }
            }
        }
    }
    if !current_spans.is_empty() {
        out.push(Line::from(current_spans));
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

fn styled_tokens(text: &str) -> Vec<(String, bool)> {
    let mut tokens: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_space: Option<bool> = None;
    for ch in text.chars() {
        let space = ch.is_whitespace();
        if current_space == Some(space) {
            current.push(ch);
            continue;
        }
        if !current.is_empty() {
            tokens.push((std::mem::take(&mut current), current_space.unwrap_or(false)));
        }
        current.push(ch);
        current_space = Some(space);
    }
    if !current.is_empty() {
        tokens.push((current, current_space.unwrap_or(false)));
    }
    tokens
}

fn split_token(text: &str, width: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in text.chars() {
        let char_width = super::common::char_width(ch);
        if current_width > 0 && current_width + char_width > width {
            parts.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += char_width;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

/// Repaint one markdown-rendered line with the preview palette and prepend the
/// shared indent span. The markdown layer emits neutral spans carrying only
/// BOLD / ITALIC modifiers — we keep those and force the foreground colour to
/// `palette::base()` so context blends with the surrounding preview rows.
fn restyle_with_indent(line: Line<'static>, indent: &'static str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::styled(indent, palette::base()));
    for span in line.spans {
        let style = palette::base().add_modifier(span.style.add_modifier);
        spans.push(Span::styled(span.content, style));
    }
    Line::from(spans)
}

fn highlight_lines<'a>(meta: &'a CardMeta, indent: &'static str, width: usize) -> Vec<Line<'a>> {
    let sentence = meta.source_sentence();
    let highlight = meta.source_highlight();
    if highlight.is_empty() {
        return value_lines(sentence, indent, width, palette::base());
    }
    let line = if let Some(pos) = sentence.find(highlight) {
        let head = &sentence[..pos];
        let middle = &sentence[pos..pos + highlight.len()];
        let tail = &sentence[pos + highlight.len()..];
        Line::from(vec![
            Span::styled(head.to_string(), palette::base()),
            Span::styled(
                middle.to_string(),
                palette::base().add_modifier(Modifier::BOLD),
            ),
            Span::styled(tail.to_string(), palette::base()),
        ])
    } else {
        Line::from(vec![Span::styled(sentence.to_string(), palette::base())])
    };
    let indent_w = super::common::display_width(indent);
    let inner = width.saturating_sub(indent_w).max(20);
    softwrap_line(line, inner)
        .into_iter()
        .map(|line| restyle_with_indent(line, indent))
        .collect()
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}

/// Return the card and its content range at one scrolling-body row.
#[must_use]
pub(crate) fn card_range_at(app: &App, width: usize, row: usize) -> Option<(usize, usize, usize)> {
    let running_target = app.cards_running_target();
    let mut offset = 0usize;
    for (index, draft) in app.cards().iter().enumerate() {
        let running =
            running_target.and_then(|(card, artifact)| (card == index).then_some(artifact));
        let expanded = index == app.card_selected() && app.card_expanded();
        let editor = if index == app.card_selected() {
            app.sentence_editor()
        } else {
            None
        };
        let (rows, trailing) = card_layout(draft, running, expanded, editor, width);
        let end = offset.saturating_add(rows);
        if row >= offset && row < end {
            return Some((index, offset, end));
        }
        offset = end.saturating_add(trailing);
    }
    None
}

/// Row offset and height of the currently focused card inside the scrolling
/// card list, in body-rect rows. Returns `None` when there are no cards yet.
/// Mirrors the per-card layout used by `cards_paragraph` so scroll-snapping
/// and renderer stay in lockstep. `width` is the body-rect width in chars.
pub(crate) fn focused_card_range(app: &App, width: usize) -> Option<(u16, u16)> {
    if app.cards().is_empty() {
        return None;
    }
    let running_target = app.cards_running_target();
    let mut offset: usize = 0;
    for (idx, draft) in app.cards().iter().enumerate() {
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let expanded = idx == app.card_selected() && app.card_expanded();
        let editor = if idx == app.card_selected() {
            app.sentence_editor()
        } else {
            None
        };
        let (rows, trailing) = card_layout(draft, running_for_card, expanded, editor, width);
        if idx == app.card_selected() {
            let focus_rows = if expanded {
                let base = head_rows(draft, width)
                    .saturating_add(step_rows_for(draft, running_for_card).len());
                editor
                    .map(|editor| {
                        base.saturating_add(SECTION_GAP_ROWS).saturating_add(
                            super::sentence_labels::editor_focus_end(
                                editor,
                                width,
                                super::common::CARD_DETAIL_COLUMN,
                                super::common::CARD_DETAIL_COLUMN,
                            ),
                        )
                    })
                    .unwrap_or(base)
                    .max(1)
            } else {
                rows
            };
            return Some((
                u16::try_from(offset).unwrap_or(u16::MAX),
                u16::try_from(focus_rows).unwrap_or(u16::MAX),
            ));
        }
        offset = offset.saturating_add(rows + trailing);
    }
    None
}

/// Total number of lines `cards_paragraph` will produce for the current state
/// of `app`. Mirrors the per-card layout: head rows (one or more, depending on
/// how the term + meta sentence wrap) + visible artifact rows + the editor and
/// optional detail pane + trailing blank line. Used by both the scroll clamp in
/// `tui::app` and the click hit tester in `tui::links`, so they stay in lockstep
/// with the renderer. `width` is the body-rect width in chars.
pub(crate) fn content_height(app: &App, width: usize) -> u16 {
    if app.cards().is_empty() {
        return 0;
    }
    let running_target = app.cards_running_target();
    let mut total: usize = 0;
    for (idx, draft) in app.cards().iter().enumerate() {
        let running_for_card =
            running_target.and_then(|(card, kind)| if card == idx { Some(kind) } else { None });
        let expanded = idx == app.card_selected() && app.card_expanded();
        let editor = if idx == app.card_selected() {
            app.sentence_editor()
        } else {
            None
        };
        let (rows, trailing) = card_layout(draft, running_for_card, expanded, editor, width);
        total = total.saturating_add(rows + trailing);
    }
    u16::try_from(total).unwrap_or(u16::MAX)
}

/// Number of head rows the focused card produces given the current body-rect
/// width. Mirrors the wrap that `card_head` runs at render time, so the click
/// hit-tester in `tui::links` can find the start of the step rows.
pub(crate) fn head_rows_for(draft: &CardDraft, width: usize) -> usize {
    head_rows(draft, width)
}

/// Return whether one cell relative to the meta row belongs to a summary tag.
#[must_use]
pub(crate) fn sentence_tag_hit_at(
    draft: &CardDraft,
    running: Option<Artifact>,
    width: usize,
    row: usize,
    column: usize,
) -> bool {
    let Some(meta) = meta_step_index(draft, running) else {
        return false;
    };
    let Some(labels) = collapsed_labels(draft, running, width) else {
        return false;
    };
    let steps = step_rows_for(draft, running);
    let Some(artifact) = steps.get(meta.saturating_add(row)).copied() else {
        return false;
    };
    let Some(row) = sentence_tag_row(artifact) else {
        return false;
    };
    let Some(column) = column.checked_sub(labels.start) else {
        return false;
    };
    labels.tags.hit_at(row, column)
}

/// Return whether the complete collapsed tag sequence fits beside the artifacts.
#[must_use]
pub(crate) fn sentence_tags_visible(
    draft: &CardDraft,
    running: Option<Artifact>,
    width: usize,
) -> bool {
    collapsed_labels(draft, running, width).is_some()
}

/// Return the expanded sentence-editor control at one cell relative to the
/// card's meta row.
pub(crate) fn sentence_editor_control_at(
    draft: &CardDraft,
    running: Option<Artifact>,
    editor: &SentenceLabelsEditor,
    width: usize,
    row: usize,
    column: usize,
) -> Option<super::sentence_labels::EditorControl> {
    let meta = meta_step_index(draft, running)?;
    let steps = step_rows_for(draft, running);
    let row = row.checked_sub(
        steps
            .len()
            .saturating_sub(meta)
            .saturating_add(SECTION_GAP_ROWS),
    )?;
    super::sentence_labels::editor_control_at(
        editor,
        width,
        column,
        row,
        super::common::CARD_DETAIL_COLUMN,
        super::common::CARD_DETAIL_COLUMN,
    )
}

/// Return the artifact step rows visible for one card.
pub(crate) fn step_rows_for(draft: &CardDraft, running: Option<Artifact>) -> Vec<Artifact> {
    let artifacts = draft.artifacts();
    STEPS
        .iter()
        .filter_map(|&(_, kind)| {
            if kind == Artifact::Meta && draft.meta().is_some()
                || slot_visible(slot_for(artifacts, kind), kind, running)
            {
                Some(kind)
            } else {
                None
            }
        })
        .collect()
}

fn card_layout(
    draft: &CardDraft,
    running: Option<Artifact>,
    expanded: bool,
    editor: Option<&SentenceLabelsEditor>,
    width: usize,
) -> (usize, usize) {
    let steps = step_rows_for(draft, running);
    let labels = sentence_label_extra_rows(draft, running, editor, expanded, width);
    let mut rows = head_rows(draft, width)
        .saturating_add(steps.len())
        .saturating_add(labels);
    if expanded {
        rows = rows.saturating_add(detail_pane_height(draft, width));
    }
    let trailing = usize::from(!steps.is_empty() || labels > 0 || expanded);
    (rows, trailing)
}

/// Number of rows added to the artifact block by the sentence pane.
pub(crate) fn sentence_label_extra_rows(
    _draft: &CardDraft,
    _running: Option<Artifact>,
    editor: Option<&SentenceLabelsEditor>,
    expanded: bool,
    width: usize,
) -> usize {
    if !expanded {
        return 0;
    }
    editor
        .map(|editor| {
            SECTION_GAP_ROWS.saturating_add(
                super::sentence_labels::editor_lines(
                    editor,
                    width,
                    super::common::CARD_DETAIL_COLUMN,
                    super::common::CARD_DETAIL_COLUMN,
                )
                .len(),
            )
        })
        .unwrap_or(0)
}

fn meta_step_index(draft: &CardDraft, running: Option<Artifact>) -> Option<usize> {
    step_rows_for(draft, running)
        .iter()
        .position(|artifact| *artifact == Artifact::Meta)
}

/// Locate the sentence editor cursor inside the complete scrolling card
/// content.
pub(crate) fn sentence_editor_cursor_for(app: &App, width: usize) -> Option<(usize, usize)> {
    let editor = app.sentence_editor()?;
    let selected = app.card_selected();
    let running_target = app.cards_running_target();
    let mut offset = 0usize;
    for (index, draft) in app.cards().iter().enumerate() {
        let running =
            running_target.and_then(|(card, artifact)| (card == index).then_some(artifact));
        if index == selected {
            let steps = step_rows_for(draft, running);
            let (column, row) = super::sentence_labels::editor_cursor(
                editor,
                width,
                super::common::CARD_DETAIL_COLUMN,
                super::common::CARD_DETAIL_COLUMN,
            )?;
            return Some((
                column,
                offset
                    .saturating_add(head_rows(draft, width))
                    .saturating_add(steps.len())
                    .saturating_add(SECTION_GAP_ROWS)
                    .saturating_add(row),
            ));
        }
        let (rows, trailing) = card_layout(draft, running, false, None, width);
        offset = offset.saturating_add(rows).saturating_add(trailing);
    }
    None
}

/// Number of body-rect rows the expanded meta-preview pane consumes for one
/// card. Verbatim mirror of `detail_pane` / `meta_preview` so callers can keep
/// scroll offsets and click hit-tests aligned with the rendered output.
pub(crate) fn detail_pane_height(draft: &CardDraft, width: usize) -> usize {
    detail_pane(draft, width, draft.staged_rewrite().is_some())
        .lines
        .len()
}

/// Row offset of the first rejected-attempt row inside the expanded detail
/// pane, counted from the pane's first row. Taken from the rendered pane
/// itself, so the click hit-tester cannot drift from the rejected block that
/// sits below the card.
pub(crate) fn rejected_rows_offset(draft: &CardDraft, width: usize) -> Option<usize> {
    detail_pane(draft, width, draft.staged_rewrite().is_some()).rejected_start
}

/// Click targets on one rejected row: the archived frame and its scene.
pub(crate) fn rejected_link_columns(
    attempt: RejectedAttempt<'_>,
    width: usize,
) -> Vec<(u16, u16, PathBuf)> {
    rejected_row(attempt, width).links
}

/// Return one card's known Gemini cost across generated artifacts.
pub(crate) fn card_cost(draft: &CardDraft) -> Option<GenerationCost> {
    draft.artifacts().cost()
}

/// Return the known Gemini cost across every card in the current batch.
pub(crate) fn total_cost(app: &App) -> Option<GenerationCost> {
    let cost = app
        .cards()
        .iter()
        .filter_map(card_cost)
        .sum::<GenerationCost>();
    if cost.nanos() == 0 {
        return None;
    }
    Some(cost)
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled("step 3/3", palette::dim2()));
    left.push(super::common::status_sep());
    left.push(Span::styled(
        app.cards_ready().to_string(),
        palette::base().add_modifier(Modifier::BOLD),
    ));
    left.push(Span::styled(
        format!("/{} ready", app.cards().len()),
        palette::dim(),
    ));
    if app.cards_pending() > 0 {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            app.cards_pending().to_string(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(" pending", palette::dim()));
    }
    if let Some(cost) = total_cost(app) {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            cost.dollars_cents(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
    }
    left.push(super::common::status_sep());
    left.push(Span::styled(elapsed(app), palette::dim2()));
    let hints = if app.sentence_editor().is_some() {
        vec![
            super::common::FooterHint::primary("Ctrl+G", "regenerate"),
            super::common::FooterHint::secondary("← →", "pick"),
            super::common::FooterHint::ghost("↑ ↓", "row"),
            super::common::FooterHint::ghost("Esc", "close"),
        ]
    } else {
        let controls = DisclosureControls::new(app.card_expanded());
        let mut hints = Vec::new();
        hints.push(super::common::FooterHint::primary("Ctrl+G", "regenerate"));
        if app.card_tunable() {
            hints.push(super::common::FooterHint::secondary("Enter/→", "tune"));
        } else {
            hints.push(controls.secondary_toggle());
        }
        hints.push(super::common::FooterHint::ghost("↑↓", "nav"));
        if app.can_start_new_batch() {
            hints.push(super::common::new_batch_hint(app.new_batch_pending()));
        }
        hints.push(super::common::quit_hint(app.quit_pending()));
        hints
    };
    super::common::footer_bar(left, hints, width)
}

fn elapsed(app: &App) -> String {
    let seconds = app.elapsed().as_secs();
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes:02}:{remainder:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn markdown_softwrap_moves_whole_words_to_the_next_row() {
        let rows = softwrap_line(Line::from("alpha beta gamma"), 10)
            .iter()
            .map(plain)
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![String::from("alpha beta"), String::from("gamma")],
            "markdown preview wrap must move whole words instead of splitting them"
        );
    }
}
