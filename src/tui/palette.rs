//! Static monochrome palette for the Kamishibai TUI.
//!
//! Values mirror the locked design tokens in
//! `kamishibai-simple/project/styles.css` (`:root`). The TUI is pure
//! manga-ink — no accent hue is allowed. Every screen reads colors through
//! this module so the rendered terminal stays in lock-step with the design.

use ratatui::style::{Color, Modifier, Style};

/// Terminal background. Matches `--bg: #0e0e10`.
pub const BG: Color = Color::Rgb(0x0e, 0x0e, 0x10);
/// Primary ink color for ordinary text. Matches `--fg: #e6e3da`.
pub const FG: Color = Color::Rgb(0xe6, 0xe3, 0xda);
/// Muted ink for secondary copy and dividers. Matches `--dim: #8b8a83`.
pub const DIM: Color = Color::Rgb(0x8b, 0x8a, 0x83);
/// Deeper muted ink for placeholder rows and pending steps. Matches `--dim2: #5a5953`.
pub const DIM2: Color = Color::Rgb(0x5a, 0x59, 0x53);
/// Border color for rules, dashed dividers, and outlined chips. Matches `--rule: #2a2a2d`.
pub const RULE: Color = Color::Rgb(0x2a, 0x2a, 0x2d);
/// Row highlight background — selected lines use this. Matches `--hl: #1c1c1f`.
pub const HL: Color = Color::Rgb(0x1c, 0x1c, 0x1f);

/// Return the base paragraph style (paper ink on terminal-dark background).
pub fn base() -> Style {
    Style::default().bg(BG).fg(FG)
}

/// Return the style for a muted / dim span (`--dim`).
pub fn dim() -> Style {
    Style::default().bg(BG).fg(DIM)
}

/// Return the style for the deepest muted span (`--dim2`).
pub fn dim2() -> Style {
    Style::default().bg(BG).fg(DIM2)
}

/// Return the style for a row highlighted as selected (background `--hl`, ink `--fg`).
pub fn highlight() -> Style {
    Style::default().bg(HL).fg(FG)
}

/// Return the style for a dim span over a highlighted row.
pub fn highlight_dim() -> Style {
    Style::default().bg(HL).fg(DIM)
}

/// Return the inverse style: black ink on cream block (used by titles).
pub fn invert() -> Style {
    Style::default().bg(FG).fg(BG)
}

/// Return the style used to draw `--rule` lines.
pub fn rule() -> Style {
    Style::default().bg(BG).fg(RULE)
}

/// Return the underlined link style — pure mono, no color shift.
pub fn link() -> Style {
    Style::default()
        .bg(BG)
        .fg(FG)
        .add_modifier(Modifier::UNDERLINED)
}
