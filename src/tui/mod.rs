//! TUI state machine skeleton.
//!
//! This module hosts the pure state / event / transition types that every
//! downstream screen task will bolt onto. It intentionally does not depend on
//! `ratatui` or `crossterm` — those are introduced by the test harness task.
//!
//! The full state map lives in `docs/tui-states/state-map.md`.

mod app;
mod event;
mod screen;
mod transition;

pub use app::App;
pub use event::{AppEvent, EditingOwner};
pub use screen::{ModalKind, Screen};
pub use transition::{Side, transit};
