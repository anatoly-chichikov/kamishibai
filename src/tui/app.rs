use crate::session::{Artifact, CardArtifacts, CardDraft, LanguagePair, WordCandidate};

use super::screen::{ModalKind, Screen};

/// The immutable shell state carried between transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    screen: Screen,
    modal: Option<ModalKind>,
    pair: LanguagePair,
    input: AppInput,
    review: Review,
    cards: CardsView,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardsView {
    pub drafts: Vec<CardDraft>,
    pub selected: usize,
    pub expanded: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppInput {
    pub blob: String,
    pub modal: String,
    pub failed: usize,
    pub target_pending: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Review {
    pub candidates: Vec<WordCandidate>,
    pub selected: usize,
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
            review: Review::default(),
            cards: CardsView::default(),
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
            review: self.review,
            cards: self.cards,
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
            review: Review::default(),
            cards: CardsView::default(),
        }
    }

    /// Return the confirmed candidates to be reviewed.
    pub fn candidates(&self) -> &[WordCandidate] {
        self.review.candidates.as_slice()
    }

    /// Return the currently highlighted candidate index.
    pub fn selected(&self) -> usize {
        self.review.selected
    }

    /// Return the app with a new set of understood candidates installed.
    pub fn understood(mut self, candidates: Vec<WordCandidate>) -> Self {
        self.review = Review {
            candidates,
            selected: 0,
        };
        self
    }

    /// Return the app with the cursor moved one row down (saturates at last).
    pub fn selected_next(mut self) -> Self {
        if !self.review.candidates.is_empty() {
            let last = self.review.candidates.len() - 1;
            if self.review.selected < last {
                self.review.selected += 1;
            }
        }
        self
    }

    /// Return the app with the cursor moved one row up (saturates at zero).
    pub fn selected_previous(mut self) -> Self {
        if self.review.selected > 0 {
            self.review.selected -= 1;
        }
        self
    }

    /// Return the current card drafts for the Your Cards screen.
    pub fn cards(&self) -> &[CardDraft] {
        self.cards.drafts.as_slice()
    }

    /// Return the currently focused card index.
    pub fn card_selected(&self) -> usize {
        self.cards.selected
    }

    /// Return whether the focused card is expanded.
    pub fn card_expanded(&self) -> bool {
        self.cards.expanded
    }

    /// Return the app with a new card session installed.
    pub fn cards_started(mut self, drafts: Vec<CardDraft>) -> Self {
        self.cards = CardsView {
            drafts,
            selected: 0,
            expanded: false,
        };
        self
    }

    /// Return the app with card cursor moved down (saturates).
    pub fn card_selected_next(mut self) -> Self {
        if !self.cards.drafts.is_empty() {
            let last = self.cards.drafts.len() - 1;
            if self.cards.selected < last {
                self.cards.selected += 1;
                self.cards.expanded = false;
            }
        }
        self
    }

    /// Return the app with card cursor moved up (saturates).
    pub fn card_selected_previous(mut self) -> Self {
        if self.cards.selected > 0 {
            self.cards.selected -= 1;
            self.cards.expanded = false;
        }
        self
    }

    /// Return the app with the focused card toggled between expanded and collapsed.
    pub fn card_toggle_expanded(mut self) -> Self {
        self.cards.expanded = !self.cards.expanded;
        self
    }

    /// Return the app with a different artifact bundle installed for the focused card.
    pub fn card_patched_artifacts(mut self, artifacts: CardArtifacts) -> Self {
        if let Some(draft) = self.cards.drafts.get(self.cards.selected).cloned() {
            self.cards.drafts[self.cards.selected] = draft.with_artifacts(artifacts);
        }
        self
    }

    /// Return the count of ready cards for the status line.
    pub fn cards_ready(&self) -> usize {
        self.cards
            .drafts
            .iter()
            .filter(|draft| draft.artifacts().all_ready())
            .count()
    }

    /// Return the count of failed cards for the status line.
    pub fn cards_failed(&self) -> usize {
        self.cards
            .drafts
            .iter()
            .filter(|draft| draft.artifacts().has_failed())
            .count()
    }

    /// Return the artifact-specific display hint for the focused card.
    pub fn card_artifact_hint(&self, kind: Artifact) -> &'static str {
        let Some(draft) = self.cards.drafts.get(self.cards.selected) else {
            return "queued";
        };
        artifact_hint(draft.artifacts(), kind)
    }

    /// Return the app with the selected candidate removed.
    pub fn dropped_selected(mut self) -> Self {
        if self.review.candidates.is_empty() {
            return self;
        }
        let index = self.review.selected.min(self.review.candidates.len() - 1);
        self.review.candidates.remove(index);
        if self.review.selected >= self.review.candidates.len()
            && !self.review.candidates.is_empty()
        {
            self.review.selected = self.review.candidates.len() - 1;
        } else if self.review.candidates.is_empty() {
            self.review.selected = 0;
        }
        self
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

fn artifact_hint(artifacts: &CardArtifacts, kind: Artifact) -> &'static str {
    let slot = match kind {
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    };
    if slot.ready() {
        return "ready";
    }
    if slot.failed_terminally() {
        return "failed";
    }
    if slot.tally().done() > 0 {
        return "retrying";
    }
    "queued"
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
