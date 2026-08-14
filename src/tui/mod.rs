//! TUI state machine.
//!
//! Hosts the pure state / event / transition types for the locked-in flow
//! plus the renderer entry point. Concrete `ratatui` and `crossterm` use
//! lives in the screen modules and the input mapper.
//!
//! The full state map lives in `docs/tui-states/state-map.md`.

mod app;
mod disclosure;
mod event;
mod input;
mod links;
pub(crate) mod palette;
mod picker;
mod pointer;
mod render;
mod screen;
mod screens;
mod sentence_editor;
mod text_field;
mod transition;

pub use app::{App, BusyKind, BusyView, WelcomeView};
pub use event::{AppEvent, EditingOwner};
pub use input::to_app;
pub use links::{
    language_chip_at, link_at, review_event_at, sentence_label_event_at, welcome_control_at,
};
pub use picker::{AUTO_CHIP, LanguageChoice, PickerCursor, PickerSection, learning_target};
pub use pointer::{
    MousePointer, mouse_pointer_at, reset_mouse_pointer, write_mouse_pointer,
    write_mouse_pointer_once,
};
pub use render::draw;
pub use screen::{KeySource, ModalKind, Screen, WelcomeFocus, WelcomeStage};
pub use screens::common::{scroll_body_width, scroll_viewport};
pub use screens::modals::picker_geometry;
pub use sentence_editor::{BatchSettingsRow, LabelEditorRow, NoteDraft, SentenceLabelsEditor};
pub use transition::{Side, transit};
