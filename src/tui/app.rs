use std::time::Duration;

use crate::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, LanguagePair, WordCandidate,
};

use super::screen::{KeySource, ModalKind, Screen, WelcomeStage};

/// The immutable shell state carried between transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    screen: Screen,
    modal: Option<ModalKind>,
    busy: Option<BusyView>,
    error: Option<String>,
    pair: LanguagePair,
    input: AppInput,
    review: Review,
    cards: CardsView,
    done: DoneArtifacts,
    welcome: WelcomeView,
    body_scroll: u16,
    quit_pending: bool,
    picker_cursor: usize,
}

/// First-run welcome state: stage, pasted key, source of that key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WelcomeView {
    pub stage: WelcomeStage,
    pub key: String,
    pub source: KeySource,
}

impl Default for WelcomeView {
    fn default() -> Self {
        Self {
            stage: WelcomeStage::PickLanguage,
            key: String::new(),
            source: KeySource::Empty,
        }
    }
}

/// The blocking text pass currently covering the interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusyKind {
    Understanding,
    BulkCorrection,
    CardCorrection,
    /// Phase 1 of `publish`: building the Anki .apkg container.
    PublishingDeck,
    /// Phase 2 of `publish`: rendering the PDF report.
    PublishingReport,
}

impl BusyKind {
    /// Return the short text shown in the universal loader.
    pub fn label(&self) -> &'static str {
        match self {
            BusyKind::Understanding => "understanding your words",
            BusyKind::BulkCorrection => "applying your changes",
            BusyKind::CardCorrection => "updating this card",
            BusyKind::PublishingDeck => "building your Anki deck",
            BusyKind::PublishingReport => "rendering your PDF report",
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
            busy: None,
            error: None,
            pair,
            input: AppInput {
                target_pending: true,
                ..AppInput::default()
            },
            review: Review::default(),
            cards: CardsView::default(),
            done: DoneArtifacts::default(),
            welcome: WelcomeView::default(),
            body_scroll: 0,
            quit_pending: false,
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

    /// Return the app rerouted onto the first-run Welcome screen starting
    /// at the language-pick stage.
    pub fn opening_welcome(self, source: KeySource, key: impl Into<String>) -> Self {
        self.opening_welcome_at(WelcomeStage::PickLanguage, source, key)
    }

    /// Return the app rerouted onto the first-run Welcome screen with an
    /// explicit starting stage. Used by `start()` to skip past whichever step
    /// is already satisfied by the loaded preferences and environment.
    pub fn opening_welcome_at(
        mut self,
        stage: WelcomeStage,
        source: KeySource,
        key: impl Into<String>,
    ) -> Self {
        self.screen = Screen::Welcome;
        self.welcome = WelcomeView {
            stage,
            key: key.into(),
            source,
        };
        self
    }

    /// Return the welcome view (read-only).
    pub fn welcome(&self) -> &WelcomeView {
        &self.welcome
    }

    /// Return the app advanced from picking language to entering a key.
    pub fn welcome_advance(mut self) -> Self {
        self.welcome.stage = WelcomeStage::EnterKey;
        self
    }

    /// Return the app stepped back from entering the key to picking the language.
    pub fn welcome_step_back(mut self) -> Self {
        self.welcome.stage = WelcomeStage::PickLanguage;
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
        self
    }

    /// Return the app with the API key cleared so the user can paste a new one.
    pub fn welcome_clear_key(mut self) -> Self {
        self.welcome.key = String::new();
        self.welcome.source = KeySource::Empty;
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

    /// Return the app with the body scroll snapped so the focused card is
    /// fully inside the `viewport`. Used after arrow-key navigation: if the
    /// user wheel-scrolled the selection out of view, the next ↑/↓ press
    /// pulls scroll back so the new selection lands at the top or bottom
    /// edge of the visible area. Inert on screens without a card cursor.
    /// `body_width` is the body rect width in chars; passed through so the
    /// snap math agrees with the renderer about the wrapped head-row height.
    pub fn body_scroll_to_selection(mut self, viewport: u16, body_width: u16) -> Self {
        if !matches!(self.screen, Screen::YourCards) {
            return self;
        }
        let Some((top, height)) =
            crate::tui::screens::your_cards::focused_card_range(&self, usize::from(body_width))
        else {
            return self;
        };
        let max = self
            .body_content_height(body_width)
            .saturating_sub(viewport);
        let bottom = top.saturating_add(height);
        let mut next = self.body_scroll;
        if top < next {
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

    fn body_content_height(&self, body_width: u16) -> u16 {
        let width = usize::from(body_width);
        match self.screen {
            Screen::YourCards => crate::tui::screens::your_cards::content_height(self, width),
            Screen::Done => crate::tui::screens::done::content_height(self),
            Screen::WhatIUnderstood => crate::tui::screens::what_i_understood::content_height(self),
            Screen::YourWords | Screen::Welcome => 0,
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

    /// Return the app with the `my` (support) language replaced by `code`.
    /// Target stays untouched. Use this from the language picker modal and
    /// from the Welcome screen — there is no implicit cycle anymore.
    pub fn set_support(mut self, code: impl Into<String>) -> Self {
        let pair = LanguagePair::new(self.pair.target().to_string(), code.into());
        self.pair = pair;
        self
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
            elapsed: Duration::ZERO,
            running: None,
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
        self.cards.drafts = drafts;
        self.cards.selected = selected;
        if self.cards.drafts.is_empty() {
            self.cards.expanded = false;
        }
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

    /// Return the app with every failed artifact slot reset to fresh so the
    /// session engine can re-enqueue it.
    pub fn cards_reset_failures(mut self) -> Self {
        for draft in self.cards.drafts.iter_mut() {
            if !draft.artifacts().has_failed() {
                continue;
            }
            let artifacts = draft.artifacts();
            let body = if artifacts.body().failed_terminally() {
                ArtifactSlot::fresh(Artifact::Body)
            } else {
                artifacts.body().clone()
            };
            let scene = if artifacts.scene().failed_terminally() {
                ArtifactSlot::fresh(Artifact::Scene)
            } else {
                artifacts.scene().clone()
            };
            let picture = if artifacts.picture().failed_terminally() {
                ArtifactSlot::fresh(Artifact::Picture)
            } else {
                artifacts.picture().clone()
            };
            let sound = if artifacts.sound().failed_terminally() {
                ArtifactSlot::fresh(Artifact::Sound)
            } else {
                artifacts.sound().clone()
            };
            *draft = draft
                .clone()
                .with_artifacts(CardArtifacts::from_parts(body, scene, picture, sound));
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
        Artifact::Body => artifacts.body(),
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
