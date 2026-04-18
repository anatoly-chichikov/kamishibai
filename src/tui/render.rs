use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::app::App;
use super::screen::{ModalKind, Screen};

/// Render the current app state into a ratatui frame.
///
/// This is a placeholder skeleton. Final pixel-perfect layouts are produced by
/// each `CTX-17x`/`CTX-18x` screen task, which anchors on the PDF reference.
pub fn draw(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let header = Paragraph::new(format!("kamishibai · {}", app.pair().label()));
    frame.render_widget(header, areas[0]);
    body(frame, areas[1], app);
    footer(frame, areas[2], app);
    if let Some(kind) = app.modal() {
        modal(frame, frame.area(), kind);
    }
}

fn body(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let title = match app.screen() {
        Screen::YourWords => "Your words",
        Screen::WhatIUnderstood => "What I understood",
        Screen::YourCards => "Your cards",
        Screen::Done => "Done",
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(block, area);
}

fn footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let hint = match app.screen() {
        Screen::YourWords => "[Enter] continue · [L] my language",
        Screen::WhatIUnderstood => "[Enter] make cards · [R] change something",
        Screen::YourCards => "[R] change this card · [Enter] expand",
        Screen::Done => "[N] new batch · [Q] quit",
    };
    frame.render_widget(Paragraph::new(Line::from(hint)), area);
}

fn modal(frame: &mut Frame, area: ratatui::layout::Rect, kind: ModalKind) {
    let title = match kind {
        ModalKind::ChangeSomething => "Change something",
        ModalKind::ChangeThisCard => "Change this card",
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inset = centered(area, 60, 12);
    frame.render_widget(Clear, inset);
    frame.render_widget(block, inset);
}

fn centered(area: ratatui::layout::Rect, width: u16, height: u16) -> ratatui::layout::Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    ratatui::layout::Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
