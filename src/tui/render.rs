use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::app::App;
use super::screen::Screen;
use super::screens;

/// Render the current app state into a ratatui frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    match app.screen() {
        Screen::YourWords => screens::your_words::draw(frame, area, app),
        Screen::WhatIUnderstood => screens::what_i_understood::draw(frame, area, app),
        other => placeholder(frame, area, app, other),
    }
    if let Some(kind) = app.modal() {
        screens::modals::draw(frame, area, kind, app);
    }
}

fn placeholder(frame: &mut Frame, area: Rect, app: &App, screen: Screen) {
    let title = match screen {
        Screen::YourWords => "Your words",
        Screen::WhatIUnderstood => "What I understood",
        Screen::YourCards => "Your cards",
        Screen::Done => "Done",
    };
    let header = Paragraph::new(format!(
        "kamishibai · {} → {}",
        app.pair().target().to_uppercase(),
        app.pair().support().to_uppercase(),
    ));
    let areas = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(1),
            ratatui::layout::Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(header, areas[0]);
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(block, areas[1]);
    let footer = match screen {
        Screen::WhatIUnderstood => "[Enter] make cards · [R] change something",
        Screen::YourCards => "[R] change this card · [Enter] expand",
        Screen::Done => "[N] new batch · [Q] quit",
        Screen::YourWords => "[Enter] continue",
    };
    frame.render_widget(Paragraph::new(Line::from(footer)), areas[2]);
}
