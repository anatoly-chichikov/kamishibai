/// The eight locked-in fullscreen TUI states plus two modal overlays.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
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
