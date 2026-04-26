//! Per-screen renderers. Each screen owns its own layout and anchors on a
//! specific PDF reference under `docs/tui-states/current-pdf/`.

pub mod busy;
pub mod common;
pub mod done;
pub mod error;
pub mod modals;
pub mod what_i_understood;
pub mod your_cards;
pub mod your_words;
