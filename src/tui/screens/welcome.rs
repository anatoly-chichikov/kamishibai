//! First-run Welcome screen.
//!
//! Two stages walked in sequence: pick the user's language, then enter a Gemini
//! API key. The key step is a small focusable form — the masked key input plus
//! a `submit` chip and, when `GEMINI_API_KEY` is present, a `load from env`
//! chip. Focus moves with the arrow keys and the focused control is drawn with
//! the same inverted block as the active language chip. `submit` hands the key
//! to the shell for a live validity check; the result lands back as a notice
//! beside the buttons (outside the input).

use std::borrow::Cow;
use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use super::language_grid::LanguageGrid;
use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::picker::PickerSection;
use crate::tui::screen::{WelcomeFocus, WelcomeStage};
use crate::tui::text_field::TextField;

const INTRO: &str = "kamishibai turns a list of words you want to learn into an anki deck plus a printable pdf. for each word it writes a natural example sentence, illustrates the scene as a manga panel, and reads it aloud in a natural, native-speaker voice.";
const HEADLINE: &str = "kamishibai";
const HINT: &str = "set up two things";
const SUBMIT_LABEL: &str = "submit";
const LOAD_ENV_LABEL: &str = "load from env";
/// Gap between the key field and the notice printed to its right.
const TRAILING_GAP: u16 = 3;
/// Column where the key value, helper line, and button row all start:
/// `"02  "` (4) + the 16-wide label + the 2-cell focus-caret gutter.
const FIELD_INDENT: u16 = 4 + 16 + 2;
/// Fixed width of the underlined key input field, matching the mask cap.
const KEY_FIELD_WIDTH: u16 = 39;
const KEY_PLACEHOLDER: &str = "paste your key [Cmd+V]";

/// `ScreenView` handle for the first-run Welcome screen. Skips the language
/// chip — the language pair is not yet locked in at this point.
pub struct Welcome;

impl ScreenView for Welcome {
    fn title(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HEADLINE)
    }

    fn hint(&self, _: &App) -> Cow<'static, str> {
        Cow::Borrowed(HINT)
    }

    fn lang_chip(&self, _: &App) -> Option<Vec<Span<'static>>> {
        None
    }

    fn status(&self, app: &App) -> Vec<Span<'static>> {
        status(app)
    }

    fn hints(&self, app: &App) -> Vec<super::common::FooterHint> {
        hints(app)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        let rows = body_rows(app, area);
        frame.render_widget(intro(area.width), rows[0]);
        frame.render_widget(language_row(app, area), rows[2]);
        let (input_line, notice_line) = input_lines(app, rows[4].width);
        frame.render_widget(input_line, rows[4]);
        frame.render_widget(key_underline_row(app), rows[5]);
        frame.render_widget(notice_line, rows[6]);
        frame.render_widget(buttons_line(app), rows[7]);
        place_key_cursor(frame, app, rows[4]);
    }
}

/// Return which language one terminal cell lands on, if any. Drives both the
/// hand-pointer policy and click dispatch on the language step.
pub fn language_at(app: &App, area: Rect, x: u16, y: u16) -> Option<usize> {
    if app.welcome().stage != WelcomeStage::PickLanguage {
        return None;
    }
    let body = super::common::frame_rects(area).body;
    let rows = body_rows(app, body);
    let origin = Rect {
        x: body.x + FIELD_INDENT,
        y: rows[2].y,
        width: rows[2].width.saturating_sub(FIELD_INDENT),
        height: rows[2].height,
    };
    if y < origin.y || y >= origin.y + origin.height {
        return None;
    }
    language_grid(body).language_at(origin, x, y)
}

/// Return which key-step control one terminal cell lands on, if any. Drives
/// both the hand-pointer policy and click dispatch in the shell.
pub fn control_at(app: &App, area: Rect, x: u16, y: u16) -> Option<WelcomeFocus> {
    if app.welcome().stage != WelcomeStage::EnterKey {
        return None;
    }
    let frame = super::common::frame_rects(area);
    let rows = body_rows(app, frame.body);
    let base = frame.body.x;
    if y == rows[7].y {
        let submit_start = base + FIELD_INDENT;
        let submit_end = submit_start + chip_width(SUBMIT_LABEL);
        if x >= submit_start && x < submit_end {
            return Some(WelcomeFocus::Submit);
        }
        if app.welcome().env_available {
            let env_start = submit_end + 3;
            let env_end = env_start + chip_width(LOAD_ENV_LABEL);
            if x >= env_start && x < env_end {
                return Some(WelcomeFocus::LoadEnv);
            }
        }
    }
    None
}

/// Rows the language step must leave to the rest of the body: the intro and its
/// blank above, one blank below, and the `02 gemini api key` label under that.
/// Everything else the form draws stays blank until the key step, so the grid
/// is free to grow into it.
const LANGUAGE_ROOM_RESERVE: u16 = 5;

/// Return the rows the language step occupies.
///
/// The grid is offered while the step is being answered; once it has been, the
/// step collapses to the one language that was chosen, the way a filled-in form
/// field stops showing its options.
fn language_height(app: &App, area: Rect) -> u16 {
    if app.welcome().stage != WelcomeStage::PickLanguage {
        return 1;
    }
    u16::try_from(language_grid(area).lines()).unwrap_or(u16::MAX)
}

fn body_rows(app: &App, area: Rect) -> Rc<[Rect]> {
    let language_height = language_height(app, area);
    let language_gap = u16::from(language_height <= 1);
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),               // intro
            Constraint::Length(1),               // blank
            Constraint::Length(language_height), // 01 · language
            Constraint::Length(language_gap),    // blank
            Constraint::Length(1),               // 02 · key field
            Constraint::Length(1),               // input underline
            Constraint::Length(1),               // blank · notice wraps here on a narrow terminal
            Constraint::Length(1),               // buttons
            Constraint::Min(0),
        ])
        .split(area)
}

fn intro(width: u16) -> Paragraph<'static> {
    let max_chars = width.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current = String::new();
    for word in INTRO.split_whitespace() {
        let candidate = if current.is_empty() {
            String::from(word)
        } else {
            format!("{current} {word}")
        };
        if candidate.chars().count() > max_chars && !current.is_empty() {
            lines.push(Line::from(Span::styled(
                current.clone(),
                palette::Ink::Detail.on(false),
            )));
            current = String::from(word);
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(Span::styled(
            current,
            palette::Ink::Detail.on(false),
        )));
    }
    Paragraph::new(lines).style(palette::base())
}

/// Return the grid the language step lays its languages out on.
///
/// The grid gets the body minus its own indent across, and minus the rows the
/// rest of the form needs down — so it grows into the room a big terminal has
/// and gives that room back on a small one instead of pushing the key field
/// off the screen.
pub fn language_grid(area: Rect) -> LanguageGrid {
    LanguageGrid::measured(
        area.width.saturating_sub(FIELD_INDENT),
        area.height.saturating_sub(LANGUAGE_ROOM_RESERVE).max(1),
    )
}

/// Return the language the step currently has picked.
pub fn picked_language(app: &App) -> usize {
    PickerSection::Known.chip_for(app.pair().known())
}

fn language_row(app: &App, area: Rect) -> Paragraph<'static> {
    Paragraph::new(language_lines(app, area)).style(palette::base())
}

fn language_lines(app: &App, area: Rect) -> Vec<Line<'static>> {
    let active = app.welcome().stage == WelcomeStage::PickLanguage;
    let picked = picked_language(app);
    if !active {
        let mut spans = step_label(false);
        spans.push(Span::styled(
            PickerSection::Known.row_text(picked),
            palette::Ink::Detail.on(false),
        ));
        return vec![Line::from(spans)];
    }
    let grid = language_grid(area);
    (0..grid.lines())
        .map(|line| {
            let mut spans = if line == 0 {
                step_label(active)
            } else {
                vec![Span::styled(
                    " ".repeat(usize::from(FIELD_INDENT)),
                    palette::base(),
                )]
            };
            spans.extend(grid.line(line, picked));
            Line::from(spans)
        })
        .collect()
}

/// The `01  your language  ›` label that opens the step's first line.
fn step_label(active: bool) -> Vec<Span<'static>> {
    let num_style = if active {
        palette::base()
    } else {
        palette::Ink::Aside.on(false)
    };
    let label_style = if active {
        palette::base()
    } else {
        palette::Ink::Detail.on(false)
    };
    vec![
        Span::styled("01  ", num_style),
        Span::styled(super::common::pad_right("your language", 16), label_style),
        chevron(active),
    ]
}

/// Build the `02 gemini api key` row spans and the visual width of its field.
fn input_row(app: &App) -> (Vec<Span<'static>>, u16) {
    let welcome = app.welcome();
    let active = welcome.stage == WelcomeStage::EnterKey;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let num_style = if active {
        palette::base()
    } else {
        palette::Ink::Aside.on(false)
    };
    spans.push(Span::styled("02  ", num_style));
    let label_style = if active || !welcome.key.is_empty() {
        palette::base()
    } else {
        palette::Ink::Detail.on(false)
    };
    spans.push(Span::styled(
        super::common::pad_right("gemini api key", 16),
        label_style,
    ));
    spans.push(chevron(active));
    if !active {
        if !welcome.key.is_empty() {
            spans.push(Span::styled(
                masked(welcome.key.as_str()),
                palette::Ink::Detail.on(false),
            ));
        }
        return (spans, 0);
    }
    let field = key_field(app);
    let width = field.display_width();
    spans.extend(field.spans());
    (spans, width)
}

/// Build the input row (02) and the row under it. The submit notice (`key
/// invalid`, `enter a key first`, …) sits to the right of the input field, and
/// drops to the row under the underline when the terminal is too narrow.
fn input_lines(app: &App, width: u16) -> (Paragraph<'static>, Paragraph<'static>) {
    let (mut spans, value_width) = input_row(app);
    let notice = if app.welcome().stage == WelcomeStage::EnterKey {
        app.welcome().notice.clone()
    } else {
        None
    };
    let Some(notice) = notice else {
        return (
            Paragraph::new(Line::from(spans)).style(palette::base()),
            Paragraph::new("").style(palette::base()),
        );
    };
    let notice_width = u16::try_from(notice.chars().count()).unwrap_or(u16::MAX);
    let notice_span = Span::styled(notice, palette::Ink::Subject.on(false));
    if FIELD_INDENT + KEY_FIELD_WIDTH + TRAILING_GAP + notice_width <= width {
        let pad = usize::from(KEY_FIELD_WIDTH.saturating_sub(value_width) + TRAILING_GAP);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(notice_span);
        (
            Paragraph::new(Line::from(spans)).style(palette::base()),
            Paragraph::new("").style(palette::base()),
        )
    } else {
        let below = vec![
            Span::raw(" ".repeat(usize::from(FIELD_INDENT))),
            notice_span,
        ];
        (
            Paragraph::new(Line::from(spans)).style(palette::base()),
            Paragraph::new(Line::from(below)).style(palette::base()),
        )
    }
}

/// Build the buttons row, indented to sit under the key field. Blank until the
/// key step.
fn buttons_line(app: &App) -> Paragraph<'static> {
    if app.welcome().stage != WelcomeStage::EnterKey {
        return Paragraph::new("").style(palette::base());
    }
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(usize::from(FIELD_INDENT)))];
    spans.extend(button_spans(app));
    Paragraph::new(Line::from(spans)).style(palette::base())
}

/// The submit chip plus the env loader chip when env can supply a key.
fn button_spans(app: &App) -> Vec<Span<'static>> {
    let welcome = app.welcome();
    let mut spans = vec![chip(SUBMIT_LABEL, welcome.focus == WelcomeFocus::Submit)];
    if welcome.env_available {
        spans.push(Span::raw("   "));
        spans.push(chip(LOAD_ENV_LABEL, welcome.focus == WelcomeFocus::LoadEnv));
    }
    spans
}

fn chip(label: &str, focused: bool) -> Span<'static> {
    let text = format!(" {label} ");
    if focused {
        Span::styled(text, palette::invert())
    } else {
        Span::styled(text, palette::Ink::Detail.on(false))
    }
}

fn chip_width(label: &str) -> u16 {
    u16::try_from(label.chars().count() + 2).unwrap_or(u16::MAX)
}

/// The 2-cell focus caret shown after a step's label: a bright `›` on the
/// active row, blank on the inactive one, so both rows stay aligned.
fn chevron(active: bool) -> Span<'static> {
    if active {
        Span::styled("› ", palette::Ink::Subject.on(false))
    } else {
        Span::styled("  ", palette::base())
    }
}

/// Solid rule drawn on the row below the key field so it reads as a text
/// input. Only on the key step; blank otherwise.
fn key_underline_row(app: &App) -> Paragraph<'static> {
    if app.welcome().stage != WelcomeStage::EnterKey {
        return Paragraph::new("").style(palette::base());
    }
    let indent = " ".repeat(usize::from(FIELD_INDENT));
    Paragraph::new(Line::from(vec![
        Span::raw(indent),
        Span::styled(
            "─".repeat(usize::from(KEY_FIELD_WIDTH)),
            palette::Ink::Detail.on(false),
        ),
    ]))
    .style(palette::base())
}

fn masked(key: &str) -> String {
    "•".repeat(key.chars().count().min(39))
}

fn key_field(app: &App) -> TextField<'static> {
    if app.welcome().key.is_empty() {
        TextField::new("", KEY_PLACEHOLDER)
    } else {
        TextField::new(masked(app.welcome().key.as_str()), KEY_PLACEHOLDER)
    }
}

fn place_key_cursor(frame: &mut Frame, app: &App, row: Rect) {
    if app.welcome().stage != WelcomeStage::EnterKey {
        return;
    }
    let cursor = key_field(app).cursor_offset();
    let cursor_x = row.x + FIELD_INDENT + cursor.min(KEY_FIELD_WIDTH.saturating_sub(1));
    frame.set_cursor_position((cursor_x, row.y));
}

fn status(app: &App) -> Vec<Span<'static>> {
    let counter = match app.welcome().stage {
        WelcomeStage::PickLanguage => "step 1/2",
        WelcomeStage::EnterKey => "step 2/2",
    };
    vec![Span::styled(
        String::from(counter),
        palette::Ink::Aside.on(false),
    )]
}

fn hints(app: &App) -> Vec<super::common::FooterHint> {
    let welcome = app.welcome();
    let mut hints: Vec<super::common::FooterHint> = Vec::new();
    match welcome.stage {
        WelcomeStage::PickLanguage => {
            hints.push(super::common::FooterHint::primary("Enter", "next"));
            hints.push(super::common::FooterHint::secondary("↑ ↓ ← →", "language"));
        }
        WelcomeStage::EnterKey => {
            hints.push(super::common::FooterHint::primary("Enter", "submit"));
            if welcome.env_available {
                hints.push(super::common::FooterHint::secondary("← →", "move"));
            }
            hints.push(super::common::FooterHint::secondary("Esc", "back"));
        }
    }
    hints.push(super::common::quit_hint(app.quit_pending()));
    hints
}
