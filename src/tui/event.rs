use super::picker::{LanguageChoice, PickerSection};
use super::screen::{ModalKind, WelcomeFocus};
use super::sentence_editor::{BatchSettingsRow, LabelEditorRow};

/// Identifies which text editor currently owns keystrokes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditingOwner {
    RawBlob,
    BulkComment,
}

/// All user and session-engine events the transition function accepts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    /// User asked to move forward from the current screen.
    Submit,
    /// User pressed Ctrl+G to start or restart generation.
    Generate,
    /// User pressed the physical Enter key.
    KeyEnter,
    /// User asked to back out of a modal or go back a screen.
    Cancel,
    /// Mouse or keyboard opened generation guidance above the reviewed words.
    SentenceSettingsOpen,
    /// Mouse focus moved to one batch sentence-settings row.
    SentenceSettingsFocus(BatchSettingsRow),
    /// Mouse picked one batch sentence-settings choice.
    SentenceSettingsChoose(BatchSettingsRow, usize),
    /// Mouse moved one batch sentence-settings row by one adjacent choice.
    SentenceSettingsAdvance(BatchSettingsRow, bool),
    /// Mouse selected one card and opened its inline sentence-label editor.
    SentenceLabelOpen(usize, LabelEditorRow),
    /// Mouse focus moved to one row of the inline sentence-label editor.
    SentenceLabelFocus(LabelEditorRow),
    /// Mouse picked one chip on one row of the inline sentence-label editor.
    SentenceLabelChoose(LabelEditorRow, usize),
    /// Mouse moved one row of the inline sentence-label editor by one choice.
    SentenceLabelAdvance(LabelEditorRow, bool),
    /// User confirmed the comment of the currently open modal.
    SendCorrection(String),
    /// User asked to quit the app from Done.
    Quit,
    /// A keyboard shortcut asked to open the language pair modal on the half
    /// preferred by the active screen.
    OpenPreferredLanguagePicker,
    /// A click asked to open the language pair modal on one explicit half of
    /// the header language chip.
    OpenLanguagePicker(PickerSection),
    /// User confirmed one language pair, from the modal or from a click on the
    /// `also plausible` hint.
    SetLanguages(LanguageChoice),
    /// Mouse highlighted one row of one picker column without confirming.
    LanguagePickerPoint(PickerSection, usize),
    /// User moved the focused picker column one row up.
    LanguagePickerPrev,
    /// User moved the focused picker column one row down.
    LanguagePickerNext,
    /// User moved picker focus onto one column of the pair.
    LanguagePickerFocus(PickerSection),
    /// Text editor inserted a character.
    KeyChar(char),
    /// Text editor pressed backspace.
    KeyBackspace,
    /// Text editor moved its cursor left.
    CursorLeft,
    /// Text editor moved its cursor right.
    CursorRight,
    /// Arrow up or k — previous row.
    NavPrev,
    /// Arrow down or j — next row.
    NavNext,
    /// Session engine emitted understanding pass result.
    UnderstandingReady,
    /// Session engine emitted bulk correction result.
    BulkCorrectionReady,
    /// Session engine reported that every artifact for every card is ready.
    BatchReady,
    /// Session engine reported that the queue fully drained, possibly with failures.
    BatchDone { failed: usize },
    /// Session engine reported that a card artifact entered retry.
    RetryStarted,
    /// Session engine reported that a card ran out of retry attempts.
    RetryExhausted,
    /// Resize or redraw — no state change, included for completeness.
    Redraw,
    /// Welcome stage 0: arrow-cycle to the previous language chip.
    WelcomePrevLanguage,
    /// Welcome stage 0: arrow-cycle to the next language chip.
    WelcomeNextLanguage,
    /// Welcome stage 0: pick the language at one place in the grid. Carries a
    /// resolved catalog position because only the terminal layer knows how wide
    /// the rendered grid is — the same reason the picker's wheel resolves a row
    /// before it becomes an event.
    WelcomeLanguageAt(usize),
    /// Welcome: a clipboard paste landed on the API key input.
    WelcomePasteKey(String),
    /// Welcome: user asked to load `GEMINI_API_KEY` from the environment.
    WelcomeLoadEnvKey,
    /// Welcome key step: move focus to a specific control (mouse click).
    WelcomeFocusTo(WelcomeFocus),
}

impl AppEvent {
    /// Return the modal kind this event is intended for, if any.
    pub fn targets(&self) -> Option<ModalKind> {
        match self {
            AppEvent::SendCorrection(_) => Some(ModalKind::ChangeSomething),
            AppEvent::LanguagePickerPoint(_, _)
            | AppEvent::LanguagePickerPrev
            | AppEvent::LanguagePickerNext
            | AppEvent::LanguagePickerFocus(_) => Some(ModalKind::PickLanguages),
            _ => None,
        }
    }
}
