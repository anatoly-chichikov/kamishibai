use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::event::AppEvent;

/// Convert one crossterm `KeyEvent` into the matching high-level `AppEvent`.
///
/// This mapper is intentionally context-free: individual screens may choose to
/// reinterpret `KeyChar`/`KeyBackspace` when the user is typing in a modal.
pub fn to_app(key: KeyEvent) -> Option<AppEvent> {
    match key.code {
        KeyCode::Enter => Some(AppEvent::Submit),
        KeyCode::Esc => Some(AppEvent::Cancel),
        KeyCode::Backspace => Some(AppEvent::KeyBackspace),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(AppEvent::Quit),
        KeyCode::Char('R') | KeyCode::Char('r') => Some(AppEvent::RequestChange),
        KeyCode::Char('N') | KeyCode::Char('n') => Some(AppEvent::NewBatch),
        KeyCode::Char('Q') | KeyCode::Char('q') => Some(AppEvent::Quit),
        KeyCode::Char('L') | KeyCode::Char('l') => Some(AppEvent::ToggleMyLanguage),
        KeyCode::Char(symbol) => Some(AppEvent::KeyChar(symbol)),
        _ => None,
    }
}
