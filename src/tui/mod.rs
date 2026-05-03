//! TUI state machine.
//!
//! Hosts the pure state / event / transition types for the locked-in flow
//! plus the renderer entry point. Concrete `ratatui` and `crossterm` use
//! lives in the screen modules and the input mapper.
//!
//! The full state map lives in `docs/tui-states/state-map.md`.

mod app;
mod event;
mod input;
mod links;
pub(crate) mod palette;
mod render;
mod screen;
mod screens;
mod transition;

pub use app::{App, BusyKind, BusyView, WelcomeView};
pub use event::{AppEvent, EditingOwner};
pub use input::to_app;
pub use links::link_at;
pub use render::draw;
pub use screen::{KeySource, ModalKind, Screen, WelcomeStage};
pub use transition::{Side, transit};
