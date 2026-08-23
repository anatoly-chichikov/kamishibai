use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::application::LearningTarget;
use crate::session::{
    Artifact, CardArtifacts, CardDraft, CardPhase, LanguagePair, Sense, SentenceBatchSettings,
    WordCandidate,
};

use super::picker::{LanguageChoice, PickerCursor, PickerSection};
use super::screen::{KeySource, ModalKind, Screen, WelcomeFocus, WelcomeStage};
use super::sentence_editor::{BatchSettingsRow, LabelEditorRow, SentenceLabelsEditor};

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
    word_clear_pending: bool,
    picker_cursor: PickerCursor,
    learning_target: LearningTarget,
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
    pub expanded: ExpandedCards,
    pub elapsed: Duration,
    pub running: Option<(usize, Artifact)>,
    editor: Option<SentenceLabelsEditor>,
    stop: GenerationStopState,
    following: bool,
}

/// The cards whose blocks stay expanded while focus walks elsewhere.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExpandedCards(BTreeSet<usize>);

impl ExpandedCards {
    /// Return whether one card's block is expanded.
    #[must_use]
    pub fn contains(&self, card: usize) -> bool {
        self.0.contains(&card)
    }

    /// Return whether no card is expanded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn with(mut self, card: usize) -> Self {
        self.0.insert(card);
        self
    }

    fn without(mut self, card: usize) -> Self {
        self.0.remove(&card);
        self
    }

    fn retained(mut self, len: usize) -> Self {
        self.0.retain(|card| *card < len);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum GenerationStopState {
    #[default]
    Inactive,
    Pending,
    Stopping,
    Cancelling,
}

/// How many cards of the batch stand in each phase.
///
/// `ready`, `failed` and `adjusted` keep the counts the status line has always
/// shown and may overlap: a card whose artifacts are all present still counts
/// as ready while it carries a staged rewrite. `working` is the exclusive
/// remainder and stays internal: the status line derives progress from `ready`
/// against the batch size, so the count only answers whether any card is left
/// unfinished.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CardCensus {
    pub ready: usize,
    working: usize,
    pub failed: usize,
    pub adjusted: usize,
}

impl CardCensus {
    /// Return whether any card still owes work — building, failed, or adjusted.
    #[must_use]
    pub fn unfinished(&self) -> bool {
        self.working > 0 || self.failed > 0 || self.adjusted > 0
    }

    /// Return the census with one more card folded in.
    #[must_use]
    fn counting(mut self, draft: &CardDraft) -> Self {
        let artifacts = draft.artifacts();
        self.ready += usize::from(artifacts.all_ready());
        self.failed += usize::from(artifacts.has_failed());
        self.adjusted += usize::from(draft.staged_rewrite().is_some());
        self.working += usize::from(draft.phase() == CardPhase::Working);
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DoneArtifacts {
    pub deck: String,
    pub report: String,
    pub output: String,
    pub cards: usize,
    pub failed: usize,
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
    pub focus: ReviewFocus,
    pub open: OpenSenseLists,
    pub notice: Option<String>,
    /// Supported codes the understanding pass judged equally plausible for
    /// this batch, excluding the one it chose. Empty when the batch is
    /// unambiguous, and empty for cache entries written before the pass
    /// started reporting them.
    pub alternates: Vec<String>,
}

/// One position on the continuous review walk: a candidate head, or one row
/// inside that candidate's open sense list, where `index == senses.len()` is
/// the list's add-more row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewFocus {
    Head(usize),
    Sense { row: usize, index: usize },
}

impl ReviewFocus {
    /// Return the candidate row this focus position belongs to.
    #[must_use]
    pub fn row(self) -> usize {
        match self {
            Self::Head(row) | Self::Sense { row, .. } => row,
        }
    }
}

impl Default for ReviewFocus {
    fn default() -> Self {
        Self::Head(0)
    }
}

/// The candidate rows whose sense lists stay open inline on the review walk.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OpenSenseLists(BTreeSet<usize>);

impl OpenSenseLists {
    /// Return whether one candidate row's sense list is open.
    #[must_use]
    pub fn contains(&self, row: usize) -> bool {
        self.0.contains(&row)
    }

    /// Return whether no sense list is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn toggled(mut self, row: usize) -> Self {
        if !self.0.remove(&row) {
            self.0.insert(row);
        }
        self
    }

    fn with(mut self, row: usize) -> Self {
        self.0.insert(row);
        self
    }

    fn without(mut self, row: usize) -> Self {
        self.0.remove(&row);
        self
    }

    fn dropped_row(self, row: usize) -> Self {
        Self(
            self.0
                .into_iter()
                .filter(|open| *open != row)
                .map(|open| if open > row { open - 1 } else { open })
                .collect(),
        )
    }
}

/// Build one language pair, preserving the case each code arrived in.
///
/// Codes reach the app in several cases — the catalog hands out lowercase,
/// `LanguagePair` from a cards document is uppercase, `LanguageCode` from a pin
/// is uppercase, and Gemini answers in whatever it likes — and the record each
/// case round-trips into is the console's, not this app's, to recase. So the
/// pair carries codes verbatim and every comparison against it is
/// case-insensitive instead.
fn paired(learning: &str, known: &str) -> LanguagePair {
    LanguagePair::new(learning.to_string(), known.to_string())
}

impl App {
    /// Create a fresh app sitting on `YourWords` with an initial language pair.
    pub fn new(pair: LanguagePair) -> Self {
        let pair = paired(pair.learning(), pair.known());
        let picker_cursor = PickerCursor::opening(pair.known(), None, PickerSection::Known);
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
            word_clear_pending: false,
            picker_cursor,
            learning_target: LearningTarget::Detect,
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

    /// Return whether a first Escape has armed clearing the words field.
    pub fn word_clear_pending(&self) -> bool {
        self.word_clear_pending
    }

    /// Return the app with the words-clear confirmation flag updated.
    pub fn with_word_clear_pending(mut self, pending: bool) -> Self {
        self.word_clear_pending = pending;
        self
    }

    /// Return whether a first Escape has armed stopping card generation.
    pub fn generation_stop_pending(&self) -> bool {
        self.cards.stop == GenerationStopState::Pending
    }

    /// Return whether the shell is draining the last in-flight artifact.
    pub fn generation_stopping(&self) -> bool {
        matches!(
            self.cards.stop,
            GenerationStopState::Stopping | GenerationStopState::Cancelling
        )
    }

    /// Return whether a stopped run is waiting for durable cancellation.
    pub fn generation_cancelling(&self) -> bool {
        self.cards.stop == GenerationStopState::Cancelling
    }

    /// Return the app with generation-stop confirmation armed or disarmed.
    pub fn with_generation_stop_pending(mut self, pending: bool) -> Self {
        if pending {
            self.cards.stop = GenerationStopState::Pending;
        } else if self.cards.stop == GenerationStopState::Pending {
            self.cards.stop = GenerationStopState::Inactive;
        }
        self
    }

    /// Return the app while it drains the current artifact before stopping.
    pub fn generation_stop_started(mut self) -> Self {
        self.cards.stop = GenerationStopState::Stopping;
        self
    }

    /// Return the app while its stopped run is being closed without publication.
    pub fn generation_cancellation_started(mut self) -> Self {
        self.cards.stop = GenerationStopState::Cancelling;
        self
    }

    /// Return the app with all transient generation-stop state cleared.
    pub fn generation_stop_finished(mut self) -> Self {
        self.cards.stop = GenerationStopState::Inactive;
        self
    }

    /// Return whether a finished batch can be replaced from the final screen.
    pub fn can_start_new_batch(&self) -> bool {
        matches!(self.screen, Screen::YourCards | Screen::Done)
            && self.batch_settled()
            && self.modal.is_none()
            && self.busy.is_none()
            && self.error.is_none()
            && self.cards.editor.is_none()
            && !self.cards.expanded.contains(self.cards.selected)
    }

    /// Return whether generation has reached a terminal or published view.
    pub fn batch_settled(&self) -> bool {
        !self.done.deck.is_empty()
            || (!self.cards.drafts.is_empty()
                && self
                    .cards
                    .drafts
                    .iter()
                    .all(|draft| draft.artifacts().all_ready() || draft.artifacts().has_failed()))
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

    /// Return the app with the last character of the API key removed.
    ///
    /// Backspace used to wipe the whole field, which is the one place in the
    /// app where a single ordinary keystroke destroyed a whole entry — and on
    /// the setup screen, where a mistyped key is exactly what you are trying to
    /// correct. It rubs out one character, like every other field.
    pub fn welcome_rubbed_key(mut self) -> Self {
        self.welcome.key.pop();
        if self.welcome.key.is_empty() {
            self.welcome.source = KeySource::Empty;
        }
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
        self.cards.following = false;
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

    /// Return the chip highlighted in each half of the language picker modal
    /// plus the focused half. Meaningful only while `PickLanguages` is open.
    pub fn picker_cursor(&self) -> PickerCursor {
        self.picker_cursor
    }

    /// Return the app with the picker cursor replaced. Used when opening the
    /// modal so the active pair is pre-selected on the requested half.
    pub fn with_picker_cursor(mut self, cursor: PickerCursor) -> Self {
        self.picker_cursor = cursor;
        self
    }

    /// Return the app with the picker cursor advanced by `delta`, wrapping
    /// around the focused half only.
    pub fn picker_cursor_advanced(mut self, delta: i32) -> Self {
        self.picker_cursor = self.picker_cursor.advanced(delta);
        self
    }

    /// Return the app with the picker focused on one half of the pair.
    pub fn picker_facing(mut self, section: PickerSection) -> Self {
        self.picker_cursor = self.picker_cursor.facing(section);
        self
    }

    /// Return the app with one picker half focused and its chip highlighted.
    pub fn picker_chosen(mut self, section: PickerSection, index: usize) -> Self {
        self.picker_cursor = self.picker_cursor.chosen(section, index);
        self
    }

    /// Return how this batch decides the language being learned.
    pub fn learning_target(&self) -> &LearningTarget {
        &self.learning_target
    }

    /// Return the pinned learning code, or `None` while detection is in charge.
    pub fn learning_pin(&self) -> Option<&str> {
        match &self.learning_target {
            LearningTarget::Detect => None,
            LearningTarget::Explicit(code) => Some(code.as_ref()),
        }
    }

    /// Return the app with one confirmed language pair adopted.
    ///
    /// A pinned learning code lands in the pair immediately — it is a decision,
    /// not a guess, so the header stops showing the pending ellipsis. Handing
    /// the half back to detection reopens that ellipsis until the pass answers.
    pub fn languages_adopted(mut self, choice: &LanguageChoice) -> Self {
        self.learning_target = choice.learning().clone();
        match choice.pinned() {
            Some(code) => {
                self.pair = paired(code, choice.known());
                self.input.learning_pending = false;
            }
            None => {
                self.pair = paired(self.pair.learning(), choice.known());
                self.input.learning_pending = true;
            }
        }
        self.picker_cursor = PickerCursor::opening(
            self.pair.known(),
            choice.pinned(),
            self.picker_cursor.section(),
        );
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
        self.pair = paired(self.pair.learning(), code.into().as_str());
        self
    }

    /// Return the app with a confirmed learning language guess from the LLM pass.
    pub fn confirmed_learning(mut self, code: impl Into<String>) -> Self {
        self.pair = paired(code.into().as_str(), self.pair.known());
        self.input.learning_pending = false;
        self
    }

    /// Return the confirmed candidates to be reviewed.
    pub fn candidates(&self) -> &[WordCandidate] {
        self.review.candidates.as_slice()
    }

    /// Return the candidate row the review walk currently stands on.
    pub fn selected(&self) -> usize {
        self.review.focus.row()
    }

    /// Return the current position of the continuous review walk.
    #[must_use]
    pub fn review_focus(&self) -> ReviewFocus {
        self.review.focus
    }

    /// Return whether one candidate row's sense list is open inline.
    #[must_use]
    pub fn sense_list_open(&self, row: usize) -> bool {
        self.review.open.contains(row)
    }

    /// Return whether any candidate row holds an open sense list.
    #[must_use]
    pub fn any_sense_list_open(&self) -> bool {
        !self.review.open.is_empty()
    }

    /// Return whether the walk stands on or inside an open sense list.
    #[must_use]
    pub fn focused_sense_list_open(&self) -> bool {
        self.review.open.contains(self.review.focus.row())
    }

    /// Return the short review notice, if any.
    pub fn review_notice(&self) -> Option<&str> {
        self.review.notice.as_deref()
    }

    /// Return the app carrying one short review notice.
    #[must_use]
    pub fn review_noticed(mut self, message: impl Into<String>) -> Self {
        self.review.notice = Some(message.into());
        self
    }

    /// Return how many cards the confirmed sense selection would commit.
    #[must_use]
    pub fn review_cards(&self) -> usize {
        self.review
            .candidates
            .iter()
            .filter(|candidate| candidate.ok())
            .map(WordCandidate::selected_count)
            .sum()
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

    /// Return the focused row of the open generation-guidance editor.
    #[must_use]
    pub fn sentence_settings_editor(&self) -> Option<BatchSettingsRow> {
        self.sentence_settings_row
    }

    /// Return the app with generation guidance open on the level row.
    #[must_use]
    pub fn sentence_settings_opened(mut self) -> Self {
        self.sentence_settings_row = Some(BatchSettingsRow::Level);
        self.review.notice = None;
        self
    }

    /// Return the app with generation guidance closed and choices retained.
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
            focus: ReviewFocus::Head(0),
            open: OpenSenseLists::default(),
            notice: None,
            alternates: Vec::new(),
        };
        self
    }

    /// Return the app with the equally plausible learning languages this pass
    /// reported. A pinned batch shows none: the user already decided.
    pub fn with_alternates(mut self, alternates: Vec<String>) -> Self {
        self.review.alternates = match self.learning_target {
            LearningTarget::Detect => alternates,
            LearningTarget::Explicit(_) => Vec::new(),
        };
        self
    }

    /// Return the learning languages the pass judged equally plausible.
    pub fn alternates(&self) -> &[String] {
        self.review.alternates.as_slice()
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
        let row = self
            .review
            .focus
            .row()
            .min(candidates.len().saturating_sub(1));
        let alternates = std::mem::take(&mut self.review.alternates);
        self.review = Review {
            candidates,
            focus: ReviewFocus::Head(row),
            open: OpenSenseLists::default(),
            notice: None,
            alternates,
        };
        self
    }

    /// Return whether the focused row can open a sense picker.
    pub fn selected_can_expand_senses(&self) -> bool {
        self.review
            .candidates
            .get(self.review.focus.row())
            .map(WordCandidate::ok)
            .unwrap_or(false)
    }

    /// Return the app with the focused head's sense list opened or closed.
    #[must_use]
    pub fn sense_list_toggled(mut self) -> Self {
        let row = self.review.focus.row();
        if !self.selected_can_expand_senses() {
            return self;
        }
        self.review.open = self.review.open.toggled(row);
        self.review.focus = ReviewFocus::Head(row);
        self.review.notice = None;
        self
    }

    /// Return the app with the focused row's sense list closed.
    #[must_use]
    pub fn sense_list_closed(mut self) -> Self {
        let row = self.review.focus.row();
        self.review.open = self.review.open.without(row);
        self.review.focus = ReviewFocus::Head(row);
        self.review.notice = None;
        self
    }

    /// Return the app with every open sense list collapsed.
    #[must_use]
    pub fn sense_lists_collapsed(mut self) -> Self {
        self.review.open = OpenSenseLists::default();
        self.review.focus = ReviewFocus::Head(self.review.focus.row());
        self
    }

    /// Return the app with every multi-meaning row's sense list opened;
    /// single-sense and off-language rows stay closed.
    #[must_use]
    pub fn sense_lists_expanded_all(mut self) -> Self {
        self.review.open = OpenSenseLists(
            self.review
                .candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| candidate.ok() && candidate.has_multiple_senses())
                .map(|(row, _)| row)
                .collect(),
        );
        self.review.focus = ReviewFocus::Head(self.review.focus.row());
        self
    }

    /// Return the app with the review walk moved one position down, entering
    /// and leaving open sense lists without closing them.
    #[must_use]
    pub fn review_focus_next(mut self) -> Self {
        let total = self.review.candidates.len();
        if total == 0 {
            return self;
        }
        self.review.notice = None;
        self.review.focus = match self.review.focus {
            ReviewFocus::Head(row) if self.review.open.contains(row) => {
                ReviewFocus::Sense { row, index: 0 }
            }
            ReviewFocus::Head(row) if row + 1 < total => ReviewFocus::Head(row + 1),
            ReviewFocus::Head(row) => ReviewFocus::Head(row),
            ReviewFocus::Sense { row, index } => {
                let last = self
                    .review
                    .candidates
                    .get(row)
                    .map(|candidate| candidate.senses().len())
                    .unwrap_or(0);
                if index < last {
                    ReviewFocus::Sense {
                        row,
                        index: index + 1,
                    }
                } else if row + 1 < total {
                    ReviewFocus::Head(row + 1)
                } else {
                    ReviewFocus::Sense { row, index }
                }
            }
        };
        self
    }

    /// Return the app with the review walk moved one position up, entering the
    /// previous row's open sense list at its add-more row.
    #[must_use]
    pub fn review_focus_previous(mut self) -> Self {
        self.review.notice = None;
        self.review.focus = match self.review.focus {
            ReviewFocus::Head(row) if row > 0 => {
                let previous = row - 1;
                if self.review.open.contains(previous) {
                    let last = self
                        .review
                        .candidates
                        .get(previous)
                        .map(|candidate| candidate.senses().len())
                        .unwrap_or(0);
                    ReviewFocus::Sense {
                        row: previous,
                        index: last,
                    }
                } else {
                    ReviewFocus::Head(previous)
                }
            }
            ReviewFocus::Head(row) => ReviewFocus::Head(row),
            ReviewFocus::Sense { row, index: 0 } => ReviewFocus::Head(row),
            ReviewFocus::Sense { row, index } => ReviewFocus::Sense {
                row,
                index: index - 1,
            },
        };
        self
    }

    /// Return the app with the focused sense toggled, committed immediately
    /// into the candidate; the last selected sense cannot be deselected.
    pub fn sense_toggled(mut self) -> Self {
        let ReviewFocus::Sense { row, index } = self.review.focus else {
            return self;
        };
        let Some(candidate) = self.review.candidates.get(row).cloned() else {
            return self;
        };
        if index >= candidate.senses().len() {
            return self;
        }
        let mut selected = candidate.selected_senses().to_vec();
        if let Some(position) = selected.iter().position(|chosen| *chosen == index) {
            if selected.len() > 1 {
                selected.remove(position);
            }
        } else {
            selected.push(index);
            selected.sort_unstable();
        }
        self.review.candidates[row] = candidate.selecting_senses(selected);
        self.review.notice = None;
        self
    }

    /// Return whether the review walk stands on an open list's add-more row.
    pub fn expanded_add_more_focused(&self) -> bool {
        let ReviewFocus::Sense { row, index } = self.review.focus else {
            return false;
        };
        self.review
            .candidates
            .get(row)
            .map(|candidate| index >= candidate.senses().len())
            .unwrap_or(false)
    }

    /// Return the app with new senses appended to the focused row.
    pub fn senses_appended_to_selected(
        mut self,
        senses: Vec<Sense>,
        message: Option<String>,
    ) -> Self {
        let row = self.review.focus.row();
        let Some(candidate) = self.review.candidates.get(row).cloned() else {
            return self;
        };
        let (candidate, first) = candidate.with_added_senses(senses);
        self.review.candidates[row] = candidate;
        self.review.notice = if first.is_none() && message.is_none() {
            Some(String::from("nothing to add"))
        } else {
            message
        };
        if let Some(index) = first {
            self.review.open = self.review.open.with(row);
            self.review.focus = ReviewFocus::Sense { row, index };
        }
        self
    }

    /// Return the current card drafts for the Your Cards screen.
    pub fn cards(&self) -> &[CardDraft] {
        self.cards.drafts.as_slice()
    }

    /// Return how many cards stand in each phase, in one pass over the batch.
    #[must_use]
    pub fn card_census(&self) -> CardCensus {
        self.cards
            .drafts
            .iter()
            .fold(CardCensus::default(), CardCensus::counting)
    }

    /// Return how many cards carry a live pending rewrite.
    #[must_use]
    pub fn cards_pending(&self) -> usize {
        self.card_census().adjusted
    }

    /// Return whether the focused card can open its sentence-label editor.
    #[must_use]
    pub fn card_tunable(&self) -> bool {
        self.card_tunable_at(self.cards.selected)
    }

    /// Return whether one card can open its sentence-label editor.
    #[must_use]
    pub fn card_tunable_at(&self, card: usize) -> bool {
        self.cards.drafts.get(card).is_some_and(CardDraft::tunable)
    }

    /// Return the currently focused card index.
    pub fn card_selected(&self) -> usize {
        self.cards.selected
    }

    /// Return whether the focused card is expanded.
    pub fn card_expanded(&self) -> bool {
        self.cards.expanded.contains(self.cards.selected)
    }

    /// Return whether one card's block is expanded, focused or parked.
    #[must_use]
    pub fn card_expanded_at(&self, card: usize) -> bool {
        self.cards.expanded.contains(card)
    }

    /// Return whether any card holds an expanded block.
    #[must_use]
    pub fn any_card_expanded(&self) -> bool {
        !self.cards.expanded.is_empty()
    }

    /// Return the app with every expanded card collapsed.
    #[must_use]
    pub fn cards_collapsed(mut self) -> Self {
        self.cards.editor = None;
        self.cards.expanded = ExpandedCards::default();
        self
    }

    /// Return the app with every card's block expanded, editors closed.
    #[must_use]
    pub fn cards_expanded_all(mut self) -> Self {
        self.cards.editor = None;
        self.cards.expanded = ExpandedCards((0..self.cards.drafts.len()).collect());
        self
    }

    /// Return the app with the editor parked: closed while its card stays
    /// expanded, every staged edit already living in the draft.
    #[must_use]
    pub fn sentence_editor_parked(mut self) -> Self {
        self.cards.editor = None;
        self
    }

    /// Return the app with the card walk moved one position down: from an
    /// expanded head into its own editor, through the editor's rows, then out
    /// onto the next card head, parking the editor without collapsing its
    /// card. The walk saturates inside the last card's editor.
    #[must_use]
    pub fn card_focus_next(mut self) -> Self {
        let last = self.cards.drafts.len().saturating_sub(1);
        if let Some(editor) = self.cards.editor.take() {
            if editor.row() != LabelEditorRow::Note {
                self.cards.editor = Some(editor.row_next());
                return self;
            }
            if self.cards.selected >= last {
                self.cards.editor = Some(editor);
                return self;
            }
        } else if self.card_expanded() && self.card_tunable() {
            return self.sentence_editor_opened_for_register();
        }
        self.cards.following = false;
        if !self.cards.drafts.is_empty() && self.cards.selected < last {
            self.cards.selected += 1;
        }
        self
    }

    /// Return the app with the card walk moved one position up: through the
    /// open editor's rows onto this card's own head, and from a head into the
    /// previous card — entering a parked card's editor at its note row.
    #[must_use]
    pub fn card_focus_previous(mut self) -> Self {
        if let Some(editor) = self.cards.editor.take() {
            if editor.row() != LabelEditorRow::Register {
                self.cards.editor = Some(editor.row_previous());
                return self;
            }
            self.cards.following = false;
            return self;
        }
        self.cards.following = false;
        if self.cards.selected > 0 {
            self.cards.selected -= 1;
            if self.card_expanded() && self.card_tunable() {
                return self.sentence_editor_opened_for_note();
            }
        }
        self
    }

    /// Return the open sentence-label editor for the focused card.
    #[must_use]
    pub fn sentence_editor(&self) -> Option<&SentenceLabelsEditor> {
        self.cards.editor.as_ref()
    }

    /// Return the app with the focused card expanded and its note row editing.
    #[must_use]
    pub fn sentence_editor_opened_for_note(self) -> Self {
        self.sentence_editor_opened_for(LabelEditorRow::Note)
    }

    /// Return the app with the focused card expanded and its register row editing.
    #[must_use]
    pub fn sentence_editor_opened_for_register(self) -> Self {
        self.sentence_editor_opened_for(LabelEditorRow::Register)
    }

    /// Return the app with the sentence-label editor and focused card collapsed.
    #[must_use]
    pub fn sentence_editor_closed(mut self) -> Self {
        self.cards.editor = None;
        self.cards.expanded = std::mem::take(&mut self.cards.expanded).without(self.cards.selected);
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

    /// Return the app with the focused card expanded and its editor live on one
    /// row. Expansion alone only displays those rows — this is what hands them
    /// the keyboard, and the walk (`↓` from the head, `↑` from below) and a
    /// click on one of the controls are what go through it.
    #[must_use]
    pub fn sentence_editor_opened_for(mut self, row: LabelEditorRow) -> Self {
        if !self.card_tunable() {
            return self;
        }
        let Some(draft) = self.cards.drafts.get(self.cards.selected) else {
            return self;
        };
        self.cards.editor = Some(SentenceLabelsEditor::seeded(draft, row));
        self.cards.expanded = std::mem::take(&mut self.cards.expanded).with(self.cards.selected);
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
            expanded: ExpandedCards::default(),
            elapsed: Duration::ZERO,
            running: None,
            editor: None,
            stop: GenerationStopState::Inactive,
            following: true,
        };
        self
    }

    /// Return the app with the currently-running artifact recorded so the UI can
    /// render an inline spinner instead of "queued". While the view still
    /// follows the engine and no card is open, the selection rides along, which
    /// is what lets the viewport stay on the card being built.
    pub fn cards_running(mut self, target: Option<(usize, Artifact)>) -> Self {
        self.cards.running = target;
        if let Some((card, _)) = target
            && self.cards.following
            && !self.cards.expanded.contains(self.cards.selected)
            && card < self.cards.drafts.len()
        {
            self.cards.selected = card;
        }
        self
    }

    /// Return the app with the viewport riding the engine again.
    #[must_use]
    pub fn cards_following(mut self) -> Self {
        self.cards.following = true;
        self
    }

    /// Return the card the viewport should ride while the batch builds.
    ///
    /// Following is suppressed rather than cleared while a card is open, so
    /// closing that card resumes the ride. An open sentence editor implies an
    /// expanded card, so this one check covers both.
    #[must_use]
    pub fn following_card(&self) -> Option<usize> {
        if !self.cards.following || self.card_expanded() {
            return None;
        }
        self.cards.running.map(|(card, _)| card)
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
        self.cards.expanded =
            std::mem::take(&mut self.cards.expanded).retained(self.cards.drafts.len());
        self
    }

    /// Return the app with the card cursor on the next card that is not finished,
    /// wrapping past the end of the batch. Unfinished means any phase but
    /// `CardPhase::Ready`, so a card carrying a staged rewrite is a stop on the
    /// walk. Leaves the app untouched when every card is finished.
    #[must_use]
    pub fn card_jumped(mut self, forward: bool) -> Self {
        let total = self.cards.drafts.len();
        if total == 0 {
            return self;
        }
        let step = if forward { 1 } else { total - 1 };
        let Some(next) = (1..=total)
            .map(|offset| (self.cards.selected + offset * step) % total)
            .find(|index| self.cards.drafts[*index].phase() != CardPhase::Ready)
        else {
            return self;
        };
        self.cards.editor = None;
        self.cards.following = false;
        self.cards.selected = next;
        self
    }

    /// Toggle the focused card, opening its editor immediately when it can be tuned.
    pub fn card_toggle_expanded(mut self) -> Self {
        let selected = self.cards.selected;
        if self.cards.expanded.contains(selected) {
            self.cards.editor = None;
            self.cards.expanded = std::mem::take(&mut self.cards.expanded).without(selected);
            return self;
        }
        self.cards.expanded = std::mem::take(&mut self.cards.expanded).with(selected);
        self
    }

    /// Return the app with one card focused and expanded (clicked disclosure).
    pub fn card_revealed(mut self, card: usize) -> Self {
        if card < self.cards.drafts.len() {
            self.cards.editor = None;
            self.cards.selected = card;
            self.cards.expanded = std::mem::take(&mut self.cards.expanded).with(card);
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
        self,
        deck: impl Into<String>,
        report: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        let failed = self.cards_failed();
        let cards = self.cards.drafts.len().saturating_sub(failed);
        self.done_published_counted(deck, report, output, cards, failed)
    }

    /// Return the app with published paths and their durable card tally installed.
    pub fn done_published_counted(
        mut self,
        deck: impl Into<String>,
        report: impl Into<String>,
        output: impl Into<String>,
        cards: usize,
        failed: usize,
    ) -> Self {
        self.done = DoneArtifacts {
            deck: deck.into(),
            report: report.into(),
            output: output.into(),
            cards,
            failed,
        };
        self.cards.stop = GenerationStopState::Inactive;
        self
    }

    /// Return the Done-screen artifact list.
    pub fn done_artifacts(&self) -> &DoneArtifacts {
        &self.done
    }

    /// Return the app with stale published output paths cleared.
    pub fn publication_cleared(mut self) -> Self {
        self.done = DoneArtifacts::default();
        self.cards.stop = GenerationStopState::Inactive;
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
        self.card_census().ready
    }

    /// Return the count of failed cards for the status line.
    pub fn cards_failed(&self) -> usize {
        self.card_census().failed
    }

    /// Return the artifact-specific display hint for the focused card.
    pub fn card_artifact_hint(&self, kind: Artifact) -> &'static str {
        let Some(draft) = self.cards.drafts.get(self.cards.selected) else {
            return "queued";
        };
        artifact_hint(draft.artifacts(), kind)
    }

    /// Return the app with the focused candidate removed.
    pub fn dropped_selected(mut self) -> Self {
        if self.review.candidates.is_empty() {
            return self;
        }
        self.review.notice = None;
        let index = self
            .review
            .focus
            .row()
            .min(self.review.candidates.len() - 1);
        self.review.candidates.remove(index);
        self.review.open = std::mem::take(&mut self.review.open).dropped_row(index);
        let row = index.min(self.review.candidates.len().saturating_sub(1));
        self.review.focus = ReviewFocus::Head(row);
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

fn boundary_at_or_before(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let mut boundary = 0;
    for (index, _) in text.grapheme_indices(true) {
        if index > cursor {
            return boundary;
        }
        boundary = index;
    }
    boundary
}

fn boundary_before(text: &str, cursor: usize) -> usize {
    let cursor = boundary_at_or_before(text, cursor);
    let mut boundary = 0;
    for (index, _) in text.grapheme_indices(true) {
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
    match text[cursor..].graphemes(true).next() {
        Some(grapheme) => cursor + grapheme.len(),
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
    for (index, grapheme) in text.grapheme_indices(true) {
        if index >= cursor {
            return (row, column);
        }
        if grapheme.ends_with('\n') {
            row += 1;
            column = 0;
        } else {
            column += UnicodeWidthStr::width(grapheme);
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
    let mut seen = 0usize;
    for (offset, grapheme) in text[start..end].grapheme_indices(true) {
        if seen >= column {
            return start + offset;
        }
        seen += UnicodeWidthStr::width(grapheme);
    }
    let missing = column.saturating_sub(UnicodeWidthStr::width(&text[start..end]));
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
    use super::{App, boundary_before, cursor_below, cursor_forward};
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
        let settings = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Mixed);
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
                SentenceTypeMix::Mixed,
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
    fn opening_batch_settings_focuses_level_and_keeps_open_sense_lists() {
        let next = App::new(LanguagePair::new("fr", "en"))
            .understood(vec![crate::session::WordCandidate::new(
                "canard", "a duck", true,
            )])
            .sense_list_toggled()
            .sentence_settings_opened();
        assert_eq!(
            (next.sentence_settings_editor(), next.sense_list_open(0)),
            (Some(BatchSettingsRow::Level), true),
            "opening sentence settings collapsed an open sense list instead of leaving it inline"
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
    fn card_navigation_parks_the_editor_without_collapsing_its_card() {
        let navigated = cards().sentence_editor_opened_for_note().card_focus_next();
        assert_eq!(
            (
                navigated.card_selected(),
                navigated.card_expanded(),
                navigated.card_expanded_at(0),
                navigated.sentence_editor(),
            ),
            (1, false, true, None),
            "walking off an open editor collapsed its card instead of parking it"
        );
    }

    #[test]
    fn blob_navigation_never_splits_thai_or_decomposed_latin_graphemes() {
        let mut vertical = String::from("a\nกิ");
        let below = cursor_below(&mut vertical, 1);
        let decomposed = "e\u{301}";
        let previous = boundary_before(decomposed, decomposed.len());
        let mut thai = String::from("กิ");
        let forward = cursor_forward(&mut thai, 0);
        assert_eq!(
            (below, vertical.len(), previous, forward, thai.len()),
            (vertical.len(), vertical.len(), 0, thai.len(), thai.len()),
            "blob navigation left a cursor inside a visible grapheme cluster"
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
