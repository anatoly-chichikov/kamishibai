use crate::session::LanguagePair;

use super::screen::{ModalKind, Screen};

/// The immutable shell state carried between transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    screen: Screen,
    modal: Option<ModalKind>,
    pair: LanguagePair,
    failed: usize,
}

impl App {
    /// Create a fresh app sitting on `YourWords` with an initial language pair.
    pub fn new(pair: LanguagePair) -> Self {
        Self {
            screen: Screen::YourWords,
            modal: None,
            pair,
            failed: 0,
        }
    }

    /// Return the current fullscreen state.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Return the currently open modal, if any.
    pub fn modal(&self) -> Option<ModalKind> {
        self.modal
    }

    /// Return the session language pair.
    pub fn pair(&self) -> &LanguagePair {
        &self.pair
    }

    /// Return how many cards failed in the current batch.
    pub fn failed(&self) -> usize {
        self.failed
    }

    /// Return the app with a different fullscreen state.
    pub fn with_screen(mut self, next: Screen) -> Self {
        self.screen = next;
        self.modal = None;
        self
    }

    /// Return the app with a modal opened.
    pub fn with_modal(mut self, modal: ModalKind) -> Self {
        self.modal = Some(modal);
        self
    }

    /// Return the app with the current modal dismissed.
    pub fn close_modal(mut self) -> Self {
        self.modal = None;
        self
    }

    /// Return the app with `my` language flipped through the catalog (for ToggleMyLanguage).
    pub fn toggle_support(self) -> Self {
        let current = self.pair.support().to_string();
        let next = cycle_support(current.as_str());
        let pair = LanguagePair::new(self.pair.target().to_string(), next);
        Self {
            screen: self.screen,
            modal: self.modal,
            pair,
            failed: self.failed,
        }
    }

    /// Return the app with a new target language code (user override).
    pub fn override_target(self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(code, self.pair.support().to_string());
        Self {
            screen: self.screen,
            modal: self.modal,
            pair,
            failed: self.failed,
        }
    }

    /// Return the app reset for a fresh batch while keeping the language pair.
    pub fn fresh_batch(self) -> Self {
        Self {
            screen: Screen::YourWords,
            modal: None,
            pair: self.pair,
            failed: 0,
        }
    }

    /// Return the app with a different number of failed cards recorded.
    pub fn with_failed(mut self, failed: usize) -> Self {
        self.failed = failed;
        self
    }
}

fn cycle_support(current: &str) -> String {
    let order = ["en", "ru", "es", "de", "el", "zh"];
    let mut position = 0;
    for (index, code) in order.iter().enumerate() {
        if *code == current {
            position = index;
            break;
        }
    }
    String::from(order[(position + 1) % order.len()])
}
