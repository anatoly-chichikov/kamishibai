//! Per-screen renderers. Each screen owns its own layout and anchors on a
//! reference state in the design package
//! (`kamishibai-simple/project/`).

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::tui::app::App;

pub mod banner;
pub mod busy;
pub mod common;
pub mod done;
pub mod error;
pub mod language_grid;
pub mod modals;
pub mod sentence_labels;
pub mod welcome;
pub mod what_i_understood;
pub mod your_cards;
pub mod your_words;

/// Contract every fullscreen TUI screen must satisfy.
///
/// `tui::render` builds a `&dyn ScreenView` from the active `Screen` enum and
/// hands it to `common::render_screen`, which is the one and only entry point
/// for drawing the chrome (header, AI disclaimer, divider, footer) and
/// delegating to the body. `body` only receives the inner body rectangle, so a
/// screen cannot paint over the header or status regions even if it tries.
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
    /// Left segment of the status bar — where this screen stands, what it
    /// costs, how long it has run. Always drawn, whatever is layered on top.
    fn status(&self, app: &App) -> Vec<Span<'static>>;
    /// Right cluster of key hints for the screen's current sub-state, in
    /// reading order with the primary first.
    ///
    /// A screen answers for its own keyboard only. When an overlay owns the
    /// keyboard instead, `common::render_screen` replaces this answer rather
    /// than asking the screen to remember the overlay exists.
    fn hints(&self, app: &App) -> Vec<common::FooterHint>;
    /// Draw the body content into the inner body rectangle. The dispatcher
    /// already painted the background and header, and will paint the AI
    /// disclaimer, divider, and footer afterwards — body code must stay inside
    /// `area`.
    fn body(&self, frame: &mut Frame, area: Rect, app: &App);
    /// Row inside the body, counted from its first line, that a block closes
    /// itself off on — drawn as a dashed rule across the full terminal width,
    /// in the same colour and column phase as the rule that closes the body
    /// off from the footer. `None` on a screen with no such block.
    ///
    /// The rule is chrome, so the dispatcher paints it: `body` is handed a
    /// rectangle a gutter short of both screen edges, and a border stopping a
    /// gutter short reads as a different border, not as the same one.
    fn body_rule(&self, _: &App) -> Option<u16> {
        None
    }
}
