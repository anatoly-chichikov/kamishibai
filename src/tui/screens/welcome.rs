//! First-run Welcome screen.
//!
//! Mirrors `kamishibai-simple/project/step-welcome.jsx`. Two stages walked in
//! sequence: pick the user's language, then paste a Gemini API key. The
//! status line under the key reflects where the key came from (env, prior
//! session, fresh paste, or nothing yet).

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::ScreenView;
use crate::languages::catalog;
use crate::tui::app::App;
use crate::tui::palette;
use crate::tui::screen::{KeySource, WelcomeStage};

const INTRO: &str = "kamishibai turns a list of words you want to learn into an anki deck built for retention: for each word it writes a natural example sentence, illustrates the scene as a manga panel, and reads it aloud in a studio-grade voice.";
const KEY_URL: &str = "aistudio.google.com/apikey";
const VALID_KEY_LENGTH: usize = 20;
const HEADLINE: &str = "kamishibai";
const HINT: &str = "set up two things";

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

    fn footer(&self, app: &App, width: u16) -> Paragraph<'static> {
        footer(app, width)
    }

    fn body(&self, frame: &mut Frame, area: Rect, app: &App) {
        let body_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(area);
        frame.render_widget(intro(area.width), body_rows[0]);
        frame.render_widget(language_row(app), body_rows[2]);
        frame.render_widget(api_row(app), body_rows[3]);
        frame.render_widget(status_row(app), body_rows[4]);
        frame.render_widget(help_row(app), body_rows[5]);
    }
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
            lines.push(Line::from(Span::styled(current.clone(), palette::dim())));
            current = String::from(word);
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(Span::styled(current, palette::dim())));
    }
    Paragraph::new(lines).style(palette::base())
}

fn language_row(app: &App) -> Paragraph<'static> {
    let stage = app.welcome().stage;
    let active = stage == WelcomeStage::PickLanguage;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let num_style = if active {
        palette::base()
    } else {
        palette::dim2()
    };
    spans.push(Span::styled("01  ", num_style));
    let label_style = if active {
        palette::base()
    } else {
        palette::dim()
    };
    spans.push(Span::styled(
        super::common::pad_right("your language", 16),
        label_style,
    ));
    spans.push(Span::raw(" "));
    let current = app.pair().support().to_ascii_lowercase();
    for code in catalog().codes() {
        let label = code.to_ascii_uppercase();
        let is_active = code == current;
        let chip = if is_active {
            Span::styled(format!(" {label} "), palette::invert())
        } else {
            Span::styled(format!(" {label} "), palette::dim())
        };
        spans.push(chip);
        spans.push(Span::raw(" "));
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
}

fn api_row(app: &App) -> Paragraph<'static> {
    let stage = app.welcome().stage;
    let active = stage == WelcomeStage::EnterKey;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let num_style = if active {
        palette::base()
    } else if stage == WelcomeStage::PickLanguage {
        palette::dim2()
    } else {
        palette::dim()
    };
    spans.push(Span::styled("02  ", num_style));
    let label_style = if active || !app.welcome().key.is_empty() {
        palette::base()
    } else {
        palette::dim()
    };
    spans.push(Span::styled(
        super::common::pad_right("gemini api key", 16),
        label_style,
    ));
    spans.push(Span::raw(" "));
    if !app.welcome().key.is_empty() {
        let masked = "•".repeat(app.welcome().key.chars().count().min(39));
        spans.push(Span::styled(masked, palette::base()));
    } else if active {
        spans.push(Span::styled("Cmd+V to paste", palette::dim2()));
    } else {
        spans.push(Span::styled("after step 01", palette::dim2()));
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
}

fn status_row(app: &App) -> Paragraph<'static> {
    let stage = app.welcome().stage;
    if stage != WelcomeStage::EnterKey {
        return Paragraph::new("").style(palette::base());
    }
    let valid = app.welcome().key.chars().count() >= VALID_KEY_LENGTH;
    let indent = " ".repeat(4 + 16 + 1);
    let badge = |label: &str| -> Span<'static> {
        Span::styled(
            String::from(label),
            palette::base().add_modifier(Modifier::BOLD),
        )
    };
    let dim_badge = Span::styled("·", palette::dim2());
    let mut spans: Vec<Span<'static>> = vec![Span::raw(indent)];
    match (app.welcome().source, valid) {
        (KeySource::Env, true) => {
            spans.push(badge("env"));
            spans.push(Span::styled(
                "  found GEMINI_API_KEY · using it · Backspace to override",
                palette::base(),
            ));
        }
        (KeySource::Restored, true) => {
            spans.push(badge("saved"));
            spans.push(Span::styled(
                "  using your key from last time · Backspace to override",
                palette::base(),
            ));
        }
        (KeySource::Pasted, true) => {
            spans.push(badge("ok"));
            spans.push(Span::styled(
                format!(
                    "  {} chars · stays on this device",
                    app.welcome().key.chars().count()
                ),
                palette::base(),
            ));
        }
        (_, _) if !app.welcome().key.is_empty() => {
            spans.push(badge("…"));
            spans.push(Span::styled(
                "  short key — gemini keys are usually 39+ chars",
                palette::dim(),
            ));
        }
        _ => {
            spans.push(dim_badge);
            spans.push(Span::styled(
                "  no key found in env. paste one with Cmd+V or press ? to get one.",
                palette::dim(),
            ));
        }
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
}

fn help_row(app: &App) -> Paragraph<'static> {
    let stage = app.welcome().stage;
    let valid = app.welcome().key.chars().count() >= VALID_KEY_LENGTH;
    if stage != WelcomeStage::EnterKey || valid {
        return Paragraph::new("").style(palette::base());
    }
    let indent = " ".repeat(4 + 16 + 1);
    let line = Line::from(vec![
        Span::raw(indent),
        Span::styled("no key? get one free at ", palette::dim()),
        Span::styled(KEY_URL, palette::link()),
        Span::styled(
            " — stays on this device, only sent to gemini.",
            palette::dim(),
        ),
    ]);
    Paragraph::new(line).style(palette::base())
}

fn footer(app: &App, width: u16) -> Paragraph<'static> {
    let stage = app.welcome().stage;
    let valid = app.welcome().key.chars().count() >= VALID_KEY_LENGTH;
    let mut left: Vec<Span<'static>> = Vec::new();
    match stage {
        WelcomeStage::PickLanguage => {
            left.extend(super::common::key_hint("← →", "pick"));
            left.push(super::common::status_sep());
            left.extend(super::common::key_hint("Enter", "next"));
        }
        WelcomeStage::EnterKey => {
            if valid {
                left.extend(super::common::key_hint("Enter", "start"));
                left.push(super::common::status_sep());
                left.extend(super::common::key_hint("Esc", "back"));
                left.push(super::common::status_sep());
                let label = match app.welcome().source {
                    KeySource::Env | KeySource::Restored => "override",
                    _ => "clear",
                };
                left.extend(super::common::key_hint("Backspace", label));
            } else {
                left.extend(super::common::key_hint("Cmd+V", "paste key"));
                left.push(super::common::status_sep());
                left.extend(super::common::key_hint("?", "get one"));
                left.push(super::common::status_sep());
                left.extend(super::common::key_hint("Esc", "back"));
            }
        }
    }
    let counter = match stage {
        WelcomeStage::PickLanguage => "step 1 of 2",
        WelcomeStage::EnterKey => "step 2 of 2",
    };
    let mut right: Vec<Span<'static>> = vec![Span::styled(
        String::from(counter),
        palette::dim2().add_modifier(Modifier::DIM),
    )];
    super::common::append_quit(&mut right, app.quit_pending());
    super::common::status_bar(left, right, width)
}
