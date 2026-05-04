//! Per-screen renderers. Each screen owns its own layout and anchors on a
//! reference state in the design package
//! (`kamishibai-simple/project/`).

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::tui::app::App;

pub mod banner;
pub mod busy;
pub mod common;
pub mod done;
pub mod error;
pub mod modals;
pub mod welcome;
pub mod what_i_understood;
pub mod your_cards;
pub mod your_words;

/// Contract every fullscreen TUI screen must satisfy.
///
/// `tui::render` builds a `&dyn ScreenView` from the active `Screen` enum and
/// hands it to `common::render_screen`, which is the one and only entry point
/// for drawing the chrome (header, dashed rule, footer) and delegating to the
/// body. `body` only receives the inner body rectangle, so a screen cannot
/// paint over the header or status regions even if it tries.
pub trait ScreenView {
    /// Title for the inverted block at the top-left of the header. Takes
    /// `app` so screens whose label switches with state (`building your cards`
    /// → `your cards`) can branch without juggling enum variants.
    fn title(&self, app: &App) -> Cow<'static, str>;
    /// Contextual sub-tagline drawn in dim ink immediately after the title,
    /// separated by a thin `·`. Return an empty value to drop both the
    /// separator and the tagline.
    fn hint(&self, app: &App) -> Cow<'static, str>;
    /// Right-edge language chip. Defaults to the standard `support → target`
    /// chip; screens that have not locked in a language pair yet (e.g. the
    /// Welcome screen) should override this to `None`.
    fn lang_chip(&self, app: &App) -> Option<Vec<Span<'static>>> {
        Some(common::language_chip(app))
    }
    /// Status bar pinned to the bottom row of the frame.
    fn footer(&self, app: &App, width: u16) -> Paragraph<'static>;
    /// Draw the body content into the inner body rectangle. The dispatcher
    /// already painted the background, header and rule, and will paint the
    /// footer afterwards — body code must stay inside `area`.
    fn body(&self, frame: &mut Frame, area: Rect, app: &App);
}
