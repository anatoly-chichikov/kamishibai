//! Crossterm key events translated into the high-level `AppEvent` enum.
//!
//! Ctrl-combinations are normalised across keyboard layouts — pressing
//! Ctrl+C on a Russian (ЙЦУКЕН) layout produces the Cyrillic letter `с`,
//! which we map back to the physical-key Latin equivalent before dispatch.
//! Plain printable characters keep their original codepoint so the user can
//! type Cyrillic into the blob editor unchanged.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::event::AppEvent;

/// Convert one crossterm `KeyEvent` into the matching high-level `AppEvent`.
///
/// This mapper is intentionally context-free: individual screens may choose to
/// reinterpret `KeyChar`/`KeyBackspace` when the user is typing in a modal.
pub fn to_app(key: KeyEvent) -> Option<AppEvent> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    match key.code {
        KeyCode::Enter => Some(AppEvent::KeyEnter),
        KeyCode::Esc => Some(AppEvent::Cancel),
        KeyCode::Backspace => Some(AppEvent::KeyBackspace),
        KeyCode::Up => Some(AppEvent::NavPrev),
        KeyCode::Down => Some(AppEvent::NavNext),
        KeyCode::Left => Some(AppEvent::CursorLeft),
        KeyCode::Right => Some(AppEvent::CursorRight),
        KeyCode::Char(symbol) if key.modifiers.contains(KeyModifiers::SUPER) => {
            match latin_for_ctrl(symbol) {
                'l' => Some(AppEvent::OpenLanguagePicker),
                _ => None,
            }
        }
        KeyCode::Char(symbol) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            match latin_for_ctrl(symbol) {
                'c' => Some(AppEvent::Quit),
                'e' => Some(AppEvent::WelcomeLoadEnvKey),
                'g' => Some(AppEvent::Generate),
                'l' => Some(AppEvent::OpenLanguagePicker),
                _ => None,
            }
        }
        KeyCode::Char(symbol) => Some(AppEvent::KeyChar(symbol)),
        _ => None,
    }
}

/// Map a Ctrl-combination character to its physical-key Latin equivalent.
///
/// Pressing Ctrl while a non-Latin layout is active still produces a
/// printable codepoint of that layout — `Ctrl + с` on ЙЦУКЕН, `Ctrl + ψ`
/// on Greek, and so on. Hotkeys are matched against the QWERTY position
/// underneath, so we fold the codepoint back here. Characters that are
/// already Latin pass through lowercased.
#[must_use]
pub fn latin_for_ctrl(symbol: char) -> char {
    let lowered = symbol.to_ascii_lowercase();
    if lowered.is_ascii_alphabetic() {
        return lowered;
    }
    match symbol {
        'й' | 'Й' => 'q',
        'ц' | 'Ц' => 'w',
        'у' | 'У' => 'e',
        'к' | 'К' => 'r',
        'е' | 'Е' => 't',
        'н' | 'Н' => 'y',
        'г' | 'Г' => 'u',
        'ш' | 'Ш' => 'i',
        'щ' | 'Щ' => 'o',
        'з' | 'З' => 'p',
        'ф' | 'Ф' => 'a',
        'ы' | 'Ы' => 's',
        'в' | 'В' => 'd',
        'а' | 'А' => 'f',
        'п' | 'П' => 'g',
        'р' | 'Р' => 'h',
        'о' | 'О' => 'j',
        'л' | 'Л' => 'k',
        'д' | 'Д' => 'l',
        'я' | 'Я' => 'z',
        'ч' | 'Ч' => 'x',
        'с' | 'С' => 'c',
        'м' | 'М' => 'v',
        'и' | 'И' => 'b',
        'т' | 'Т' => 'n',
        'ь' | 'Ь' => 'm',
        'α' | 'Α' => 'a',
        'β' | 'Β' => 'b',
        'ψ' | 'Ψ' => 'c',
        'δ' | 'Δ' => 'd',
        'ε' | 'Ε' => 'e',
        'φ' | 'Φ' => 'f',
        'γ' | 'Γ' => 'g',
        'η' | 'Η' => 'h',
        'ι' | 'Ι' => 'i',
        'ξ' | 'Ξ' => 'j',
        'κ' | 'Κ' => 'k',
        'λ' | 'Λ' => 'l',
        'μ' | 'Μ' => 'm',
        'ν' | 'Ν' => 'n',
        'ο' | 'Ο' => 'o',
        'π' | 'Π' => 'p',
        'ρ' | 'Ρ' => 'r',
        'σ' | 'Σ' => 's',
        'τ' | 'Τ' => 't',
        'θ' | 'Θ' => 'u',
        'ω' | 'Ω' => 'v',
        'ς' => 'w',
        'χ' | 'Χ' => 'x',
        'υ' | 'Υ' => 'y',
        'ζ' | 'Ζ' => 'z',
        _ => lowered,
    }
}
