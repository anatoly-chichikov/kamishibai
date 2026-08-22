//! Static monochrome palette for the Kamishibai TUI.
//!
//! The TUI is pure manga-ink — no accent hue is allowed. Three channels carry
//! every distinction the screens draw, and each channel carries exactly one
//! meaning: [`Ink`] ranks a span inside the row it belongs to, the `HL`
//! background marks the row under the cursor and nothing else, and
//! `Modifier::BOLD` marks a finished card and nothing else. Underlining a span
//! says it opens something when clicked; its brightness still comes from its
//! rank.

use ratatui::style::{Color, Modifier, Style};

/// Terminal background.
pub const BG: Color = Color::Rgb(0x0e, 0x0e, 0x10);
/// Primary ink for the subject of a row.
pub const FG: Color = Color::Rgb(0xe6, 0xe3, 0xda);
/// Muted ink for the copy that explains the subject.
pub const DIM: Color = Color::Rgb(0x8b, 0x8a, 0x83);
/// Deeper muted ink for the bookkeeping beside the subject.
pub const DIM2: Color = Color::Rgb(0x5a, 0x59, 0x53);
/// Border color for rules, dashed dividers, and outlined chips.
pub const RULE: Color = Color::Rgb(0x2a, 0x2a, 0x2d);
/// Row highlight background — the row under the cursor uses this.
pub const HL: Color = Color::Rgb(0x1c, 0x1c, 0x1f);

/// Rank a span holds inside the row it belongs to.
///
/// The rank answers "how much of this row is this span", never "what state is
/// this row in": focus moves the background, completion moves the weight, and
/// neither of them repaints the ink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ink {
    /// The thing the row is about — a term, a chosen value, a broken artifact.
    Subject,
    /// The copy that explains the subject — glosses, sentences, questions.
    Detail,
    /// Bookkeeping beside the subject — indices, costs, timings, separators.
    Aside,
}

impl Ink {
    /// Return the style this rank takes on a row that is or is not focused.
    #[must_use]
    pub fn on(self, focused: bool) -> Style {
        Style::default()
            .bg(if focused { HL } else { BG })
            .fg(self.color())
    }

    /// Return the style of a span at this rank that opens something when clicked.
    #[must_use]
    pub fn link(self, focused: bool) -> Style {
        self.on(focused).add_modifier(Modifier::UNDERLINED)
    }

    /// Return the ink this rank writes with.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Subject => FG,
            Self::Detail => DIM,
            Self::Aside => DIM2,
        }
    }
}

/// Return the base paragraph style — subject ink on the terminal background.
pub fn base() -> Style {
    Style::default().bg(BG).fg(FG)
}

/// Return the inverse style: dark ink on a cream block, used by titles and chips.
pub fn invert() -> Style {
    Style::default().bg(FG).fg(BG)
}

/// Return the style used to draw `--rule` lines.
pub fn rule() -> Style {
    Style::default().bg(BG).fg(RULE)
}
