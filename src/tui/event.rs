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
    /// User toggled "my" language override.
    ToggleMyLanguage,
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
}

impl AppEvent {
    /// Return the modal kind this event is intended for, if any.
    pub fn targets(&self) -> Option<ModalKind> {
        match self {
            AppEvent::SendCorrection(_) => Some(ModalKind::ChangeSomething),
            _ => None,
        }
    }
}
