//! Renderer for the sense-check screen.
//!
//! This is step one of two between raw input and expensive card generation.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::session::{MetaSegment, MetaTone, WordCandidate};
use crate::tui::app::App;
use crate::tui::palette;

const GUTTER: u16 = 4;
const TARGET_PENDING: &str = "...";

/// Draw the sense-check screen for the current `App`.
pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let width = area.width.saturating_sub(GUTTER * 2);
    frame.render_widget(top_bar(app, width), inset(rows[0]));
    frame.render_widget(question_bar(app, width), inset(rows[1]));
    frame.render_widget(rule(width), inset(rows[2]));
    frame.render_widget(body(app), inset(rows[3]));
    frame.render_widget(footer_primary(app), inset(rows[4]));
    frame.render_widget(footer_secondary(app, width), inset(rows[5]));
}

fn top_bar(app: &App, width: u16) -> Paragraph<'static> {
    let copy = copy(app);
    split_line(direction(app), copy.step, width, false)
}

fn question_bar(app: &App, width: u16) -> Paragraph<'static> {
    let copy = copy(app);
    split_line(copy.question, copy.subtitle, width, true)
}

fn split_line(
    left: impl Into<String>,
    right: impl Into<String>,
    width: u16,
    bold: bool,
) -> Paragraph<'static> {
    let left = left.into();
    let right = right.into();
    let gap = (width as usize).saturating_sub(left.chars().count() + right.chars().count());
    let left_style = if bold {
        palette::base().add_modifier(Modifier::BOLD)
    } else {
        palette::base()
    };
    Paragraph::new(Line::from(vec![
        Span::styled(left, left_style),
        Span::styled(" ".repeat(gap), palette::base()),
        Span::styled(right, palette::base()),
    ]))
    .style(palette::base())
}

fn rule(width: u16) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "─".repeat(width as usize),
        palette::dim(),
    )))
    .style(palette::base())
}

fn body(app: &App) -> Paragraph<'_> {
    if app.candidates().is_empty() {
        let copy = copy(app);
        let message = if app.target_pending() {
            copy.pending
        } else {
            copy.empty
        };
        return Paragraph::new(Line::from(Span::styled(message, palette::dim())))
            .style(palette::base());
    }
    let term_width = padded_width(app.candidates(), |candidate| candidate.term(), 12);
    let preview_width = padded_width(app.candidates(), |candidate| candidate.preview(), 14);
    let lines = app
        .candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            candidate_line(index, candidate, app.selected(), term_width, preview_width)
        })
        .collect::<Vec<_>>();
    Paragraph::new(lines).style(palette::base())
}

fn candidate_line<'a>(
    index: usize,
    candidate: &'a WordCandidate,
    selected: usize,
    term_width: usize,
    preview_width: usize,
) -> Line<'a> {
    let mut spans = vec![
        Span::styled(
            format!("{:>2}.", index + 1),
            row_number_style(index == selected),
        ),
        Span::raw(" "),
        Span::styled(pad(candidate.term(), term_width), palette::base()),
        Span::raw("  "),
        Span::styled(pad(candidate.preview(), preview_width), palette::base()),
        Span::raw("  "),
    ];
    spans.extend(meta_spans(candidate.meta().segments()));
    Line::from(spans)
}

fn meta_spans(segments: &[MetaSegment]) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", palette::dim()));
        }
        spans.push(Span::styled(segment.text(), meta_style(segment.tone())));
    }
    spans
}

fn meta_style(tone: MetaTone) -> Style {
    match tone {
        MetaTone::Dim => palette::dim(),
        MetaTone::Bright => palette::base().add_modifier(Modifier::BOLD),
    }
}

fn row_number_style(selected: bool) -> Style {
    if selected {
        return palette::base();
    }
    palette::dim()
}

fn footer_primary(app: &App) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        copy(app).footer_primary,
        palette::key(),
    )))
    .style(palette::base())
}

fn footer_secondary(app: &App, width: u16) -> Paragraph<'static> {
    let text = (copy(app).footer_secondary)(
        app.candidates()
            .iter()
            .filter(|candidate| candidate.included())
            .count(),
    );
    let pad = (width as usize).saturating_sub(text.chars().count());
    Paragraph::new(Line::from(vec![
        Span::styled(" ".repeat(pad), palette::base()),
        Span::styled(text, palette::key()),
    ]))
    .style(palette::base())
}

fn direction(app: &App) -> String {
    let target = if app.target_pending() {
        String::from(TARGET_PENDING)
    } else {
        app.pair().target().to_uppercase()
    };
    format!(
        "kamishibai · {target} → {}",
        app.pair().support().to_uppercase()
    )
}

fn padded_width<F>(candidates: &[WordCandidate], value: F, minimum: usize) -> usize
where
    F: Fn(&WordCandidate) -> &str,
{
    candidates
        .iter()
        .map(value)
        .map(|item| item.chars().count())
        .max()
        .unwrap_or(minimum)
        .max(minimum)
}

fn pad(value: &str, width: usize) -> String {
    let mut text = String::from(value);
    let gap = width.saturating_sub(value.chars().count());
    text.push_str(" ".repeat(gap).as_str());
    text
}

fn inset(area: Rect) -> Rect {
    let clamp = GUTTER.min(area.width / 2);
    Rect {
        x: area.x + clamp,
        y: area.y,
        width: area.width.saturating_sub(clamp * 2),
        height: area.height,
    }
}

fn copy(app: &App) -> SenseCopy {
    match app.pair().support() {
        "ru" => SenseCopy {
            step: "шаг 1 из 2 · проверка смысла",
            question: "Я правильно понял эти слова?",
            subtitle: "поправь до того, как я сгенерирую карточки",
            pending: "понимаю твои слова...",
            empty: "не осталось слов для проверки",
            footer_primary: "[↑↓] навигация · [d] удалить · [R] поправить · [L] мой язык",
            footer_secondary: russian_footer,
        },
        "de" => SenseCopy {
            step: "Schritt 1 von 2 · Bedeutungsprüfung",
            question: "Habe ich diese Wörter richtig verstanden?",
            subtitle: "korrigiere sie, bevor ich Karten generiere",
            pending: "ich verstehe deine Wörter...",
            empty: "keine Wörter mehr zu prüfen",
            footer_primary: "[↑↓] Navigation · [d] löschen · [R] korrigieren · [L] meine Sprache",
            footer_secondary: german_footer,
        },
        "el" => SenseCopy {
            step: "βήμα 1 από 2 · έλεγχος νοήματος",
            question: "Κατάλαβα σωστά αυτές τις λέξεις;",
            subtitle: "διόρθωσέ τες πριν δημιουργήσω κάρτες",
            pending: "καταλαβαίνω τις λέξεις σου...",
            empty: "δεν έμειναν λέξεις για έλεγχο",
            footer_primary: "[↑↓] πλοήγηση · [d] διαγραφή · [R] διόρθωση · [L] γλώσσα μου",
            footer_secondary: greek_footer,
        },
        "es" => SenseCopy {
            step: "paso 1 de 2 · revisión del sentido",
            question: "¿Entendí bien estas palabras?",
            subtitle: "corrige antes de que genere las tarjetas",
            pending: "entendiendo tus palabras...",
            empty: "no quedan palabras por revisar",
            footer_primary: "[↑↓] navegación · [d] eliminar · [R] corregir · [L] mi idioma",
            footer_secondary: spanish_footer,
        },
        "zh" => SenseCopy {
            step: "第 1 步，共 2 步 · 意义检查",
            question: "我正确理解这些词了吗？",
            subtitle: "在我生成卡片前先修改",
            pending: "正在理解你的词...",
            empty: "没有剩余词条需要检查",
            footer_primary: "[↑↓] 导航 · [d] 删除 · [R] 修改 · [L] 我的语言",
            footer_secondary: chinese_footer,
        },
        _ => SenseCopy {
            step: "step 1 of 2 · sense check",
            question: "Did I understand these words correctly?",
            subtitle: "change them before I generate cards",
            pending: "understanding your words...",
            empty: "nothing left to review",
            footer_primary: "[↑↓] navigate · [d] delete · [R] change · [L] my language",
            footer_secondary: english_footer,
        },
    }
}

fn english_footer(cards: usize) -> String {
    let noun = if cards == 1 { "card" } else { "cards" };
    format!("[T] change target · [Enter] generate {cards} {noun}")
}

fn german_footer(cards: usize) -> String {
    let noun = if cards == 1 { "Karte" } else { "Karten" };
    format!("[T] target wechseln · [Enter] {cards} {noun} generieren")
}

fn greek_footer(cards: usize) -> String {
    let noun = if cards == 1 {
        "κάρτα"
    } else {
        "κάρτες"
    };
    format!("[T] αλλαγή target · [Enter] δημιουργία {cards} {noun}")
}

fn russian_footer(cards: usize) -> String {
    format!(
        "[T] сменить target · [Enter] сгенерировать {cards} {}",
        russian_card_word(cards)
    )
}

fn spanish_footer(cards: usize) -> String {
    let noun = if cards == 1 { "tarjeta" } else { "tarjetas" };
    format!("[T] cambiar target · [Enter] generar {cards} {noun}")
}

fn chinese_footer(cards: usize) -> String {
    format!("[T] 切换 target · [Enter] 生成 {cards} 张卡片")
}

fn russian_card_word(cards: usize) -> &'static str {
    let last_two = cards % 100;
    let last = cards % 10;
    if (11..=14).contains(&last_two) {
        return "карточек";
    }
    match last {
        1 => "карточку",
        2..=4 => "карточки",
        _ => "карточек",
    }
}

struct SenseCopy {
    step: &'static str,
    question: &'static str,
    subtitle: &'static str,
    pending: &'static str,
    empty: &'static str,
    footer_primary: &'static str,
    footer_secondary: fn(usize) -> String,
}
