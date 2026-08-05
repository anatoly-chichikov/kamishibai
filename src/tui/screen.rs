/// The locked-in fullscreen TUI states. The `Welcome` screen is shown until
/// the user has explicitly confirmed setup; the rest is the steady-state flow.
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
    /// Picker for the user's `my` language. Opened by `Cmd+L` / `Ctrl+L` or by
    /// clicking the language chip in the header. Shows the supported codes
    /// from `LanguageCatalog`, navigated with `←/→` and confirmed with
    /// `Enter`. The currently active code is pre-selected.
    PickMyLanguage,
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

/// Which button of the `EnterKey` step currently holds focus. The key field
/// itself is always editable and never takes focus.
///
/// `←/→` move focus between the buttons; `LoadEnv` only joins the cycle when
/// `GEMINI_API_KEY` is present in the environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WelcomeFocus {
    Submit,
    LoadEnv,
}
