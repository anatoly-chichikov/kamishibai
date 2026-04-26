use ratatui::Frame;

use super::app::App;
use super::screen::Screen;
use super::screens;

/// Render the current app state into a ratatui frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    screens::common::paint_background(frame, area);
    match app.screen() {
        Screen::YourWords => screens::your_words::draw(frame, area, app),
        Screen::WhatIUnderstood => screens::what_i_understood::draw(frame, area, app),
        Screen::YourCards => screens::your_cards::draw(frame, area, app),
        Screen::Done => screens::done::draw(frame, area, app),
    }
    if let Some(kind) = app.modal() {
        screens::modals::draw(frame, area, kind, app);
    }
    if let Some(busy) = app.busy() {
        screens::busy::draw(frame, area, busy);
    }
    if let Some(error) = app.error() {
        screens::error::draw(frame, area, app, error);
    }
}
