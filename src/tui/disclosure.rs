//! Shared controls for rows that open into an inline detail pane.

use super::event::AppEvent;
use super::screens::common::FooterHint;

/// The semantic action requested by a disclosure key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisclosureIntent {
    /// No disclosure-level action applies.
    None,
    /// Open the focused row's detail pane.
    Open,
    /// Close the focused row's detail pane.
    Close,
    /// Run the focused action inside the open detail pane.
    Action,
}

/// A reusable keyboard and footer contract for inline expandable rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DisclosureControls {
    open: bool,
    action: Option<&'static str>,
}

impl DisclosureControls {
    /// Return controls for one row, parameterized by whether its detail pane is open.
    pub(crate) fn new(open: bool) -> Self {
        Self { open, action: None }
    }

    /// Return controls with one Space-triggered action available inside the open pane.
    pub(crate) fn with_action(self, label: &'static str) -> Self {
        Self {
            action: Some(label),
            ..self
        }
    }

    /// Classify a raw app event against the shared disclosure contract.
    ///
    /// The side arrows belong to the focused horizontal control wherever one
    /// exists — a carousel, a text cursor, a picker column. A row that owns no
    /// such control lets them through to its own disclosure, so `→` opens what
    /// `Enter` opens and `←` closes what `Esc` closes.
    pub(crate) fn intent(self, event: &AppEvent) -> DisclosureIntent {
        match event {
            AppEvent::KeyEnter if self.open => DisclosureIntent::Close,
            AppEvent::KeyEnter => DisclosureIntent::Open,
            AppEvent::CursorRight if !self.open => DisclosureIntent::Open,
            AppEvent::CursorLeft if self.open => DisclosureIntent::Close,
            AppEvent::KeyChar(' ') if self.open && self.action.is_some() => {
                DisclosureIntent::Action
            }
            _ => DisclosureIntent::None,
        }
    }

    /// Return the secondary footer hint for the open/close toggle.
    pub(crate) fn secondary_toggle(self) -> FooterHint {
        FooterHint::secondary(self.toggle_key(), "toggle")
    }

    /// Return the Space action hint when the open pane has an action.
    pub(crate) fn primary_action(self) -> Option<FooterHint> {
        self.action.map(|label| FooterHint::primary("Space", label))
    }

    fn toggle_key(self) -> &'static str {
        if self.open { "Enter/←" } else { "Enter/→" }
    }
}
