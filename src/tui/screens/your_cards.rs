//! Renderer for the `your cards` / `building your cards` screen.
//!
//! Mirrors `kamishibai-simple/project/steps-2.jsx` (StepGenerating). One block
//! per card: head row plus four step lines (meta · scene · audio · picture).
//! "meta" is the rich body produced by the Pro Gemini pass and is the first
//! real step in the pipeline. When a card is selected and finished the row
//! expands into a body preview + artifact pane.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::session::{Artifact, ArtifactSlot, CardArtifacts, CardBody, CardDraft};
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE_WORKING: &str = "building your cards";
const HEADLINE_DONE: &str = "your cards";
const HINT_WORKING: &str = "drawing each card one by one";
const HINT_DONE: &str = "all done";
const HINT_DONE_FAILED: &str = "some cards didn't make it";
const STEP_NAMES: [&str; 4] = ["meta", "audio", "scene", "picture"];
const SPINNER_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

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
        footer(app, all_finished(app), width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        let finished = all_finished(app);
        frame.render_widget(
            body(app, finished, area.width as usize).scroll((app.body_scroll(), 0)),
            area,
        );
    }
}

fn body(app: &App, all_finished: bool, width: usize) -> Paragraph<'_> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    if all_finished {
        lines.extend(outputs_banner(app));
        lines.push(Line::from(""));
    }
    if app.cards().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("preparing cards…", palette::dim())));
        return Paragraph::new(lines).style(palette::base());
    }
    let spinner_frame = (app.elapsed().as_millis() / 180) as usize % SPINNER_FRAMES.len();
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
            width,
            running_for_card,
            spinner_frame,
        ));
    }
    Paragraph::new(lines).style(palette::base())
}

fn outputs_banner(app: &App) -> Vec<Line<'_>> {
    let done = app.done_artifacts();
    let entries: Vec<(&str, &str)> = [("APKG", done.deck.as_str()), ("PDF", done.report.as_str())]
        .into_iter()
        .filter(|(_, path)| !path.is_empty())
        .collect();
    if entries.is_empty() {
        return Vec::new();
    }
    let mut top: Vec<Span<'_>> = vec![Span::styled("│ ", palette::base())];
    for (idx, (label, _)) in entries.iter().enumerate() {
        if idx > 0 {
            top.push(Span::styled("    ", palette::base()));
        }
        top.push(Span::styled("↓ ", palette::dim()));
        top.push(Span::styled(String::from(*label), palette::link()));
    }
    vec![Line::from(top)]
}

fn card_block<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    width: usize,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    lines.push(card_head(draft, idx, focused, expanded, width));
    let artifacts = draft.artifacts();
    for name in STEP_NAMES {
        lines.push(step_line(name, artifacts, running, spinner_frame));
    }
    if expanded {
        lines.extend(detail_pane(draft));
    }
    lines.push(Line::from(""));
    lines
}

fn card_head<'a>(
    draft: &'a CardDraft,
    idx: usize,
    focused: bool,
    expanded: bool,
    width: usize,
) -> Line<'a> {
    let row_style = if focused {
        palette::highlight()
    } else {
        palette::base()
    };
    let glyph = if expanded {
        "▾"
    } else if card_finished(draft) {
        "▸"
    } else if any_running(draft.artifacts()) {
        "·"
    } else {
        " "
    };
    let glyph_style = if focused {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::dim2()
    };
    let num_style = if focused {
        palette::highlight()
    } else {
        palette::dim2()
    };
    let term_style = if focused {
        palette::highlight().add_modifier(Modifier::BOLD)
    } else {
        palette::base()
    };
    let summary = card_summary(draft);
    let summary_style = if focused {
        palette::highlight_dim()
    } else {
        palette::dim()
    };
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(format!(" {glyph} "), glyph_style));
    spans.push(Span::styled(format!("{:0>2}  ", idx + 1), num_style));
    spans.push(Span::styled(
        super::common::pad_right(draft.term(), 18),
        term_style,
    ));
    spans.push(Span::styled(summary, summary_style));
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    Line::from(spans)
}

fn card_finished(draft: &CardDraft) -> bool {
    let artifacts = draft.artifacts();
    artifacts.all_ready() || artifacts.has_failed()
}

fn any_running(artifacts: &CardArtifacts) -> bool {
    for slot in [
        artifacts.body(),
        artifacts.scene(),
        artifacts.picture(),
        artifacts.sound(),
    ] {
        if slot.ready() || slot.discarded() || slot.failed_terminally() {
            continue;
        }
        if slot.tally().done() > 0 {
            return true;
        }
    }
    false
}

fn card_summary(draft: &CardDraft) -> String {
    let artifacts = draft.artifacts();
    if artifacts.has_failed() {
        return String::from("gave up");
    }
    if artifacts.all_ready() {
        return String::from("ready");
    }
    let done = step_done_count(artifacts);
    if done > 0 {
        return format!("{done}/4");
    }
    String::from("queued")
}

fn step_done_count(artifacts: &CardArtifacts) -> usize {
    let mut done = 0usize;
    if artifacts.body().ready() {
        done += 1;
    }
    if artifacts.scene().ready() {
        done += 1;
    }
    if artifacts.picture().ready() {
        done += 1;
    }
    if artifacts.sound().ready() {
        done += 1;
    }
    done
}

fn step_line<'a>(
    name: &'a str,
    artifacts: &'a CardArtifacts,
    running: Option<Artifact>,
    spinner_frame: usize,
) -> Line<'a> {
    let slot_kind: Artifact = match name {
        "meta" => Artifact::Body,
        "scene" => Artifact::Scene,
        "audio" => Artifact::Sound,
        "picture" => Artifact::Picture,
        _ => Artifact::Body,
    };
    let slot: &ArtifactSlot = match slot_kind {
        Artifact::Body => artifacts.body(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    };
    let active = running == Some(slot_kind);
    let (glyph, status_style, name_style, note_spans) = step_state(slot, active, spinner_frame);
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled("    ", palette::base()));
    spans.push(Span::styled(format!("{glyph} "), status_style));
    spans.push(Span::styled(super::common::pad_right(name, 9), name_style));
    spans.extend(note_spans);
    Line::from(spans)
}

fn step_state<'a>(
    slot: &'a ArtifactSlot,
    active: bool,
    spinner_frame: usize,
) -> (
    String,
    ratatui::style::Style,
    ratatui::style::Style,
    Vec<Span<'a>>,
) {
    let row_dim = palette::dim();
    let row_dim2 = palette::dim2();
    let row_fg = palette::base();
    if slot.ready() {
        let mut note: Vec<Span<'a>> = Vec::new();
        if let Some(file) = slot.file() {
            note.push(Span::styled(String::from(file.name()), palette::link()));
            note.push(Span::styled(format!(" · {}", file.size()), palette::dim()));
        }
        return (String::from("✓"), row_fg, row_fg, note);
    }
    if slot.discarded() {
        return (
            String::from("⊘"),
            row_dim,
            row_dim,
            vec![Span::styled(String::from("discarded"), palette::dim())],
        );
    }
    if slot.failed_terminally() {
        return (
            String::from("✗"),
            row_fg,
            row_fg,
            vec![Span::styled(
                String::from("gave up after 3 tries"),
                palette::dim(),
            )],
        );
    }
    let attempts = slot.tally().done();
    if attempts > 0 {
        let label = if active {
            format!("retry {}/3…", attempts + 1)
        } else {
            format!("retry {}/3 paused", attempts + 1)
        };
        let glyph = if active {
            String::from(SPINNER_FRAMES[spinner_frame])
        } else {
            String::from("·")
        };
        return (
            glyph,
            row_fg,
            row_fg,
            vec![Span::styled(label, palette::dim())],
        );
    }
    if active {
        return (
            String::from(SPINNER_FRAMES[spinner_frame]),
            row_fg,
            row_fg,
            vec![Span::styled(String::from("working…"), palette::dim())],
        );
    }
    (
        String::from("○"),
        row_dim2,
        row_dim2,
        vec![Span::styled(String::from("queued"), palette::dim())],
    )
}

fn detail_pane(draft: &CardDraft) -> Vec<Line<'_>> {
    let mut lines: Vec<Line<'_>> = Vec::new();
    let indent = "      ";
    lines.push(Line::from(""));
    if let Some(body) = draft.body() {
        lines.extend(body_preview(body, indent));
    } else {
        lines.push(Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled("body not generated yet", palette::dim2()),
        ]));
    }
    lines
}

fn body_preview<'a>(body: &'a CardBody, indent: &'static str) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let label = |text: &'static str| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::dim2()),
        ])
    };
    let value = |text: String| {
        Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(text, palette::base()),
        ])
    };
    lines.push(label("target"));
    lines.push(value(body.target_sentence().to_string()));
    lines.push(Line::from(""));
    lines.push(label("source"));
    lines.push(highlight_line(body, indent));
    lines.push(Line::from(""));
    lines.push(label("hint"));
    lines.push(value(body.source_hint().to_string()));
    lines.push(Line::from(""));
    lines.push(label(
        "meaning · pronunciation · transcription · importance",
    ));
    lines.push(value(format!(
        "{} · /{}/ · /{}/ · {}/10",
        body.meaning(),
        body.pronunciation(),
        body.transcription(),
        body.importance(),
    )));
    if !body.source_context().trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(label("context"));
        for chunk in body.source_context().lines() {
            lines.push(value(chunk.to_string()));
        }
    }
    lines
}

fn highlight_line<'a>(body: &'a CardBody, indent: &'static str) -> Line<'a> {
    let sentence = body.source_sentence();
    let highlight = body.source_highlight();
    if highlight.is_empty() {
        return Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(sentence.to_string(), palette::base()),
        ]);
    }
    if let Some(pos) = sentence.find(highlight) {
        let head = &sentence[..pos];
        let middle = &sentence[pos..pos + highlight.len()];
        let tail = &sentence[pos + highlight.len()..];
        return Line::from(vec![
            Span::styled(indent, palette::base()),
            Span::styled(head.to_string(), palette::base()),
            Span::styled(
                middle.to_string(),
                palette::base().add_modifier(Modifier::BOLD),
            ),
            Span::styled(tail.to_string(), palette::base()),
        ]);
    }
    Line::from(vec![
        Span::styled(indent, palette::base()),
        Span::styled(sentence.to_string(), palette::base()),
    ])
}

fn all_finished(app: &App) -> bool {
    !app.cards().is_empty()
        && app
            .cards()
            .iter()
            .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed())
}

fn footer(app: &App, all_finished: bool, width: u16) -> Paragraph<'static> {
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
    if app.cards_failed() > 0 {
        left.push(super::common::status_sep());
        left.push(Span::styled(
            app.cards_failed().to_string(),
            palette::base().add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(" gave up", palette::dim()));
    }
    left.push(super::common::status_sep());
    left.push(Span::styled(elapsed(app), palette::dim2()));
    let mut right: Vec<Span<'static>> = Vec::new();
    if all_finished {
        right.extend(super::common::key_hint("↑↓", "open"));
        right.push(super::common::status_sep());
        right.extend(super::common::key_hint("n", "new batch"));
    } else {
        right.push(Span::styled("working…", palette::dim2()));
    }
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}

fn elapsed(app: &App) -> String {
    let seconds = app.elapsed().as_secs();
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes:02}:{remainder:02}")
}
