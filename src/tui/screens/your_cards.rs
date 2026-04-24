//! Renderer for the `Your cards` screen.
//!
//! Mirrors states 03, 03c (retrying), 03d (couldn't finish) of the design
//! mockup (`states.js` · "Your cards" family).

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::session::{Artifact, ArtifactSlot, CardArtifacts, CardDraft};
use crate::tui::app::App;
use crate::tui::palette;

const HEADLINE: &str = "Your cards";
const HINT_KEYS_DEFAULT: &str =
    "[↑↓] nav · [Enter] expand/collapse · [R] change this card · [d] drop artifact";
const HINT_KEYS_FAILED: &str =
    "[r] regenerate failed · [Enter] keep going anyway · [R] change this card";

/// Draw the `Your cards` screen for the current `App`.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let has_failure = app.cards_failed() > 0;
    let frame_rects = super::common::frame(area);
    frame.render_widget(
        super::common::language_badge(app).style(palette::base()),
        frame_rects.badge,
    );
    frame.render_widget(
        super::common::header(HEADLINE, &tagline(app), frame_rects.header.width),
        frame_rects.header,
    );
    let body_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame_rects.body);
    frame.render_widget(
        super::common::section_divider("cards", body_rows[0].width),
        body_rows[0],
    );
    frame.render_widget(cards_panel(app, body_rows[2].width), body_rows[2]);
    frame.render_widget(
        super::common::section_divider("status", body_rows[3].width),
        body_rows[3],
    );
    frame.render_widget(status_panel(app), body_rows[5]);
    let hint_keys = if has_failure {
        HINT_KEYS_FAILED
    } else {
        HINT_KEYS_DEFAULT
    };
    frame.render_widget(
        super::common::dashed_divider(frame_rects.footer_rule.width),
        frame_rects.footer_rule,
    );
    frame.render_widget(
        super::common::footer(hint_keys, frame_rects.footer.width),
        frame_rects.footer,
    );
}

fn tagline(app: &App) -> String {
    let failed = app.cards_failed();
    if failed > 0 {
        let plural = if failed == 1 { "" } else { "s" };
        format!("{failed} card{plural} gave up after 3 tries")
    } else {
        format!(
            "group 1 of 1 · {}/{} ready",
            app.cards_ready(),
            app.cards().len()
        )
    }
}

fn cards_panel(app: &App, width: u16) -> Paragraph<'_> {
    if app.cards().is_empty() {
        return Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("preparing cards…", palette::dim())),
        ])
        .style(palette::base());
    }
    let mut lines: Vec<Line<'_>> = Vec::new();
    if app.cards_failed() > 0 {
        lines.extend(failure_banner(width));
    } else {
        lines.push(Line::from(""));
    }
    for (index, draft) in app.cards().iter().enumerate() {
        let focused = index == app.card_selected();
        let expanded = focused && app.card_expanded();
        lines.push(card_row(draft, index, focused, expanded));
        if expanded {
            lines.extend(expanded_rows(draft, width));
        }
    }
    Paragraph::new(lines).style(palette::base())
}

fn card_row<'a>(draft: &'a CardDraft, index: usize, focused: bool, expanded: bool) -> Line<'a> {
    let glyph = row_marker(draft, expanded);
    let marker_style = if draft.artifacts().has_failed() {
        palette::failure()
    } else if draft.artifacts().all_ready() {
        palette::base()
    } else if working(draft.artifacts()) {
        palette::wn()
    } else {
        palette::dim()
    };
    let term_style = if focused {
        Style::default()
            .bg(palette::BG)
            .fg(palette::FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(palette::BG).fg(palette::FG)
    };
    let mut spans = vec![
        Span::styled(glyph, marker_style),
        Span::raw("  "),
        Span::styled(format!("{:>2}.", index + 1), palette::base()),
        Span::raw(" "),
        Span::styled(format!("{:<14}", draft.term()), term_style),
    ];
    spans.extend(artifact_span("scene", draft.artifacts().scene()));
    spans.push(Span::raw("   "));
    spans.extend(artifact_span("picture", draft.artifacts().picture()));
    spans.push(Span::raw("   "));
    spans.extend(artifact_span("sound", draft.artifacts().sound()));
    if let Some(note) = failure_note(draft) {
        spans.push(Span::raw("    "));
        spans.push(Span::styled(note.to_string(), palette::dim()));
    } else if let Some(note) = retry_note(draft) {
        spans.push(Span::raw("    "));
        spans.push(Span::styled(note.to_string(), palette::dim()));
    }
    Line::from(spans)
}

fn row_marker(draft: &CardDraft, expanded: bool) -> &'static str {
    if draft.artifacts().has_failed() {
        return "✗";
    }
    if draft.artifacts().all_ready() {
        if expanded { "▾" } else { "▸" }
    } else if working(draft.artifacts()) {
        "◐"
    } else {
        "○"
    }
}

fn artifact_span(name: &'static str, slot: &ArtifactSlot) -> Vec<Span<'static>> {
    if slot.ready() {
        return vec![Span::styled(format!("✓ {name}"), palette::ok())];
    }
    if slot.discarded() {
        return vec![Span::styled(format!("⊘ {name}"), palette::dim())];
    }
    if slot.failed_terminally() {
        return vec![Span::styled(format!("✗ {name}"), palette::failure())];
    }
    let attempts = slot.tally().done();
    if attempts > 0 {
        return vec![Span::styled(
            format!("↻ {name} {attempts}/3"),
            palette::wn(),
        )];
    }
    vec![Span::styled(format!("○ {name}"), palette::dim())]
}

fn retry_note(draft: &CardDraft) -> Option<&'static str> {
    for slot in [
        draft.artifacts().scene(),
        draft.artifacts().picture(),
        draft.artifacts().sound(),
    ] {
        if slot.ready() || slot.discarded() || slot.failed_terminally() {
            continue;
        }
        let attempts = slot.tally().done();
        if attempts == 0 {
            continue;
        }
        return Some(match slot.kind() {
            Artifact::Scene => "· retrying scene",
            Artifact::Picture => "· retrying picture",
            Artifact::Sound => "· retrying sound",
        });
    }
    None
}

fn failure_note(draft: &CardDraft) -> Option<&'static str> {
    for slot in [
        draft.artifacts().scene(),
        draft.artifacts().picture(),
        draft.artifacts().sound(),
    ] {
        if !slot.failed_terminally() {
            continue;
        }
        return Some(match slot.kind() {
            Artifact::Scene => "· scene couldn't finish",
            Artifact::Picture => "· picture couldn't finish",
            Artifact::Sound => "· sound couldn't finish",
        });
    }
    None
}

fn working(artifacts: &CardArtifacts) -> bool {
    for slot in [artifacts.scene(), artifacts.picture(), artifacts.sound()] {
        if slot.ready() || slot.discarded() || slot.failed_terminally() {
            continue;
        }
        if slot.tally().done() > 0 {
            return true;
        }
    }
    false
}

const EXPAND_INDENT: &str = "    ";

fn expanded_rows<'a>(draft: &'a CardDraft, width: u16) -> Vec<Line<'a>> {
    let rule = |label: &str| -> Line<'a> {
        let prefix = format!("── {label} ");
        let prefix_len = prefix.chars().count();
        let remaining = (width as usize)
            .saturating_sub(prefix_len + EXPAND_INDENT.len())
            .max(3);
        Line::from(vec![
            Span::raw(EXPAND_INDENT),
            Span::styled(format!("{prefix}{}", "─".repeat(remaining)), palette::dim()),
        ])
    };
    let hint = draft.payload().hint();
    let mut rows: Vec<Line<'a>> = vec![
        Line::from(""),
        rule("front"),
        Line::from(""),
        sentence_row(draft.payload().front(), draft.payload().highlight()),
    ];
    if !hint.is_empty() {
        rows.push(Line::from(vec![
            Span::raw(EXPAND_INDENT),
            Span::styled(
                String::from(hint),
                palette::dim().add_modifier(Modifier::ITALIC),
            ),
        ]));
    }
    rows.push(Line::from(""));
    rows.push(rule("back"));
    rows.push(Line::from(""));
    rows.extend(text_rows(draft.payload().back()));
    rows.push(Line::from(""));
    rows.push(rule("files"));
    rows.push(Line::from(""));
    rows.extend(file_rows(draft.artifacts()));
    rows.push(Line::from(""));
    rows.push(rule("change this card"));
    rows.push(Line::from(""));
    rows.push(Line::from(vec![
        Span::raw(EXPAND_INDENT),
        Span::styled("[R]", palette::key()),
        Span::raw(" "),
        Span::styled("change this card", palette::dim()),
        Span::raw("   "),
        Span::styled("[d]", palette::key()),
        Span::raw(" "),
        Span::styled("drop picture / scene / sound", palette::dim()),
    ]));
    rows.push(Line::from(""));
    rows
}

fn sentence_row(text: &str, highlight: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(EXPAND_INDENT));
    if !highlight.is_empty()
        && let Some(start) = text.find(highlight)
    {
        let (before, rest) = text.split_at(start);
        let (matched, after) = rest.split_at(highlight.len());
        if !before.is_empty() {
            spans.push(Span::styled(String::from(before), palette::base()));
        }
        spans.push(Span::styled(
            String::from(matched),
            palette::base().add_modifier(Modifier::REVERSED),
        ));
        if !after.is_empty() {
            spans.push(Span::styled(String::from(after), palette::base()));
        }
    } else {
        spans.push(Span::styled(String::from(text), palette::base()));
    }
    Line::from(spans)
}

fn text_rows(text: &str) -> Vec<Line<'static>> {
    if text.is_empty() {
        return vec![Line::from(vec![
            Span::raw(EXPAND_INDENT),
            Span::styled("—", palette::dim()),
        ])];
    }
    text.split('\n')
        .map(|line| {
            Line::from(vec![
                Span::raw(EXPAND_INDENT),
                Span::styled(String::from(line), palette::base()),
            ])
        })
        .collect()
}

fn file_rows(artifacts: &CardArtifacts) -> Vec<Line<'static>> {
    vec![
        file_row("▣", "picture", artifacts.picture()),
        file_row("▣", "scene  ", artifacts.scene()),
        file_row("♪", "sound  ", artifacts.sound()),
    ]
}

fn file_row(glyph: &str, name: &str, slot: &ArtifactSlot) -> Line<'static> {
    if let Some(file) = slot.file() {
        return Line::from(vec![
            Span::raw(EXPAND_INDENT),
            Span::styled(String::from(glyph), palette::dim()),
            Span::raw(" "),
            Span::styled(String::from(name), palette::base()),
            Span::raw("   "),
            Span::styled(String::from(file.name()), palette::link()),
            Span::raw("   "),
            Span::styled(format!("· {}", file.size()), palette::dim()),
        ]);
    }
    file_status_row(glyph, name, artifact_state(slot))
}

fn file_status_row(glyph: &str, name: &str, status: String) -> Line<'static> {
    Line::from(vec![
        Span::raw(EXPAND_INDENT),
        Span::styled(String::from(glyph), palette::dim()),
        Span::raw(" "),
        Span::styled(String::from(name), palette::base()),
        Span::raw("   "),
        Span::styled(status, palette::dim()),
    ])
}

fn artifact_state(slot: &ArtifactSlot) -> String {
    if slot.ready() {
        return String::from("ready");
    }
    if slot.discarded() {
        return String::from("discarded");
    }
    if slot.failed_terminally() {
        return format!("failed after {} tries", slot.tally().ceiling());
    }
    if slot.tally().done() > 0 {
        return format!(
            "retrying {}/{}",
            slot.tally().done(),
            slot.tally().ceiling()
        );
    }
    String::from("queued")
}

fn failure_banner(width: u16) -> Vec<Line<'static>> {
    let box_width = width.clamp(40, 78) as usize;
    let inner = box_width.saturating_sub(2);
    let top = format!("╔{}╗", "═".repeat(inner));
    let blank = format!("║{}║", " ".repeat(inner));
    let bottom = format!("╚{}╝", "═".repeat(inner));
    let pad_line = |text: &str, dim: bool| -> Line<'static> {
        let visible = text.chars().count();
        let padding = inner.saturating_sub(visible);
        let style = if dim { palette::dim() } else { palette::base() };
        Line::from(vec![
            Span::styled("║", palette::base()),
            Span::styled(String::from(text), style),
            Span::styled(" ".repeat(padding), palette::base()),
            Span::styled("║", palette::base()),
        ])
    };
    let keys_line: Line<'static> = {
        let left_key = "[r]";
        let left_label = " regenerate failed";
        let right_key = "[Enter]";
        let right_label = " keep going anyway";
        let gap = "        ";
        let prefix = "   ";
        let visible = prefix.chars().count()
            + left_key.chars().count()
            + left_label.chars().count()
            + gap.chars().count()
            + right_key.chars().count()
            + right_label.chars().count();
        let padding = inner.saturating_sub(visible);
        Line::from(vec![
            Span::styled("║", palette::base()),
            Span::raw(prefix),
            Span::styled(left_key, palette::key()),
            Span::styled(left_label, palette::base()),
            Span::raw(gap),
            Span::styled(right_key, palette::key()),
            Span::styled(right_label, palette::base()),
            Span::styled(" ".repeat(padding), palette::base()),
            Span::styled("║", palette::base()),
        ])
    };
    vec![
        Line::from(Span::styled(top, palette::base())),
        pad_line("   1 card couldn't finish after 3 tries.", false),
        pad_line("   the rest is fine — cached, will be free to retry.", true),
        Line::from(Span::styled(blank, palette::base())),
        keys_line,
        Line::from(Span::styled(bottom, palette::base())),
        Line::from(""),
        Line::from(""),
    ]
}

fn status_panel(app: &App) -> Paragraph<'_> {
    let ready = app.cards_ready();
    let total = app.cards().len();
    let failed = app.cards_failed();
    let retrying = app
        .cards()
        .iter()
        .filter(|draft| working(draft.artifacts()))
        .count();
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{ready}/{total} ready"));
    if failed > 0 {
        parts.push(format!("{failed} couldn't finish"));
    }
    if retrying > 0 {
        parts.push(format!("retrying {retrying}"));
    }
    let cache_hits = cache_hits(app);
    if retrying == 0 && failed == 0 && cache_hits > 0 {
        parts.push(format!("cache hits {cache_hits}"));
    }
    parts.push(format!("elapsed {}", elapsed(app)));
    Paragraph::new(Line::from(Span::styled(parts.join(" · "), palette::base())))
        .style(palette::base())
}

fn cache_hits(app: &App) -> usize {
    app.cards()
        .iter()
        .map(|draft| {
            [
                draft.artifacts().scene(),
                draft.artifacts().picture(),
                draft.artifacts().sound(),
            ]
            .iter()
            .filter(|slot| slot.file().is_some_and(|file| file.cached()))
            .count()
        })
        .sum()
}

fn elapsed(app: &App) -> String {
    let seconds = app.elapsed().as_secs();
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    format!("{minutes:02}:{remainder:02}")
}
