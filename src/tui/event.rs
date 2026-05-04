use super::screen::ModalKind;

/// Identifies which text editor currently owns keystrokes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditingOwner {
    RawBlob,
    BulkComment,
    CardComment,
}

/// All user and session-engine events the transition function accepts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    /// User asked to move forward from the current screen (Enter on body).
    Submit,
    /// User pressed the physical Enter key.
    KeyEnter,
    /// User asked to back out of a modal or go back a screen.
    Cancel,
    /// User pressed R — requests bulk or per-card correction depending on context.
    RequestChange,
    /// User confirmed the comment of the currently open modal.
    SendCorrection(String),
    /// User asked to start a new batch after Done.
    NewBatch,
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
    /// User overrode the detected target language.
    OverrideTarget(String),
    /// Text editor appended characters.
    KeyChar(char),
    /// Text editor pressed backspace.
    KeyBackspace,
    /// Arrow up or k — previous row.
    NavPrev,
    /// Arrow down or j — next row.
    NavNext,
    /// Session engine emitted understanding pass result.
    UnderstandingReady,
    /// Session engine emitted bulk correction result.
    BulkCorrectionReady,
    /// Session engine emitted per-card correction result.
    CardCorrectionReady,
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
    /// Welcome: user pressed `?` to be taken to the key URL.
    WelcomeOpenKeyHelp,
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
