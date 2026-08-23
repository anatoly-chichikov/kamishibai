//! Crossterm key events translated into the high-level `AppEvent` enum.
//!
//! Hotkeys are named in English but pressed on whatever layout the user has
//! active, so every combination this mapper dispatches on is folded back to
//! the Latin letter printed on that physical key: Ctrl+C on a Russian
//! (ЙЦУКЕН) layout arrives as the Cyrillic `с`, and `latin_key` answers `c`.
//! Plain printable characters keep their original codepoint here so the user
//! can type Cyrillic into the blob editor unchanged; the screens that own a
//! plain-letter hotkey fold it themselves, in `transition::promote`, on the
//! screens where nothing is being typed.

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
            match latin_key(symbol) {
                'l' => Some(AppEvent::OpenPreferredLanguagePicker),
                _ => None,
            }
        }
        KeyCode::Char(symbol) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            match latin_key(symbol) {
                'c' => Some(AppEvent::Quit),
                'e' => Some(AppEvent::WelcomeLoadEnvKey),
                'g' => Some(AppEvent::Generate),
                'l' => Some(AppEvent::OpenPreferredLanguagePicker),
                _ => None,
            }
        }
        KeyCode::Char(symbol) => Some(AppEvent::KeyChar(symbol)),
        _ => None,
    }
}

/// Map one typed character to the Latin letter printed on that physical key.
///
/// A non-Latin layout produces its own codepoint for every key — `с` on
/// ЙЦУКЕН, `ψ` on Greek, `і` on Ukrainian — while every hotkey in the
/// application is named by its QWERTY position, so we fold the codepoint back
/// here and hotkeys stop depending on which layout is active. Characters that
/// are already Latin pass through lowercased, and anything this table does not
/// name is returned unchanged. Ukrainian `і` takes the ЙЦУКЕН `ы` position it
/// replaces; the Belarusian `і` sitting a row below is not a hotkey either way.
#[must_use]
pub fn latin_key(symbol: char) -> char {
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
        'і' | 'І' => 's',
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
