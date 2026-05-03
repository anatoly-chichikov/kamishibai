use ratatui::Frame;

use super::app::App;
use super::screen::Screen;
use super::screens;
use super::screens::ScreenView;

/// Render the current app state into a ratatui frame.
///
/// Fullscreen screens go through exactly one path: pick a `&dyn ScreenView`
/// from the active `Screen` enum variant and hand it to
/// `screens::common::render_screen`, which owns the chrome (background,
/// header, dashed rule, footer). Overlays (modals, busy spinner, error toast)
/// are layered on top afterwards — they are not screens and do not
/// participate in the chrome contract.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let view: &dyn ScreenView = match app.screen() {
        Screen::Welcome => &screens::welcome::Welcome,
        Screen::YourWords => &screens::your_words::YourWords,
        Screen::WhatIUnderstood => &screens::what_i_understood::WhatIUnderstood,
        Screen::YourCards => &screens::your_cards::YourCards,
        Screen::Done => &screens::done::Done,
    };
    screens::common::render_screen(frame, area, app, view);
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
