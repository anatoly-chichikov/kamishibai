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
    /// User asked to open the `my` language picker modal (`Cmd+L`, `Ctrl+L`,
    /// or click on the header language chip).
    OpenLanguagePicker,
    /// User picked a `my` language code from the picker modal.
    SetMyLanguage(String),
    /// User cycled the picker selection one step left.
    LanguagePickerPrev,
    /// User cycled the picker selection one step right.
    LanguagePickerNext,
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
            AppEvent::SetMyLanguage(_)
            | AppEvent::LanguagePickerPrev
            | AppEvent::LanguagePickerNext => Some(ModalKind::PickMyLanguage),
            _ => None,
        }
    }
}
