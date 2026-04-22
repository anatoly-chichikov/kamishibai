//! Grayscale ink palette for the Kamishibai TUI.
//!
//! Values are lifted directly from the design mockup
//! (`Kamishibai TUI A.html` · `:root` block). The TUI renders monochrome
//! manga-ink only — no red, no yellow, no accent hues. Every screen must
//! go through this module to stay in lock-step with the reference.

use ratatui::style::{Color, Modifier, Style};

/// Terminal background. Matches `--term-bg: #0a0a0a`.
pub const BG: Color = Color::Rgb(0x0a, 0x0a, 0x0a);
/// Primary ink color for ordinary text. Matches `--term-fg: #f0ede4`.
pub const FG: Color = Color::Rgb(0xf0, 0xed, 0xe4);
/// Positive / ready state. Matches `.c-ok: #cfccc2`.
pub const OK: Color = Color::Rgb(0xcf, 0xcc, 0xc2);
/// In-progress state. Matches `.c-wn: #9a968b`.
pub const WN: Color = Color::Rgb(0x9a, 0x96, 0x8b);
/// Dim muted text (dividers, captions). Matches `--term-dim: #6e6a60`.
pub const DIM: Color = Color::Rgb(0x6e, 0x6a, 0x60);
/// Key-hint color. Matches `.c-key: #bdbab0`.
pub const KEY: Color = Color::Rgb(0xbd, 0xba, 0xb0);

/// Return the base paragraph style (paper ink on terminal-dark background).
pub fn base() -> Style {
    Style::default().bg(BG).fg(FG)
}

/// Return the style for a muted / dim span.
pub fn dim() -> Style {
    Style::default().bg(BG).fg(DIM)
}

/// Return the style for a done / positive span.
pub fn ok() -> Style {
    Style::default().bg(BG).fg(OK)
}

/// Return the style for an in-progress span.
pub fn wn() -> Style {
    Style::default().bg(BG).fg(WN)
}

/// Return the style for a keyboard hint span.
pub fn key() -> Style {
    Style::default().bg(BG).fg(KEY)
}

/// Return the style for a failed / wavy-underlined span (wavy approximated
/// as plain underline — no terminal widget supports SGR 4:3 directly).
pub fn failure() -> Style {
    Style::default()
        .bg(BG)
        .fg(FG)
        .add_modifier(Modifier::UNDERLINED)
}

/// Return the style for a link-like span (`c-tl` — underlined white).
pub fn link() -> Style {
    Style::default()
        .bg(BG)
        .fg(FG)
        .add_modifier(Modifier::UNDERLINED)
}
