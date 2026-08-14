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
    /// Picker for the whole session language pair. Opened by `Cmd+L` /
    /// `Ctrl+L` or by clicking a half of the header language chip. Shows the
    /// supported codes from `LanguageCatalog` twice — once for the known half
    /// and once, behind a leading `auto` chip, for the learning half. `↑/↓`
    /// moves between the halves, `←/→` moves inside one, and `Enter` confirms
    /// both at once. The active pair is pre-selected.
    PickLanguages,
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
