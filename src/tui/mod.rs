//! TUI state machine skeleton.
//!
//! This module hosts the pure state / event / transition types that every
//! downstream screen task will bolt onto. It intentionally does not depend on
//! `ratatui` or `crossterm` — those are introduced by the test harness task.
//!
//! The full state map lives in `docs/tui-states/state-map.md`.

mod app;
mod event;
mod input;
pub(crate) mod palette;
mod render;
mod screen;
mod screens;
mod transition;

pub use app::{App, BusyKind, BusyView};
pub use event::{AppEvent, EditingOwner};
pub use input::to_app;
pub use render::draw;
pub use screen::{ModalKind, Screen};
pub use transition::{Side, transit};
