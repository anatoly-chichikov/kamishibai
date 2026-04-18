use crate::session::LanguagePair;

use super::screen::{ModalKind, Screen};

/// The immutable shell state carried between transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    screen: Screen,
    modal: Option<ModalKind>,
    pair: LanguagePair,
    input: AppInput,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppInput {
    pub blob: String,
    pub modal: String,
    pub failed: usize,
    pub target_pending: bool,
}

impl App {
    /// Create a fresh app sitting on `YourWords` with an initial language pair.
    pub fn new(pair: LanguagePair) -> Self {
        Self {
            screen: Screen::YourWords,
            modal: None,
            pair,
            input: AppInput {
                target_pending: true,
                ..AppInput::default()
            },
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
        self.input.failed
    }

    /// Return the raw blob currently typed on Your words.
    pub fn blob(&self) -> &str {
        self.input.blob.as_str()
    }

    /// Return the comment currently typed in an open modal.
    pub fn modal_buffer(&self) -> &str {
        self.input.modal.as_str()
    }

    /// Return whether the detected target language has been confirmed yet.
    pub fn target_pending(&self) -> bool {
        self.input.target_pending
    }

    /// Return the app with a different fullscreen state.
    pub fn with_screen(mut self, next: Screen) -> Self {
        self.screen = next;
        self.modal = None;
        self.input.modal.clear();
        self
    }

    /// Return the app with a modal opened.
    pub fn with_modal(mut self, modal: ModalKind) -> Self {
        self.modal = Some(modal);
        self.input.modal.clear();
        self
    }

    /// Return the app with the current modal dismissed.
    pub fn close_modal(mut self) -> Self {
        self.modal = None;
        self.input.modal.clear();
        self
    }

    /// Return the app with `my` language flipped through the catalog.
    pub fn toggle_support(self) -> Self {
        let current = self.pair.support().to_string();
        let next = cycle_support(current.as_str());
        let pair = LanguagePair::new(self.pair.target().to_string(), next);
        Self {
            screen: self.screen,
            modal: self.modal,
            pair,
            input: self.input,
        }
    }

    /// Return the app with a new target language code (user override).
    pub fn override_target(mut self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(code, self.pair.support().to_string());
        self.pair = pair;
        self.input.target_pending = false;
        self
    }

    /// Return the app with a confirmed target language guess from the LLM pass.
    pub fn confirmed_target(mut self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(code, self.pair.support().to_string());
        self.pair = pair;
        self.input.target_pending = false;
        self
    }

    /// Return the app reset for a fresh batch while keeping the language pair.
    pub fn fresh_batch(self) -> Self {
        Self {
            screen: Screen::YourWords,
            modal: None,
            pair: self.pair,
            input: AppInput {
                target_pending: true,
                ..AppInput::default()
            },
        }
    }

    /// Return the app with a different number of failed cards recorded.
    pub fn with_failed(mut self, failed: usize) -> Self {
        self.input.failed = failed;
        self
    }

    /// Return the app with one character appended to the active text buffer.
    pub fn typed(mut self, symbol: char) -> Self {
        if self.modal.is_some() {
            self.input.modal.push(symbol);
        } else if self.screen == Screen::YourWords {
            self.input.blob.push(symbol);
        }
        self
    }

    /// Return the app with one character removed from the active text buffer.
    pub fn rubbed(mut self) -> Self {
        if self.modal.is_some() {
            self.input.modal.pop();
        } else if self.screen == Screen::YourWords {
            self.input.blob.pop();
        }
        self
    }

    /// Return the app with a brand new blob installed (used for clipboard paste).
    pub fn seeded_blob(mut self, blob: impl Into<String>) -> Self {
        self.input.blob = blob.into();
        self
    }

    /// Return the app with the blob wiped (used after successful submission).
    pub fn clear_blob(mut self) -> Self {
        self.input.blob.clear();
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
