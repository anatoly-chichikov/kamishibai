/// The locked-in fullscreen TUI states. The first-run `Welcome` screen is
/// shown only on a brand new install; the rest is the steady-state flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Welcome,
    YourWords,
    WhatIUnderstood,
    YourCards,
    Done,
}

/// Modal overlays that live on top of a fullscreen screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModalKind {
    ChangeSomething,
    ChangeThisCard,
}

/// Stage the first-run Welcome screen is currently on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WelcomeStage {
    PickLanguage,
    EnterKey,
}

/// Source of the Gemini API key currently held by the Welcome screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeySource {
    Empty,
    Env,
    Restored,
    Pasted,
}
