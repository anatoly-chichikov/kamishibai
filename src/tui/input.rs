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
        KeyCode::Up => Some(AppEvent::NavPrev),
        KeyCode::Down => Some(AppEvent::NavNext),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(AppEvent::Quit),
        KeyCode::Char(symbol) => Some(AppEvent::KeyChar(symbol)),
        _ => None,
    }
}
