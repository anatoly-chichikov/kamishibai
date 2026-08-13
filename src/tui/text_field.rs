//! Shared single-line text field rendering for TUI inputs and text modals.

use std::borrow::Cow;

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette;

/// One single-line field with a dim placeholder and a terminal-cursor offset.
pub(crate) struct TextField<'a> {
    value: Cow<'a, str>,
    placeholder: &'static str,
}

impl<'a> TextField<'a> {
    /// Create a text field from a visible value and placeholder.
    pub(crate) fn new(value: impl Into<Cow<'a, str>>, placeholder: &'static str) -> Self {
        Self {
            value: value.into(),
            placeholder,
        }
    }

    /// Render the field as one line.
    pub(crate) fn line(&self) -> Line<'static> {
        Line::from(self.spans())
    }

    /// Render the field as spans that can be embedded into a larger row.
    pub(crate) fn spans(&self) -> Vec<Span<'static>> {
        if self.value.is_empty() {
            vec![Span::styled(
                String::from(self.placeholder),
                palette::dim2(),
            )]
        } else {
            vec![Span::styled(self.value.to_string(), palette::base())]
        }
    }

    /// Return the display width occupied by the rendered value or placeholder.
    pub(crate) fn display_width(&self) -> u16 {
        text_width(if self.value.is_empty() {
            self.placeholder
        } else {
            self.value.as_ref()
        })
    }

    /// Return where the terminal cursor should sit inside the field.
    pub(crate) fn cursor_offset(&self) -> u16 {
        if self.value.is_empty() {
            0
        } else {
            text_width(self.value.as_ref())
        }
    }
}

fn text_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}
