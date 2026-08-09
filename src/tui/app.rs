use std::fmt;
use std::time::Duration;

use crate::session::{
    Artifact, CardArtifacts, CardDraft, LanguagePair, Sense, SentenceBatchSettings,
    SentenceLabelSelection, WordCandidate,
};

use super::screen::{KeySource, ModalKind, Screen, WelcomeFocus, WelcomeStage};
use super::sentence_editor::{BatchSettingsRow, LabelEditorRow, NoteDraft, SentenceLabelsEditor};

/// The immutable shell state carried between transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    screen: Screen,
    modal: Option<ModalKind>,
    busy: Option<BusyView>,
    error: Option<String>,
    pair: LanguagePair,
    input: AppInput,
    blob_cursor: usize,
    review: Review,
    sentence_settings: SentenceBatchSettings,
    sentence_settings_row: Option<BatchSettingsRow>,
    cards: CardsView,
    done: DoneArtifacts,
    welcome: WelcomeView,
    body_scroll: u16,
    quit_pending: bool,
    new_batch_pending: bool,
    picker_cursor: usize,
}

/// First-run welcome state: stage, typed key, source of that key, focused
/// control on the key step, and whether `GEMINI_API_KEY` is offered from env.
#[derive(Clone, Eq, PartialEq)]
pub struct WelcomeView {
    pub stage: WelcomeStage,
    pub key: String,
    pub source: KeySource,
    pub notice: Option<String>,
    pub focus: WelcomeFocus,
    pub env_available: bool,
}

impl fmt::Debug for WelcomeView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WelcomeView")
            .field("stage", &self.stage)
            .field("key", &"[REDACTED]")
            .field("source", &self.source)
            .field("notice", &self.notice)
            .field("focus", &self.focus)
            .field("env_available", &self.env_available)
            .finish()
    }
}

impl Default for WelcomeView {
    fn default() -> Self {
        Self {
            stage: WelcomeStage::PickLanguage,
            key: String::new(),
            source: KeySource::Empty,
            notice: None,
            focus: WelcomeFocus::Submit,
            env_available: false,
        }
    }
}

/// The blocking text pass currently covering the interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusyKind {
    Understanding,
    BulkCorrection,
    /// Welcome key step: probing Gemini to confirm the entered key is accepted.
    CheckingKey,
    /// Phase 1 of `publish`: building the Anki .apkg container.
    PublishingDeck,
    /// Phase 2 of `publish`: rendering the printable PDF.
    PublishingReport,
}

impl BusyKind {
    /// Return the short text shown in the universal loader.
    pub fn label(&self) -> &'static str {
        match self {
            BusyKind::Understanding => "understanding your words",
            BusyKind::BulkCorrection => "adding missing meanings",
            BusyKind::CheckingKey => "checking your key",
            BusyKind::PublishingDeck => "building your anki deck",
            BusyKind::PublishingReport => "rendering your printable pdf",
        }
    }
}

/// The universal loader overlay state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusyView {
    kind: BusyKind,
    elapsed: Duration,
}

impl BusyView {
    /// Create one loader state for a blocking text pass.
    pub fn new(kind: BusyKind) -> Self {
        Self {
            kind,
            elapsed: Duration::ZERO,
        }
    }

    /// Return the currently running blocking pass.
    pub fn kind(&self) -> BusyKind {
        self.kind
    }

    /// Return how long the current blocking pass has been running.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Return the loader with a refreshed elapsed duration.
    pub fn with_elapsed(mut self, elapsed: Duration) -> Self {
        self.elapsed = elapsed;
        self
    }

    /// Return the loader with the kind swapped, preserving elapsed time. Used
    /// when a single background job advances through multiple phases (the
    /// publish job flips from `PublishingDeck` to `PublishingReport`).
    pub fn with_kind(mut self, kind: BusyKind) -> Self {
        self.kind = kind;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardsView {
    pub drafts: Vec<CardDraft>,
    pub selected: usize,
    pub expanded: bool,
    pub elapsed: Duration,
    pub running: Option<(usize, Artifact)>,
    editor: Option<SentenceLabelsEditor>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DoneArtifacts {
    pub deck: String,
    pub report: String,
    pub output: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppInput {
    pub blob: String,
    pub modal: String,
    pub failed: usize,
    pub learning_pending: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Review {
    pub candidates: Vec<WordCandidate>,
    pub selected: usize,
    pub expanded: Option<ExpandedSense>,
    pub notice: Option<String>,
}

/// Expanded sense picker state for one review row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpandedSense {
    pub row: usize,
    pub cursor: usize,
    pub selected: Vec<usize>,
}

impl App {
    /// Create a fresh app sitting on `YourWords` with an initial language pair.
    pub fn new(pair: LanguagePair) -> Self {
        Self {
            screen: Screen::YourWords,
            modal: None,
            busy: None,
            error: None,
            pair,
            input: AppInput {
                learning_pending: true,
                ..AppInput::default()
            },
            blob_cursor: 0,
            review: Review::default(),
            sentence_settings: SentenceBatchSettings::default(),
            sentence_settings_row: None,
            cards: CardsView::default(),
            done: DoneArtifacts::default(),
            welcome: WelcomeView::default(),
            body_scroll: 0,
            quit_pending: false,
            new_batch_pending: false,
            picker_cursor: 0,
        }
    }

    /// Return whether a first Ctrl+C has been received and the user is one
    /// keystroke away from quitting.
    pub fn quit_pending(&self) -> bool {
        self.quit_pending
    }

    /// Return the app with the quit-pending flag updated.
    pub fn with_quit_pending(mut self, pending: bool) -> Self {
        self.quit_pending = pending;
        self
    }

    /// Return whether a first Escape has armed the final-screen new-batch gesture.
    pub fn new_batch_pending(&self) -> bool {
        self.new_batch_pending
    }

    /// Return the app with the new-batch confirmation flag updated.
    pub fn with_new_batch_pending(mut self, pending: bool) -> Self {
        self.new_batch_pending = pending;
        self
    }

    /// Return whether a finished batch can be replaced from the final screen.
    pub fn can_start_new_batch(&self) -> bool {
        let terminal = !self.cards.drafts.is_empty()
            && self
                .cards
                .drafts
                .iter()
                .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed());
        matches!(self.screen, Screen::YourCards | Screen::Done)
            && (!self.done.deck.is_empty() || terminal)
            && self.modal.is_none()
            && self.busy.is_none()
            && self.error.is_none()
            && self.cards.editor.is_none()
    }

    /// Start a clean batch while preserving the user's current language direction.
    pub fn starting_new_batch(self) -> Self {
        Self::new(self.pair)
    }

    /// Return the app rerouted onto the first-run Welcome screen starting
    /// at the language-pick stage. `env_available` tells the key step whether
    /// to offer the `load from env` action.
    pub fn opening_welcome(
        self,
        source: KeySource,
        key: impl Into<String>,
        env_available: bool,
    ) -> Self {
        self.opening_welcome_at(WelcomeStage::PickLanguage, source, key, env_available)
    }

    /// Return the app rerouted onto the first-run Welcome screen with an
    /// explicit starting stage. Used by `start()` to skip past whichever step
    /// is already satisfied by the loaded preferences. `env_available` reflects
    /// whether `GEMINI_API_KEY` is present — it is never loaded into the buffer
    /// implicitly, only offered as the `load from env` action.
    pub fn opening_welcome_at(
        mut self,
        stage: WelcomeStage,
        source: KeySource,
        key: impl Into<String>,
        env_available: bool,
    ) -> Self {
        self.screen = Screen::Welcome;
        self.welcome = WelcomeView {
            stage,
            key: key.into(),
            source,
            notice: None,
            focus: WelcomeFocus::Submit,
            env_available,
        };
        self.sentence_settings_row = None;
        self
    }

    /// Return the welcome view (read-only).
    pub fn welcome(&self) -> &WelcomeView {
        &self.welcome
    }

    /// Return the app advanced from picking language to entering a key.
    pub fn welcome_advance(mut self) -> Self {
        self.welcome.stage = WelcomeStage::EnterKey;
        self.welcome.notice = None;
        self.welcome.focus = WelcomeFocus::Submit;
        self
    }

    /// Return the app stepped back from entering the key to picking the language.
    pub fn welcome_step_back(mut self) -> Self {
        self.welcome.stage = WelcomeStage::PickLanguage;
        self.welcome.notice = None;
        self
    }

    /// Return the app with a freshly pasted API key on the welcome screen.
    pub fn welcome_paste_key(mut self, key: impl Into<String>) -> Self {
        let key: String = key.into();
        let trimmed = key.trim().to_string();
        self.welcome.key = trimmed.clone();
        self.welcome.source = if trimmed.is_empty() {
            KeySource::Empty
        } else {
            KeySource::Pasted
        };
        self.welcome.notice = None;
        self
    }

    /// Return the app with an API key explicitly loaded from the environment.
    pub fn welcome_env_key(mut self, key: impl Into<String>) -> Self {
        let key: String = key.into();
        let trimmed = key.trim().to_string();
        self.welcome.key = trimmed.clone();
        self.welcome.source = if trimmed.is_empty() {
            KeySource::Empty
        } else {
            KeySource::Env
        };
        self.welcome.notice = None;
        self.welcome.focus = WelcomeFocus::Submit;
        self
    }

    /// Return the app with a setup notice shown on the Welcome screen — used for
    /// the inline `key invalid` / `enter a key first` / env messages.
    pub fn welcome_notice(mut self, message: impl Into<String>) -> Self {
        self.welcome.notice = Some(message.into());
        self
    }

    /// Return the app with the API key cleared so the user can paste a new one.
    pub fn welcome_clear_key(mut self) -> Self {
        self.welcome.key = String::new();
        self.welcome.source = KeySource::Empty;
        self.welcome.notice = None;
        self
    }

    /// Return the app with welcome focus moved to the next control in the cycle.
    pub fn welcome_focus_next(mut self) -> Self {
        self.welcome.focus = step_focus(self.welcome.focus, self.welcome.env_available, 1);
        self
    }

    /// Return the app with welcome focus moved to the previous control.
    pub fn welcome_focus_prev(mut self) -> Self {
        self.welcome.focus = step_focus(self.welcome.focus, self.welcome.env_available, -1);
        self
    }

    /// Return the app with welcome focus set to a specific control (mouse click).
    pub fn welcome_focus(mut self, focus: WelcomeFocus) -> Self {
        self.welcome.focus = focus;
        self
    }

    /// Return the current fullscreen state.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Return the currently open modal, if any.
    pub fn modal(&self) -> Option<ModalKind> {
        self.modal
    }

    /// Return the universal blocking loader, if a text pass is running.
    pub fn busy(&self) -> Option<&BusyView> {
        self.busy.as_ref()
    }

    /// Return the last recoverable request error, if one is being shown.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
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

    /// Return the raw blob cursor as a byte offset.
    pub fn blob_cursor(&self) -> usize {
        self.blob_cursor
    }

    /// Return the comment currently typed in an open modal.
    pub fn modal_buffer(&self) -> &str {
        self.input.modal.as_str()
    }

    /// Return whether the detected learning language has been confirmed yet.
    pub fn learning_pending(&self) -> bool {
        self.input.learning_pending
    }

    /// Return the app with a different fullscreen state.
    pub fn with_screen(mut self, next: Screen) -> Self {
        self.screen = next;
        self.modal = None;
        self.input.modal.clear();
        self.cards.editor = None;
        self.sentence_settings_row = None;
        self.body_scroll = 0;
        self
    }

    /// Return the current body scroll offset in lines.
    pub fn body_scroll(&self) -> u16 {
        self.body_scroll
    }

    /// Return the app with the body scroll bumped by `delta` lines, clamped so
    /// the bottom row of content stays at or above the bottom row of the
    /// `viewport`. Pass the actual scrollable height the renderer hands to the
    /// body widget — for `Your cards` / `Done` that is the body rect height
    /// minus the sticky outputs banner. `body_width` is the body rect width in
    /// chars; the layout calc on `Your cards` wraps the meta sentence on the
    /// head row, so the clamp must agree with the renderer about that width.
    /// A zero `viewport` clamps to zero.
    pub fn body_scrolled(mut self, delta: i32, viewport: u16, body_width: u16) -> Self {
        let max = self
            .body_content_height(body_width)
            .saturating_sub(viewport);
        let next = i32::from(self.body_scroll).saturating_add(delta).max(0);
        let clamped = next.min(i32::from(max));
        self.body_scroll = u16::try_from(clamped).unwrap_or(u16::MAX);
        self
    }

    /// Return the app with the body scroll re-clamped against the current
    /// `viewport`. Called every render tick so content that shrinks (e.g. when
    /// the user collapses an expanded card or removes candidates) snaps the
    /// view back so no blank tail is left below the content. `body_width` is
    /// the body rect width in chars, used by the head-row wrap calc.
    pub fn body_scroll_clamped(mut self, viewport: u16, body_width: u16) -> Self {
        let max = self
            .body_content_height(body_width)
            .saturating_sub(viewport);
        if self.body_scroll > max {
            self.body_scroll = max;
        }
        self
    }

    /// Return the app with the body scroll reset to the top.
    pub fn body_scroll_reset(mut self) -> Self {
        self.body_scroll = 0;
        self
    }

    /// Return the app with the focused row fully inside the body viewport.
    /// Used after text edits and keyboard navigation so wheel-scrolled content
    /// follows the active text cursor, review candidate, or card selection.
    /// `body_width` is the body rect width in chars; passed through so the
    /// `YourCards` snap math agrees with the renderer's wrapped head rows. An
    /// open card editor whose focused range fits anchors its card head at the
    /// viewport top; smaller viewports retain the focused-row fallback.
    pub fn body_scroll_to_selection(mut self, viewport: u16, body_width: u16) -> Self {
        let Some((top, height)) = self.focused_body_range(body_width) else {
            return self;
        };
        let max = self
            .body_content_height(body_width)
            .saturating_sub(viewport);
        let bottom = top.saturating_add(height);
        let mut next = self.body_scroll;
        let anchor_editor =
            self.screen == Screen::YourCards && self.cards.editor.is_some() && height <= viewport;
        if anchor_editor || top < next {
            next = top;
        } else if bottom > next.saturating_add(viewport) {
            next = bottom.saturating_sub(viewport);
        }
        if next > max {
            next = max;
        }
        self.body_scroll = next;
        self
    }

    fn focused_body_range(&self, body_width: u16) -> Option<(u16, u16)> {
        match self.screen {
            Screen::YourWords => {
                let (row, _) = cursor_row_column(&self.input.blob, self.blob_cursor);
                Some((u16::try_from(row).unwrap_or(u16::MAX), 1))
            }
            Screen::WhatIUnderstood if !self.review.candidates.is_empty() => {
                crate::tui::screens::what_i_understood::focused_range(self, usize::from(body_width))
            }
            Screen::YourCards => {
                crate::tui::screens::your_cards::focused_card_range(self, usize::from(body_width))
            }
            _ => None,
        }
    }

    fn body_content_height(&self, body_width: u16) -> u16 {
        let width = usize::from(body_width);
        match self.screen {
            Screen::YourCards => crate::tui::screens::your_cards::content_height(self, width),
            Screen::Done => crate::tui::screens::done::content_height(self),
            Screen::WhatIUnderstood => {
                crate::tui::screens::what_i_understood::content_height(self, width)
            }
            Screen::YourWords => crate::tui::screens::your_words::content_height(self),
            Screen::Welcome => 0,
        }
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

    /// Return the index of the chip currently highlighted inside the
    /// language picker modal. Meaningful only while `PickMyLanguage` is open.
    pub fn picker_cursor(&self) -> usize {
        self.picker_cursor
    }

    /// Return the app with the picker cursor set to a specific index. Used
    /// when opening the modal so the active language is pre-selected.
    pub fn with_picker_cursor(mut self, index: usize) -> Self {
        self.picker_cursor = index;
        self
    }

    /// Return the app with the picker cursor advanced by `delta`, wrapping
    /// around the supported-language catalog.
    pub fn picker_cursor_advanced(mut self, delta: i32) -> Self {
        let len = crate::languages::catalog().codes().len() as i32;
        let next = (self.picker_cursor as i32 + delta).rem_euclid(len);
        self.picker_cursor = next as usize;
        self
    }

    /// Return the app with the universal blocking loader shown.
    pub fn busy_started(mut self, kind: BusyKind) -> Self {
        self.busy = Some(BusyView::new(kind));
        self
    }

    /// Return the app with the universal blocking loader elapsed time updated.
    pub fn busy_elapsed(mut self, elapsed: Duration) -> Self {
        if let Some(busy) = self.busy.take() {
            self.busy = Some(busy.with_elapsed(elapsed));
        }
        self
    }

    /// Return the app with the universal blocking loader hidden.
    pub fn busy_finished(mut self) -> Self {
        self.busy = None;
        self
    }

    /// Return the app with the active loader's kind replaced — elapsed time
    /// keeps ticking. No-op if no loader is currently shown. Used by the
    /// publish flow to flip the label from `PublishingDeck` to
    /// `PublishingReport` mid-job.
    pub fn busy_kind_swapped(mut self, kind: BusyKind) -> Self {
        if let Some(busy) = self.busy.take() {
            self.busy = Some(busy.with_kind(kind));
        }
        self
    }

    /// Return the app with a recoverable request error shown.
    pub fn error_shown(mut self, message: impl Into<String>) -> Self {
        self.error = Some(message.into());
        self
    }

    /// Return the app with the recoverable request error dismissed.
    pub fn error_cleared(mut self) -> Self {
        self.error = None;
        self
    }

    /// Return the app with the known (native) language replaced by `code`.
    /// The learning language stays untouched. Use this from the language picker
    /// modal and from the Welcome screen — there is no implicit cycle anymore.
    pub fn set_known(mut self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(self.pair.learning().to_string(), code.into());
        self.pair = pair;
        self
    }

    /// Return the app with a confirmed learning language guess from the LLM pass.
    pub fn confirmed_learning(mut self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(code, self.pair.known().to_string());
        self.pair = pair;
        self.input.learning_pending = false;
        self
    }

    /// Return the confirmed candidates to be reviewed.
    pub fn candidates(&self) -> &[WordCandidate] {
        self.review.candidates.as_slice()
    }

    /// Return the currently highlighted candidate index.
    pub fn selected(&self) -> usize {
        self.review.selected
    }

    /// Return the expanded sense picker state, if any.
    pub fn expanded_sense(&self) -> Option<ExpandedSense> {
        self.review.expanded.clone()
    }

    /// Return the short review notice, if any.
    pub fn review_notice(&self) -> Option<&str> {
        self.review.notice.as_deref()
    }

    /// Return the durable sentence preferences chosen for this reviewed batch.
    #[must_use]
    pub fn sentence_settings(&self) -> SentenceBatchSettings {
        self.sentence_settings
    }

    /// Return the app carrying durable sentence preferences for this batch.
    #[must_use]
    pub fn with_sentence_settings(mut self, settings: SentenceBatchSettings) -> Self {
        self.sentence_settings = settings;
        self
    }

    /// Return the focused row of the open batch sentence-settings editor.
    #[must_use]
    pub fn sentence_settings_editor(&self) -> Option<BatchSettingsRow> {
        self.sentence_settings_row
    }

    /// Return the app with batch sentence settings open on the level row.
    #[must_use]
    pub fn sentence_settings_opened(mut self) -> Self {
        self.sentence_settings_row = Some(BatchSettingsRow::Level);
        self.review.expanded = None;
        self.review.notice = None;
        self
    }

    /// Return the app with batch sentence settings closed and choices retained.
    #[must_use]
    pub fn sentence_settings_closed(mut self) -> Self {
        self.sentence_settings_row = None;
        self
    }

    /// Return the app with one batch sentence-settings row focused.
    #[must_use]
    pub fn sentence_settings_focused(mut self, row: BatchSettingsRow) -> Self {
        if self.sentence_settings_row.is_some() {
            self.sentence_settings_row = Some(row);
        }
        self
    }

    /// Return the app with batch sentence-settings focus moved one row up.
    #[must_use]
    pub fn sentence_settings_row_previous(mut self) -> Self {
        if let Some(row) = self.sentence_settings_row {
            self.sentence_settings_row = Some(row.previous());
        }
        self
    }

    /// Return the app with batch sentence-settings focus moved one row down.
    #[must_use]
    pub fn sentence_settings_row_next(mut self) -> Self {
        if let Some(row) = self.sentence_settings_row {
            self.sentence_settings_row = Some(row.next());
        }
        self
    }

    /// Return the app with the focused batch sentence choice moved one step.
    #[must_use]
    pub fn sentence_settings_advanced(mut self, forward: bool) -> Self {
        if let Some(row) = self.sentence_settings_row {
            self.sentence_settings = row.advanced(self.sentence_settings, forward);
        }
        self
    }

    /// Return the app with one batch sentence choice selected directly.
    #[must_use]
    pub fn sentence_settings_chosen(mut self, index: usize) -> Self {
        if let Some(row) = self.sentence_settings_row {
            self.sentence_settings = row.choosing(self.sentence_settings, index);
        }
        self
    }

    /// Return the app with a new set of understood candidates installed.
    pub fn understood(mut self, candidates: Vec<WordCandidate>) -> Self {
        self.review = Review {
            candidates,
            selected: 0,
            expanded: None,
            notice: None,
        };
        self
    }

    /// Return the app with understood candidates installed while preserving selected senses by row.
    pub fn understood_preserving_senses(mut self, mut candidates: Vec<WordCandidate>) -> Self {
        for (index, candidate) in candidates.iter_mut().enumerate() {
            if let Some(previous) = self.review.candidates.get(index) {
                let last = candidate.senses().len() - 1;
                let selected = previous
                    .selected_senses()
                    .iter()
                    .map(|index| (*index).min(last))
                    .collect();
                *candidate = candidate.clone().selecting_senses(selected);
            }
        }
        let selected = self.review.selected.min(candidates.len().saturating_sub(1));
        self.review = Review {
            candidates,
            selected,
            expanded: None,
            notice: None,
        };
        self
    }

    /// Return whether the selected row can open a sense picker.
    pub fn selected_can_expand_senses(&self) -> bool {
        self.review
            .candidates
            .get(self.review.selected)
            .map(WordCandidate::ok)
            .unwrap_or(false)
    }

    /// Return the app with the selected row's sense picker opened.
    pub fn senses_expanded(mut self) -> Self {
        let Some(candidate) = self.review.candidates.get(self.review.selected) else {
            return self;
        };
        if !candidate.ok() {
            return self;
        }
        let selected = candidate.selected_senses().to_vec();
        let cursor = candidate.selected();
        self.review.expanded = Some(ExpandedSense {
            row: self.review.selected,
            cursor,
            selected,
        });
        self.review.notice = None;
        self
    }

    /// Return the app with the expanded sense cursor moved down.
    pub fn sense_next(mut self) -> Self {
        let Some(expanded) = self.review.expanded else {
            return self;
        };
        let Some(candidate) = self.review.candidates.get(expanded.row) else {
            self.review.expanded = None;
            return self;
        };
        let last = candidate.senses().len();
        let cursor = expanded.cursor.min(last).saturating_add(1).min(last);
        self.review.expanded = Some(ExpandedSense { cursor, ..expanded });
        self.review.notice = None;
        self
    }

    /// Return the app with the expanded sense cursor moved up.
    pub fn sense_previous(mut self) -> Self {
        let Some(expanded) = self.review.expanded else {
            return self;
        };
        let Some(_candidate) = self.review.candidates.get(expanded.row) else {
            self.review.expanded = None;
            return self;
        };
        let cursor = expanded.cursor.saturating_sub(1);
        self.review.expanded = Some(ExpandedSense { cursor, ..expanded });
        self.review.notice = None;
        self
    }

    /// Return the app with the focused sense toggled in the expanded multi-select.
    pub fn sense_toggled(mut self) -> Self {
        let Some(mut expanded) = self.review.expanded else {
            return self;
        };
        let Some(candidate) = self.review.candidates.get(expanded.row) else {
            self.review.expanded = None;
            return self;
        };
        if expanded.cursor >= candidate.senses().len() {
            self.review.expanded = Some(expanded);
            return self;
        }
        if let Some(position) = expanded
            .selected
            .iter()
            .position(|index| *index == expanded.cursor)
        {
            if expanded.selected.len() > 1 {
                expanded.selected.remove(position);
            }
        } else {
            expanded.selected.push(expanded.cursor);
            expanded.selected.sort_unstable();
        }
        self.review.expanded = Some(expanded);
        self.review.notice = None;
        self
    }

    /// Return whether the expanded picker cursor is on its add-more row.
    pub fn expanded_add_more_focused(&self) -> bool {
        let Some(expanded) = &self.review.expanded else {
            return false;
        };
        self.review
            .candidates
            .get(expanded.row)
            .map(|candidate| expanded.cursor >= candidate.senses().len())
            .unwrap_or(false)
    }

    /// Return the app with the expanded sense picker confirmed and closed.
    pub fn senses_confirmed(mut self) -> Self {
        if let Some(expanded) = self.review.expanded
            && let Some(candidate) = self.review.candidates.get(expanded.row).cloned()
        {
            self.review.candidates[expanded.row] = candidate.selecting_senses(expanded.selected);
        }
        self.review.expanded = None;
        self.review.notice = None;
        self
    }

    /// Return the app with the expanded sense picker cancelled and closed.
    pub fn senses_cancelled(mut self) -> Self {
        self.review.expanded = None;
        self.review.notice = None;
        self
    }

    /// Return the app with new senses appended to the selected row.
    pub fn senses_appended_to_selected(
        mut self,
        senses: Vec<Sense>,
        message: Option<String>,
    ) -> Self {
        let selected = self.review.selected;
        let Some(candidate) = self.review.candidates.get(selected).cloned() else {
            return self;
        };
        let (candidate, first) = candidate.with_added_senses(senses);
        self.review.candidates[selected] = candidate;
        self.review.notice = if first.is_none() && message.is_none() {
            Some(String::from("nothing to add"))
        } else {
            message
        };
        if let Some(cursor) = first {
            let expanded_selected = self.review.candidates[selected].selected_senses().to_vec();
            self.review.expanded = Some(ExpandedSense {
                row: selected,
                cursor,
                selected: expanded_selected,
            });
        }
        self
    }

    /// Return the app with the cursor moved one row down (saturates at last).
    pub fn selected_next(mut self) -> Self {
        self.review.expanded = None;
        self.review.notice = None;
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
        self.review.expanded = None;
        self.review.notice = None;
        if self.review.selected > 0 {
            self.review.selected -= 1;
        }
        self
    }

    /// Return the current card drafts for the Your Cards screen.
    pub fn cards(&self) -> &[CardDraft] {
        self.cards.drafts.as_slice()
    }

    /// Return how many cards carry a live pending rewrite.
    #[must_use]
    pub fn cards_pending(&self) -> usize {
        self.cards
            .drafts
            .iter()
            .filter(|draft| draft.staged_rewrite().is_some())
            .count()
    }

    /// Return whether the focused card can open its sentence-label editor.
    #[must_use]
    pub fn card_tunable(&self) -> bool {
        self.card_tunable_at(self.cards.selected)
    }

    /// Return whether one card can open its sentence-label editor.
    #[must_use]
    pub fn card_tunable_at(&self, card: usize) -> bool {
        self.cards.drafts.get(card).is_some_and(|draft| {
            draft.meta().is_some()
                && (draft.rewrite().is_none() || draft.staged_rewrite().is_some())
        })
    }

    /// Return the currently focused card index.
    pub fn card_selected(&self) -> usize {
        self.cards.selected
    }

    /// Return whether the focused card is expanded.
    pub fn card_expanded(&self) -> bool {
        self.cards.expanded
    }

    /// Return the open sentence-label editor for the focused card.
    #[must_use]
    pub fn sentence_editor(&self) -> Option<&SentenceLabelsEditor> {
        self.cards.editor.as_ref()
    }

    /// Return the app with the focused card expanded and its note row editing.
    #[must_use]
    pub fn sentence_editor_opened_for_note(self) -> Self {
        self.sentence_editor_opened(LabelEditorRow::Note)
    }

    /// Return the app with the focused card expanded and its register row editing.
    #[must_use]
    pub fn sentence_editor_opened_for_register(self) -> Self {
        self.sentence_editor_opened(LabelEditorRow::Register)
    }

    /// Return the app with the sentence-label editor and focused card collapsed.
    #[must_use]
    pub fn sentence_editor_closed(mut self) -> Self {
        self.cards.editor = None;
        self.cards.expanded = false;
        self
    }

    /// Return the app with the sentence-label editor focused on one row.
    #[must_use]
    pub fn sentence_editor_focused(mut self, row: LabelEditorRow) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.focused(row));
        }
        self
    }

    /// Return the app with the sentence-label editor moved to its previous row.
    #[must_use]
    pub fn sentence_editor_row_previous(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.row_previous());
        }
        self
    }

    /// Return the app with the sentence-label editor moved to its next row.
    #[must_use]
    pub fn sentence_editor_row_next(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.row_next());
        }
        self
    }

    /// Return the app with the focused sentence-label axis moved one chip.
    #[must_use]
    pub fn sentence_editor_axis_advanced(mut self, forward: bool) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.axis_advanced(forward));
            self.stage_sentence_editor();
        }
        self
    }

    /// Return the app with one chip selected on the focused sentence-label axis.
    #[must_use]
    pub fn sentence_editor_axis_chosen(mut self, index: usize) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.axis_chosen(index));
            self.stage_sentence_editor();
        }
        self
    }

    /// Return the app with one character inserted into the focused rewrite note.
    #[must_use]
    pub fn sentence_editor_typed(mut self, symbol: char) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.typed(symbol));
            self.stage_sentence_editor();
        }
        self
    }

    /// Return the app with one character removed from the focused rewrite note.
    #[must_use]
    pub fn sentence_editor_rubbed(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.rubbed());
            self.stage_sentence_editor();
        }
        self
    }

    /// Return the app with the focused rewrite-note cursor moved left.
    #[must_use]
    pub fn sentence_editor_cursor_left(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.cursor_left());
        }
        self
    }

    /// Return the app with the focused rewrite-note cursor moved right.
    #[must_use]
    pub fn sentence_editor_cursor_right(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            self.cards.editor = Some(editor.cursor_right());
        }
        self
    }

    fn sentence_editor_opened(mut self, row: LabelEditorRow) -> Self {
        if !self.card_tunable() {
            return self;
        }
        let Some(draft) = self.cards.drafts.get(self.cards.selected) else {
            return self;
        };
        let (baseline, selection, note) = sentence_editor_seed(draft);
        self.cards.editor = Some(SentenceLabelsEditor::new(baseline, selection, row, note));
        self.cards.expanded = true;
        self
    }

    fn stage_sentence_editor(&mut self) {
        let Some(editor) = self.cards.editor.as_ref() else {
            return;
        };
        let selection = editor.selection().clone();
        let note = editor.note().value().to_string();
        let Some(draft) = self.cards.drafts.get(self.cards.selected).cloned() else {
            return;
        };
        self.cards.drafts[self.cards.selected] = draft.staging_rewrite(selection, note);
    }

    /// Return the app with a new card session installed.
    pub fn cards_started(mut self, drafts: Vec<CardDraft>) -> Self {
        self.sentence_settings_row = None;
        self.cards = CardsView {
            drafts,
            selected: 0,
            expanded: false,
            elapsed: Duration::ZERO,
            running: None,
            editor: None,
        };
        self
    }

    /// Return the app with the currently-running artifact recorded so the UI can
    /// render an inline spinner instead of "queued".
    pub fn cards_running(mut self, target: Option<(usize, Artifact)>) -> Self {
        self.cards.running = target;
        self
    }

    /// Return which (card, artifact) pair is being worked on right now, if any.
    pub fn cards_running_target(&self) -> Option<(usize, Artifact)> {
        self.cards.running
    }

    /// Return the app with card drafts replaced while preserving UI cursor state.
    pub fn cards_replaced(mut self, drafts: Vec<CardDraft>) -> Self {
        let selected = self.cards.selected.min(drafts.len().saturating_sub(1));
        if selected != self.cards.selected || drafts.is_empty() {
            self.cards.editor = None;
        }
        self.cards.drafts = drafts;
        self.cards.selected = selected;
        if self.cards.drafts.is_empty() {
            self.cards.expanded = false;
        }
        self
    }

    /// Return the app with card cursor moved down (saturates).
    pub fn card_selected_next(mut self) -> Self {
        self.cards.editor = None;
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
        self.cards.editor = None;
        if self.cards.selected > 0 {
            self.cards.selected -= 1;
            self.cards.expanded = false;
        }
        self
    }

    /// Toggle the focused card, opening its editor immediately when it can be tuned.
    pub fn card_toggle_expanded(mut self) -> Self {
        if self.cards.expanded {
            self.cards.editor = None;
            self.cards.expanded = false;
            return self;
        }
        if self.card_tunable() {
            return self.sentence_editor_opened_for_register();
        }
        self.cards.expanded = true;
        self
    }

    /// Return the app with one card focused and expanded (clicked disclosure).
    pub fn card_revealed(mut self, card: usize) -> Self {
        if card < self.cards.drafts.len() {
            self.cards.editor = None;
            self.cards.selected = card;
            self.cards.expanded = false;
            if self.card_tunable() {
                return self.sentence_editor_opened_for_register();
            }
            self.cards.expanded = true;
        }
        self
    }

    /// Return the elapsed generation time shown on the Your Cards status line.
    pub fn elapsed(&self) -> Duration {
        self.cards.elapsed
    }

    /// Return the app with an updated generation elapsed time.
    pub fn with_elapsed(mut self, elapsed: Duration) -> Self {
        self.cards.elapsed = elapsed;
        self
    }

    /// Return the app with Done artifacts installed for the final screen.
    pub fn done_published(
        mut self,
        deck: impl Into<String>,
        report: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        self.done = DoneArtifacts {
            deck: deck.into(),
            report: report.into(),
            output: output.into(),
        };
        self
    }

    /// Return the Done-screen artifact list.
    pub fn done_artifacts(&self) -> &DoneArtifacts {
        &self.done
    }

    /// Return the app with stale published output paths cleared.
    pub fn publication_cleared(mut self) -> Self {
        self.done = DoneArtifacts::default();
        self
    }

    /// Return the app with failed artifacts and their blocked dependents reset
    /// so the session engine can re-enqueue them.
    pub fn cards_reset_failures(mut self) -> Self {
        self.done = DoneArtifacts::default();
        for draft in self.cards.drafts.iter_mut() {
            if !draft.artifacts().has_failed() {
                continue;
            }
            let artifacts = draft.artifacts();
            let meta_failed = artifacts.meta().failed_terminally();
            let scene_failed = artifacts.scene().failed_terminally();
            let picture_failed = artifacts.picture().failed_terminally();
            let sound_failed = artifacts.sound().failed_terminally();
            let meta = if meta_failed {
                artifacts.meta().clone().retry()
            } else {
                artifacts.meta().clone()
            };
            let scene = if meta_failed || scene_failed {
                artifacts.scene().clone().retry()
            } else {
                artifacts.scene().clone()
            };
            let picture = if meta_failed || scene_failed || picture_failed {
                artifacts.picture().clone().retry()
            } else {
                artifacts.picture().clone()
            };
            let sound = if meta_failed || sound_failed {
                artifacts.sound().clone().retry()
            } else {
                artifacts.sound().clone()
            };
            *draft = draft
                .clone()
                .with_artifacts(CardArtifacts::from_parts(meta, scene, picture, sound));
        }
        self
    }

    /// Return the app with a different artifact bundle installed for the focused card.
    pub fn card_patched_artifacts(mut self, artifacts: CardArtifacts) -> Self {
        if let Some(draft) = self.cards.drafts.get(self.cards.selected).cloned() {
            self.cards.drafts[self.cards.selected] = draft.with_artifacts(artifacts);
        }
        self
    }

    /// Return the app with a replacement draft installed for the focused card.
    pub fn card_replaced(mut self, draft: CardDraft) -> Self {
        if self.cards.selected < self.cards.drafts.len() {
            self.cards.drafts[self.cards.selected] = draft;
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
        self.review.expanded = None;
        self.review.notice = None;
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

    /// Return the app with one character inserted into the active text buffer.
    pub fn typed(mut self, symbol: char) -> Self {
        if self.modal.is_some() {
            self.input.modal.push(symbol);
        } else if self.screen == Screen::YourWords {
            let cursor = boundary_at_or_before(&self.input.blob, self.blob_cursor);
            self.input.blob.insert(cursor, symbol);
            self.blob_cursor = cursor + symbol.len_utf8();
        }
        self
    }

    /// Return the app with one character removed from the active text buffer.
    pub fn rubbed(mut self) -> Self {
        if self.modal.is_some() {
            self.input.modal.pop();
        } else if self.screen == Screen::YourWords {
            let cursor = boundary_at_or_before(&self.input.blob, self.blob_cursor);
            let previous = boundary_before(&self.input.blob, cursor);
            self.input.blob.replace_range(previous..cursor, "");
            self.blob_cursor = previous;
        }
        self
    }

    /// Return the app with the raw blob cursor moved one character left.
    pub fn cursor_left(mut self) -> Self {
        self.blob_cursor = boundary_before(&self.input.blob, self.blob_cursor);
        self
    }

    /// Return the app with the raw blob cursor moved one character right.
    pub fn cursor_right(mut self) -> Self {
        self.blob_cursor = cursor_forward(&mut self.input.blob, self.blob_cursor);
        self
    }

    /// Return the app with the raw blob cursor moved one visual row up.
    pub fn cursor_up(mut self) -> Self {
        self.blob_cursor = cursor_above(&mut self.input.blob, self.blob_cursor);
        self
    }

    /// Return the app with the raw blob cursor moved one visual row down.
    pub fn cursor_down(mut self) -> Self {
        self.blob_cursor = cursor_below(&mut self.input.blob, self.blob_cursor);
        self
    }

    /// Return the app with a brand new blob installed (used for clipboard paste).
    pub fn seeded_blob(mut self, blob: impl Into<String>) -> Self {
        self.input.blob = blob.into();
        self.blob_cursor = self.input.blob.len();
        self
    }

    /// Return the app with the blob wiped (used after successful submission).
    pub fn clear_blob(mut self) -> Self {
        self.input.blob.clear();
        self.blob_cursor = 0;
        self
    }
}

fn sentence_editor_seed(
    draft: &CardDraft,
) -> (SentenceLabelSelection, SentenceLabelSelection, NoteDraft) {
    let baseline = draft
        .meta()
        .and_then(|meta| meta.sentence_labels())
        .map(SentenceLabelSelection::from_labels)
        .unwrap_or_else(SentenceLabelSelection::empty);
    if let Some(rewrite) = draft.staged_rewrite() {
        return (
            baseline,
            rewrite.selection().clone(),
            NoteDraft::new(rewrite.note()),
        );
    }
    (baseline.clone(), baseline, NoteDraft::default())
}

fn boundary_at_or_before(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(cursor) {
        return cursor;
    }
    let mut boundary = 0;
    for (index, _) in text.char_indices() {
        if index > cursor {
            return boundary;
        }
        boundary = index;
    }
    text.len()
}

fn boundary_before(text: &str, cursor: usize) -> usize {
    let cursor = boundary_at_or_before(text, cursor);
    let mut boundary = 0;
    for (index, _) in text.char_indices() {
        if index >= cursor {
            return boundary;
        }
        boundary = index;
    }
    boundary
}

fn cursor_forward(text: &mut String, cursor: usize) -> usize {
    let cursor = boundary_at_or_before(text, cursor);
    if cursor >= text.len() {
        text.insert(cursor, ' ');
        return cursor + 1;
    }
    let mut characters = text[cursor..].chars();
    match characters.next() {
        Some(character) => cursor + character.len_utf8(),
        None => text.len(),
    }
}

fn cursor_above(text: &mut String, cursor: usize) -> usize {
    let starts = line_starts(text);
    let (row, column) = cursor_row_column(text, cursor);
    if row == 0 {
        return boundary_at_or_before(text, cursor);
    }
    cursor_for_column(text, starts[row - 1], column)
}

fn cursor_below(text: &mut String, cursor: usize) -> usize {
    let cursor = boundary_at_or_before(text, cursor);
    let (row, column) = cursor_row_column(text, cursor);
    let starts = line_starts(text);
    let next = row + 1;
    if next >= starts.len() {
        let end = line_end(text, cursor);
        text.insert(end, '\n');
        return cursor_for_column(text, end + 1, column);
    }
    cursor_for_column(text, starts[next], column)
}

fn cursor_row_column(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = boundary_at_or_before(text, cursor);
    let mut row = 0;
    let mut column = 0;
    for (index, character) in text.char_indices() {
        if index >= cursor {
            return (row, column);
        }
        if character == '\n' {
            row += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    (row, column)
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, character) in text.char_indices() {
        if character == '\n' {
            starts.push(index + character.len_utf8());
        }
    }
    starts
}

fn cursor_for_column(text: &mut String, start: usize, column: usize) -> usize {
    let end = line_end(text, start);
    for (seen, (offset, _)) in text[start..end].char_indices().enumerate() {
        if seen == column {
            return start + offset;
        }
    }
    let missing = column.saturating_sub(text[start..end].chars().count());
    text.insert_str(end, &" ".repeat(missing));
    end + missing
}

fn line_end(text: &str, start: usize) -> usize {
    for (offset, character) in text[start..].char_indices() {
        if character == '\n' {
            return start + offset;
        }
    }
    text.len()
}

fn step_focus(current: WelcomeFocus, env_available: bool, direction: i32) -> WelcomeFocus {
    let order: &[WelcomeFocus] = if env_available {
        &[WelcomeFocus::Submit, WelcomeFocus::LoadEnv]
    } else {
        &[WelcomeFocus::Submit]
    };
    let position = order.iter().position(|item| *item == current).unwrap_or(0) as i32;
    let next = (position + direction).rem_euclid(order.len() as i32) as usize;
    order[next]
}

fn artifact_hint(artifacts: &CardArtifacts, kind: Artifact) -> &'static str {
    let slot = match kind {
        Artifact::Meta => artifacts.meta(),
        Artifact::Scene => artifacts.scene(),
        Artifact::Picture => artifacts.picture(),
        Artifact::Sound => artifacts.sound(),
    };
    if slot.ready() {
        return "ready";
    }
    if slot.discarded() {
        return "discarded";
    }
    if slot.failed_terminally() {
        return "failed";
    }
    if slot.tally().done() > 0 {
        return "retrying";
    }
    "queued"
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::session::{
        AxisSet, CardDraft, CardMeta, LanguagePair, Register, SentenceAxis, SentenceBatchSettings,
        SentenceKind, SentenceLabelSelection, SentenceLabels, SentenceLevel, SentenceTypeMix,
    };
    use crate::tui::{BatchSettingsRow, LabelEditorRow, Screen};

    fn generated(term: &str, understanding: &str, pair: LanguagePair) -> CardDraft {
        CardDraft::new(term, understanding, pair).with_meta(
            CardMeta::new(
                format!("/{term}/"),
                format!("/{term} sentence/"),
                format!("meaning of {term}"),
                5,
                format!("source with {term}"),
                term,
                format!("hint for {term}"),
                format!("context for {term}"),
                format!("Example with {term}."),
            )
            .with_sentence_labels(SentenceLabels::new(
                Register::Neutral,
                SentenceLevel::B1,
                SentenceKind::Statement,
                AxisSet::default(),
                AxisSet::default(),
            )),
            None,
        )
    }

    fn cards() -> App {
        let pair = LanguagePair::new("fr", "en");
        App::new(pair.clone()).cards_started(vec![
            generated("flâner", "to stroll", pair.clone()),
            generated("canard", "a duck", pair),
        ])
    }

    #[test]
    fn screen_changes_close_batch_settings_without_losing_their_choices() {
        let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Varied);
        let next = App::new(LanguagePair::new("fr", "en"))
            .with_sentence_settings(settings)
            .sentence_settings_opened()
            .with_screen(Screen::YourWords);
        assert_eq!(
            (next.sentence_settings(), next.sentence_settings_editor()),
            (settings, None),
            "changing screens lost batch sentence choices or kept their editor open"
        );
    }

    #[test]
    fn starting_a_new_batch_resets_sentence_settings() {
        let next = App::new(LanguagePair::new("fr", "en"))
            .with_sentence_settings(SentenceBatchSettings::new(
                Some(SentenceLevel::B1),
                SentenceTypeMix::Varied,
            ))
            .sentence_settings_opened()
            .starting_new_batch();
        assert_eq!(
            (next.sentence_settings(), next.sentence_settings_editor()),
            (SentenceBatchSettings::default(), None),
            "a clean batch inherited sentence settings from the previous one"
        );
    }

    #[test]
    fn opening_batch_settings_focuses_level_and_closes_the_sense_picker() {
        let next = App::new(LanguagePair::new("fr", "en"))
            .understood(vec![crate::session::WordCandidate::new(
                "canard", "a duck", true,
            )])
            .senses_expanded()
            .sentence_settings_opened();
        assert_eq!(
            (next.sentence_settings_editor(), next.expanded_sense()),
            (Some(BatchSettingsRow::Level), None),
            "opening sentence settings left another review layer open"
        );
    }

    #[test]
    fn closing_the_editor_retains_the_live_pending_note() {
        let opened = cards().sentence_editor_opened_for_note();
        let changed = opened.clone().sentence_editor_typed('x');
        let closed = changed.clone().sentence_editor_closed();
        assert_eq!(
            (
                opened.card_expanded(),
                opened.sentence_editor().map(|editor| editor.row()),
                opened.cards()[0].rewrite(),
                changed.cards()[0].rewrite().map(|rewrite| rewrite.note()),
                closed.sentence_editor(),
                closed.cards()[0].rewrite().map(|rewrite| rewrite.note()),
            ),
            (
                true,
                Some(LabelEditorRow::Note),
                None,
                Some("x"),
                None,
                Some("x")
            ),
            "closing the live editor rolled its pending note back"
        );
    }

    #[test]
    fn every_chip_and_note_edit_updates_the_selected_pending_draft() {
        let changed = cards()
            .sentence_editor_opened_for_register()
            .sentence_editor_axis_chosen(1)
            .sentence_editor_focused(LabelEditorRow::Note)
            .sentence_editor_typed('é');
        let rewrite = changed.cards()[0]
            .rewrite()
            .expect("the selected card must carry its rewrite");
        assert_eq!(
            (
                rewrite.selection().register(),
                rewrite
                    .selection()
                    .pinned()
                    .contains(SentenceAxis::Register),
                rewrite.note(),
                changed.sentence_editor().is_some(),
                changed.card_expanded(),
            ),
            (Some(Register::Casual), true, "é", true, true),
            "live editing lost its label pin, note, or expanded editor"
        );
    }

    #[test]
    fn card_navigation_closes_the_editor() {
        let navigated = cards()
            .sentence_editor_opened_for_note()
            .card_selected_next();
        assert_eq!(
            (
                navigated.card_selected(),
                navigated.card_expanded(),
                navigated.sentence_editor(),
            ),
            (1, false, None),
            "card navigation kept an editor attached to the previous selection"
        );
    }

    #[test]
    fn reopening_a_queued_rewrite_restores_its_note_and_selection() {
        let draft = generated("flâner", "to stroll", LanguagePair::new("fr", "en"))
            .staging_rewrite(
                SentenceLabelSelection::empty().choosing(SentenceAxis::Register, 2),
                "shorter",
            );
        let opened = App::new(LanguagePair::new("fr", "en"))
            .cards_started(vec![draft])
            .sentence_editor_opened_for_register();
        let editor = opened
            .sentence_editor()
            .expect("the queued rewrite editor must open");
        assert_eq!(
            (
                editor.selection().register(),
                editor.note().value(),
                editor.row(),
            ),
            (Some(Register::Formal), "shorter", LabelEditorRow::Register),
            "reopening a queued rewrite lost its pending request"
        );
    }
}
