//! Renderer for the `Your cards` screen.
//!
//! Anchors on `docs/tui-states/current-pdf/04-your-cards.png` and also covers
//! the retry (`06-your-cards-retrying.png`) and failure (`07-your-cards-
//! couldnt-finish.png`) variants — they are inline states inside this screen.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::session::{Artifact, ArtifactSlot, CardArtifacts, CardDraft};
use crate::tui::app::App;

const HEADLINE: &str = "Your cards";
const HINT_LEFT: &str = "единственный рабочий экран. Артефактов всегда три: scene · picture · sound. Видно кто где прямо сейчас.";
const HINT_RIGHT: &str = "[↑↓] nav · [Enter] expand · [R] change this card · [d] drop artifact";

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(language_badge(app), areas[0]);
    let tagline = status_headline(app);
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title_top(Line::from(Span::styled(
            format!(" {HEADLINE} "),
            Style::default().add_modifier(Modifier::BOLD),
        )))
        .title_top(
            Line::from(Span::styled(
                format!(" {tagline} "),
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Right),
        );
    let inner = outer.inner(areas[1]);
    frame.render_widget(outer, areas[1]);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(section_header("cards"), sections[0]);
    frame.render_widget(cards_panel(app), sections[1]);
    frame.render_widget(section_header("status"), sections[2]);
    frame.render_widget(status_panel(app), sections[3]);
    frame.render_widget(divider(sections[4].width), sections[4]);
    frame.render_widget(footer(sections[5].width), sections[5]);
}

fn language_badge(app: &App) -> Paragraph<'_> {
    let target = app.pair().target().to_uppercase();
    let support = app.pair().support().to_uppercase();
    Paragraph::new(Line::from(Span::styled(
        format!("kamishibai · {target} → {support}"),
        Style::default().fg(Color::DarkGray),
    )))
}

fn status_headline(app: &App) -> String {
    format!(
        "group 1 of 1 · {}/{} ready",
        app.cards_ready(),
        app.cards().len(),
    )
}

fn section_header(label: &str) -> Paragraph<'_> {
    Paragraph::new(Line::from(Span::styled(
        format!("  {label} ─"),
        Style::default().fg(Color::DarkGray),
    )))
    .style(Style::default().bg(Color::Black))
}

fn cards_panel(app: &App) -> Paragraph<'_> {
    if app.cards().is_empty() {
        return Paragraph::new(Line::from(Span::styled(
            "  preparing cards…",
            Style::default().fg(Color::Gray),
        )))
        .style(Style::default().bg(Color::Black));
    }
    let mut lines: Vec<Line<'_>> = Vec::new();
    for (index, draft) in app.cards().iter().enumerate() {
        let focused = index == app.card_selected();
        let glyph = row_marker(draft, focused && app.card_expanded());
        let row = format!(
            "  {glyph} {number:>2}. {term:<12}  {scene}   {picture}   {sound}",
            glyph = glyph,
            number = index + 1,
            term = draft.term(),
            scene = artifact_label("scene", draft.artifacts().scene()),
            picture = artifact_label("picture", draft.artifacts().picture()),
            sound = artifact_label("sound", draft.artifacts().sound()),
        );
        let style = if focused {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(row, style)));
        if focused && app.card_expanded() {
            lines.extend(expanded_rows(draft));
        }
    }
    Paragraph::new(lines).style(Style::default().bg(Color::Black).fg(Color::White))
}

fn row_marker(draft: &CardDraft, expanded: bool) -> &'static str {
    if draft.artifacts().all_ready() {
        if expanded {
            return "▾";
        }
        return "▸";
    }
    if draft.artifacts().has_failed() {
        return "✗";
    }
    if working(draft.artifacts()) {
        return "●";
    }
    "○"
}

fn artifact_label(name: &str, slot: &ArtifactSlot) -> String {
    if slot.ready() {
        return format!("✓ {name}");
    }
    if slot.failed_terminally() {
        return format!("✗ {name}");
    }
    if slot.tally().done() > 0 {
        return format!("● {name} (retrying {}/3)", slot.tally().done());
    }
    format!("○ {name}")
}

fn working(artifacts: &CardArtifacts) -> bool {
    [artifacts.scene(), artifacts.picture(), artifacts.sound()]
        .iter()
        .any(|slot| slot.tally().done() > 0 && !slot.ready() && !slot.failed_terminally())
}

fn expanded_rows(draft: &CardDraft) -> Vec<Line<'_>> {
    let mut rows: Vec<Line<'_>> = Vec::new();
    rows.push(Line::from(Span::styled(
        "       — front",
        Style::default().fg(Color::DarkGray),
    )));
    rows.push(Line::from(Span::styled(
        format!("       {}", draft.payload().front()),
        Style::default().fg(Color::White),
    )));
    rows.push(Line::from(Span::styled(
        "       — back",
        Style::default().fg(Color::DarkGray),
    )));
    rows.push(Line::from(Span::styled(
        format!("       {}", draft.payload().back()),
        Style::default().fg(Color::White),
    )));
    rows.push(Line::from(Span::styled(
        "       — change this card",
        Style::default().fg(Color::DarkGray),
    )));
    rows.push(Line::from(Span::styled(
        "       [R] change this card    [d] drop picture / scene / sound",
        Style::default().fg(Color::Gray),
    )));
    rows
}

fn status_panel(app: &App) -> Paragraph<'_> {
    let ready = app.cards_ready();
    let total = app.cards().len();
    let failed = app.cards_failed();
    let failure_note = if failed > 0 {
        format!(" · {failed} couldn't finish — [F] regenerate failed")
    } else {
        String::new()
    };
    let line1 = format!("  {ready}/{total} ready{failure_note}");
    let line2 = format!(
        "  artifacts: scene={} · picture={} · sound={}",
        app.card_artifact_hint(Artifact::Scene),
        app.card_artifact_hint(Artifact::Picture),
        app.card_artifact_hint(Artifact::Sound),
    );
    let style = if failed > 0 {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Paragraph::new(vec![
        Line::from(Span::styled(line1, style)),
        Line::from(Span::styled(line2, Style::default().fg(Color::Gray))),
    ])
    .style(Style::default().bg(Color::Black).fg(Color::White))
}

fn divider(width: u16) -> Paragraph<'static> {
    let dashes = "-".repeat(width as usize);
    Paragraph::new(Line::from(Span::styled(
        dashes,
        Style::default().fg(Color::DarkGray),
    )))
}

fn footer(width: u16) -> Paragraph<'static> {
    let left = HINT_LEFT;
    let right = HINT_RIGHT;
    let total = left.chars().count() + right.chars().count();
    let gap = (width as usize).saturating_sub(total);
    let padding = " ".repeat(gap);
    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::Gray)),
        Span::raw(padding),
        Span::styled(
            right,
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(line)
}
