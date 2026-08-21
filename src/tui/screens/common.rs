//! Shared chrome for the Kamishibai TUI screens.
//!
//! These primitives keep every fullscreen screen anchored on the same design
//! grid as the HTML mockup (`kamishibai-simple/project/styles.css`). All
//! tokens come from the static palette — no accent hue is allowed.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::ScreenView;
use crate::tui::app::App;
use crate::tui::palette;

/// Horizontal breathing room applied to every screen's content column.
pub const GUTTER: u16 = 4;
/// Breathing room above the title row so it doesn't touch the top edge.
pub const TOP_MARGIN: u16 = 1;
/// Breathing room between the header and the body content.
pub const HEADER_GAP: u16 = 1;
/// Body-local column shared by artifact labels and sentence-label controls.
pub(crate) const CARD_DETAIL_COLUMN: usize = 6;
const AI_DISCLAIMER: &str = "ai may be wrong, please verify results";

/// Describes the rows of a fullscreen screen: header, body, AI disclaimer,
/// divider, and status bar.
pub struct ScreenFrame {
    pub header: Rect,
    pub body: Rect,
    pub disclaimer: Rect,
    pub status_rule: Rect,
    pub status: Rect,
}

/// Split the available area into the common screen frame.
///
/// Order top-down: top margin, header, breathing row, body, right-aligned AI
/// disclaimer, divider, status bar pinned to the last row.
pub fn frame_rects(area: Rect) -> ScreenFrame {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TOP_MARGIN),
            Constraint::Length(1),
            Constraint::Length(HEADER_GAP),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    ScreenFrame {
        header: inset_horizontal(rows[1], GUTTER),
        body: inset_horizontal(rows[3], GUTTER),
        disclaimer: disclaimer_rect(rows[4]),
        status_rule: rows[5],
        status: inset_horizontal(rows[6], GUTTER),
    }
}

/// Render the status divider below the persistent AI disclaimer.
pub fn paint_rules(frame: &mut Frame, rects: &ScreenFrame) {
    frame.render_widget(dashed_rule(rects.status_rule.width), rects.status_rule);
}

fn ai_disclaimer() -> Paragraph<'static> {
    Paragraph::new(AI_DISCLAIMER)
        .alignment(Alignment::Right)
        .style(palette::dim2())
}

fn disclaimer_rect(area: Rect) -> Rect {
    let width = u16::try_from(AI_DISCLAIMER.chars().count())
        .expect("invariant: AI disclaimer width must fit in u16");
    let gutter = if area.width >= width + GUTTER * 2 {
        GUTTER
    } else {
        0
    };
    inset_horizontal(area, gutter)
}

fn dashed_rule(width: u16) -> Paragraph<'static> {
    Paragraph::new(Line::from(dashed_spans(0, usize::from(width)))).style(palette::base())
}

/// Return one dashed rule as a line, indented by `start` plain columns. Screens
/// reuse it to separate blocks inside the body the way the status rule
/// separates the footer.
pub fn dashed_line(start: usize, width: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(" ".repeat(start), palette::base())];
    spans.extend(dashed_spans(start, width));
    Line::from(spans)
}

fn dashed_spans(start: usize, width: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(width);
    let dash_style = palette::rule().add_modifier(Modifier::CROSSED_OUT);
    for column in start..start + width {
        if column % 2 == 0 {
            spans.push(Span::styled(String::from(" "), dash_style));
        } else {
            spans.push(Span::styled(String::from(" "), palette::base()));
        }
    }
    spans
}

/// Return the rectangle shrunk by `gutter` columns on each side.
pub fn inset_horizontal(area: Rect, gutter: u16) -> Rect {
    let clamp = gutter.min(area.width / 2);
    Rect {
        x: area.x + clamp,
        y: area.y,
        width: area.width.saturating_sub(clamp * 2),
        height: area.height,
    }
}

/// Return a centered overlay rectangle that cannot cover the persistent AI
/// disclaimer or the chrome rows below it.
pub fn overlay_rect(area: Rect, width: u16, height: u16) -> Rect {
    let disclaimer_y = frame_rects(area).disclaimer.y;
    let actual_width = width.min(area.width);
    let actual_height = height.min(disclaimer_y.saturating_sub(area.y));
    let centered_y = area.y + area.height.saturating_sub(actual_height) / 2;
    let latest_y = disclaimer_y.saturating_sub(actual_height);
    Rect {
        x: area.x + area.width.saturating_sub(actual_width) / 2,
        y: centered_y.min(latest_y),
        width: actual_width,
        height: actual_height,
    }
}

/// Render the header row: inverted-block title, dim contextual tagline pinned
/// immediately to the right of the title, and the language chip pinned to the
/// right edge of the row.
///
/// The chip is provided as styled spans so the renderer can mark only the
/// learning language with the bright inverted block — the user's own
/// language stays dim, mirroring the "active label" treatment of the title.
///
/// Layout: `[ TITLE ]  hint                                 support → target`.
/// Two plain spaces separate the inverted title block from the dim hint —
/// enough breathing room after the bright background, no extra punctuation.
/// The flexible gap sits between the hint and the chip so the chip stays
/// anchored to the right edge.
pub fn header(
    title: &str,
    hint: &str,
    lang_chip: Option<Vec<Span<'static>>>,
    width: u16,
) -> Paragraph<'static> {
    let title = String::from(title);
    let mut hint = String::from(hint);
    let title_block = format!(" {title} ");
    let title_visible = display_width(title_block.as_str());
    let mut hint_lead = if hint.is_empty() { 0 } else { 2 };
    let mut chip = lang_chip.unwrap_or_default();
    let mut chip_visible = spans_width(&chip);
    let mut chip_lead = if chip.is_empty() { 0 } else { 2 };
    let mut used = header_width(title_visible, hint_lead, &hint, chip_visible, chip_lead);
    let width = usize::from(width);
    if used > width && !chip.is_empty() {
        chip = compact_chip(chip);
        chip_visible = spans_width(&chip);
        hint_lead = if hint.is_empty() { 0 } else { 1 };
        chip_lead = 1;
        used = header_width(title_visible, hint_lead, &hint, chip_visible, chip_lead);
    }
    if used > width && !hint.is_empty() {
        let reserved = title_visible + hint_lead + chip_visible + chip_lead;
        hint = take_chars(&hint, width.saturating_sub(reserved));
        hint_lead = if hint.is_empty() { 0 } else { hint_lead };
        used = header_width(title_visible, hint_lead, &hint, chip_visible, chip_lead);
    }
    let gap = width.saturating_sub(used);
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        title_block,
        palette::invert().add_modifier(Modifier::BOLD),
    ));
    if !hint.is_empty() {
        spans.push(Span::styled(" ".repeat(hint_lead), palette::base()));
        spans.push(Span::styled(hint, palette::dim()));
    }
    spans.push(Span::styled(" ".repeat(gap), palette::base()));
    if !chip.is_empty() {
        spans.push(Span::styled(" ".repeat(chip_lead), palette::base()));
        spans.extend(chip);
    }
    Paragraph::new(Line::from(spans)).style(palette::base())
}

fn header_width(
    title_visible: usize,
    hint_lead: usize,
    hint: &str,
    chip_visible: usize,
    chip_lead: usize,
) -> usize {
    title_visible + hint_lead + display_width(hint) + chip_visible + chip_lead
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum()
}

fn compact_chip(chip: Vec<Span<'static>>) -> Vec<Span<'static>> {
    chip.into_iter()
        .map(|span| Span::styled(span.content.replace(" → ", "→"), span.style))
        .collect()
}

fn take_chars(text: &str, limit: usize) -> String {
    split_display_prefix(text, limit).0.to_string()
}

/// Build the language chip — bold bright `support → target`.
///
/// Reading order is `support → target` so the user reads "from your language
/// into the language i'm learning". The whole chip — both languages and the
/// arrow between them — is rendered in bold bright `palette::base()`,
/// matching the title block on the opposite side of the header.
pub fn language_chip(app: &App) -> Vec<Span<'static>> {
    let known = app.pair().known().to_uppercase();
    let learning_text = if app.learning_pending() {
        String::from("…")
    } else {
        app.pair().learning().to_uppercase()
    };
    let style = palette::base().add_modifier(Modifier::BOLD);
    vec![
        Span::styled(known, style),
        Span::styled(" → ", style),
        Span::styled(learning_text, style),
    ]
}

/// Color weight of a footer key hint. This only paints the hint; how soon a
/// hint is shed on a narrow bar is decided by `FooterHint::keep`, not by tier.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The one action that advances the screen — brightest.
    Primary,
    /// A useful but non-advancing action — bold key, dim label.
    Secondary,
    /// Conventional or omnipresent keys (navigation, quit) — fully dim.
    Ghost,
}

/// Blank gap between status-bar segments and footer hints — two spaces, no
/// glyph. The bar carries no punctuation between items.
const SEPARATOR: &str = "  ";

/// One `[KEY] label` footer hint: its text, its color `tier`, and `keep` — how
/// long it survives when the bar is too narrow (higher is dropped later).
///
/// Built through `primary` / `secondary` / `ghost`; the status-bar renderer in
/// this module reads the fields directly to paint the hint and to pick the
/// drop order on a cramped line.
pub struct FooterHint {
    key: String,
    label: String,
    tier: Tier,
    keep: u8,
}

impl FooterHint {
    /// The screen's main action — bright label; kept the longest and shed only
    /// when even it cannot fit beside the status.
    pub fn primary(key: &str, label: &str) -> Self {
        Self::with(key, label, Tier::Primary, 3)
    }

    /// A secondary action — bold key, dim label, dropped before the quit hint.
    pub fn secondary(key: &str, label: &str) -> Self {
        Self::with(key, label, Tier::Secondary, 1)
    }

    /// A conventional or omnipresent key — fully dim, the first to be dropped.
    pub fn ghost(key: &str, label: &str) -> Self {
        Self::with(key, label, Tier::Ghost, 0)
    }

    fn with(key: &str, label: &str, tier: Tier, keep: u8) -> Self {
        Self {
            key: String::from(key),
            label: String::from(label),
            tier,
            keep,
        }
    }

    fn width(&self) -> usize {
        display_width(self.key.as_str()) + display_width(self.label.as_str()) + 3
    }

    /// Paint the hint as `[KEY] label` spans, colored by its tier. Used by the
    /// status bar and by modal action rows so every hint stays in lock-step.
    pub fn spans(&self) -> Vec<Span<'static>> {
        let (key_style, label_style) = match self.tier {
            Tier::Primary => (
                palette::base().add_modifier(Modifier::BOLD),
                palette::base(),
            ),
            Tier::Secondary => (palette::base().add_modifier(Modifier::BOLD), palette::dim()),
            Tier::Ghost => (palette::dim2(), palette::dim2()),
        };
        vec![
            Span::styled(format!("[{}]", self.key), key_style),
            Span::styled(format!(" {}", self.label), label_style),
        ]
    }
}

/// Render the bottom status bar: a left status segment and a right cluster of
/// key hints, separated by a flexible gap.
///
/// Hints arrive in reading order (primary first). When the line would overflow
/// `width`, whole hints are shed — never clipped — lowest `keep` first
/// (navigation, then secondaries right-to-left, then the quit hint), leaving
/// the bright primary action standing the longest. The assembled line is
/// finally clamped so it can never exceed `width`.
pub fn footer_bar(
    left: Vec<Span<'static>>,
    hints: Vec<FooterHint>,
    width: u16,
) -> Paragraph<'static> {
    Paragraph::new(Line::from(footer_spans(left, hints, width))).style(palette::base())
}

fn footer_spans(
    left: Vec<Span<'static>>,
    mut hints: Vec<FooterHint>,
    width: u16,
) -> Vec<Span<'static>> {
    let left_width: usize = left
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum();
    while !hints.is_empty() && !footer_fits(left_width, &hints, width) {
        let victim = droppable_hint(&hints);
        hints.remove(victim);
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    for hint in &hints {
        if !right.is_empty() {
            right.push(status_sep());
        }
        right.extend(hint.spans());
    }
    let right_width: usize = right
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum();
    let gap = (width as usize).saturating_sub(left_width + right_width);
    let mut spans = left;
    spans.push(Span::styled(" ".repeat(gap), palette::base()));
    spans.extend(right);
    clamp_spans(spans, width)
}

fn footer_fits(left_width: usize, hints: &[FooterHint], width: u16) -> bool {
    if hints.is_empty() {
        return left_width <= width as usize;
    }
    left_width + display_width(SEPARATOR) + hints_width(hints) <= usize::from(width)
}

fn hints_width(hints: &[FooterHint]) -> usize {
    let keys: usize = hints.iter().map(FooterHint::width).sum();
    keys + hints.len().saturating_sub(1) * display_width(SEPARATOR)
}

fn droppable_hint(hints: &[FooterHint]) -> usize {
    let mut victim = 0;
    for index in 1..hints.len() {
        if hints[index].keep <= hints[victim].keep {
            victim = index;
        }
    }
    victim
}

/// Trim a span run so its visible width never exceeds `width`, cutting inside
/// the overflowing span only as a last resort. Hints are dropped whole before
/// reaching here, so this bites only when the left status alone is wider than
/// an extremely narrow terminal.
fn clamp_spans(spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let mut budget = width as usize;
    let mut clamped: Vec<Span<'static>> = Vec::new();
    for span in spans {
        let length = display_width(span.content.as_ref());
        if length <= budget {
            budget -= length;
            clamped.push(span);
            continue;
        }
        let head = split_display_prefix(span.content.as_ref(), budget)
            .0
            .to_string();
        if !head.is_empty() {
            clamped.push(Span::styled(head, span.style));
        }
        break;
    }
    clamped
}

/// The blank gap between status-bar segments — two spaces, no glyph.
pub fn status_sep() -> Span<'static> {
    Span::styled(String::from(SEPARATOR), palette::base())
}

/// The global Ctrl+C quit hint — normally the rightmost item in a footer.
///
/// A dim `Ghost`; once the first Ctrl+C has been seen it brightens and its
/// label switches from `quit` to `again` to signal the next press will exit.
/// It outlives the secondary actions on a narrow line but is dropped — never
/// clipped — before the bar would overflow.
pub fn quit_hint(pending: bool) -> FooterHint {
    if pending {
        FooterHint::with("Ctrl+C", "again", Tier::Secondary, 2)
    } else {
        FooterHint::with("Ctrl+C", "quit", Tier::Ghost, 2)
    }
}

/// The review-screen back action, retained alongside quit on a crowded footer.
pub fn back_hint() -> FooterHint {
    FooterHint::with("Esc", "back", Tier::Secondary, 2)
}

/// The review-screen generation-guidance action, retained on a crowded footer.
pub fn sentence_settings_hint() -> FooterHint {
    FooterHint::with("↑", "guidance", Tier::Secondary, 2)
}

/// The nonempty words action, painted quietly but retained ahead of language
/// selection when the footer is crowded.
pub fn clear_words_hint() -> FooterHint {
    FooterHint::with("Esc", "clear", Tier::Ghost, 1)
}

/// The finished-screen Escape action and its armed confirmation state.
pub fn new_batch_hint(pending: bool) -> FooterHint {
    if pending {
        escape_again_hint()
    } else {
        FooterHint::with("Esc", "new cards", Tier::Ghost, 2)
    }
}

/// The high-priority second-Escape confirmation shared by destructive actions.
pub fn escape_again_hint() -> FooterHint {
    FooterHint::with("Esc", "again", Tier::Primary, 4)
}

/// One and only entry point for drawing a fullscreen screen.
///
/// Paints the background, computes the standard chrome layout, draws the
/// header, hands the inner body rectangle to `view.body`, and finishes with
/// the fixed AI disclaimer, divider, and footer. Screens cannot
/// bypass this function — `render::draw` only dispatches through it, and the
/// dispatcher hands `view.body` the body Rect only, so a screen has no handle
/// on the chrome regions.
pub fn render_screen(frame: &mut Frame, area: Rect, app: &App, view: &dyn ScreenView) {
    paint_background(frame, area);
    let rects = frame_rects(area);
    let title = view.title(app);
    let hint = view.hint(app);
    frame.render_widget(
        header(
            title.as_ref(),
            hint.as_ref(),
            view.lang_chip(app),
            rects.header.width,
        ),
        rects.header,
    );
    view.body(frame, rects.body, app);
    frame.render_widget(ai_disclaimer(), rects.disclaimer);
    paint_rules(frame, &rects);
    frame.render_widget(view.footer(app, rects.status.width), rects.status);
}

/// Clear a rectangle with the terminal-dark background so no stray paper bleeds through.
pub fn paint_background(frame: &mut Frame, area: Rect) {
    let filler = " ".repeat(area.width as usize);
    for row in 0..area.height {
        let strip = Rect {
            x: area.x,
            y: area.y + row,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(filler.clone(), palette::base()))),
            strip,
        );
    }
}

/// Number of body-rect rows actually available for scrolling content on the
/// current screen, given the full terminal area. Equals the body rect height
/// minus the rows reserved for the sticky outputs banner (when it is shown).
/// Used by the wheel-scroll clamp so the user can never push content past the
/// bottom edge of the viewport.
pub fn scroll_viewport(app: &App, terminal_area: Rect) -> u16 {
    let body_height = frame_rects(terminal_area).body.height;
    let banner_rows = if banner_visible(app) {
        super::banner::height(app)
    } else {
        0
    };
    body_height.saturating_sub(banner_rows)
}

/// Body-rect width in chars for the current `terminal_area`. Used by callers
/// that need to feed the layout calc — `Your cards` wraps the meta sentence on
/// the head row, so scroll clamp and click hit-test must agree on the width.
pub fn scroll_body_width(terminal_area: Rect) -> u16 {
    frame_rects(terminal_area).body.width
}

fn banner_visible(app: &App) -> bool {
    if !super::banner::has_entries(app) {
        return false;
    }
    match app.screen() {
        crate::tui::screen::Screen::Done => true,
        crate::tui::screen::Screen::YourCards => app.batch_settled(),
        _ => false,
    }
}

/// Pad a string to the requested character width.
pub fn pad_right(value: &str, width: usize) -> String {
    let mut text = String::from(value);
    let gap = width.saturating_sub(display_width(value));
    text.push_str(" ".repeat(gap).as_str());
    text
}

/// Return the display width of `text` in terminal cells.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Return the display width of one character in terminal cells.
pub fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Wrap plain text by terminal cells without leaving leading spaces on wrapped rows.
pub fn wrap_words(text: &str, first_width: usize, continuation_width: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec![String::new()];
    }
    if first_width == 0 || continuation_width == 0 {
        return vec![String::from(text)];
    }
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut limit = first_width;
    for word in text.split_whitespace() {
        let word_width = display_width(word);
        let separator = usize::from(!current.is_empty());
        if current_width + separator + word_width <= limit {
            if separator == 1 {
                current.push(' ');
            }
            current.push_str(word);
            current_width += separator + word_width;
            continue;
        }
        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
            limit = continuation_width;
        }
        let mut tail = word;
        while display_width(tail) > limit {
            let (head, rest) = split_display_prefix(tail, limit);
            rows.push(String::from(head));
            tail = rest;
            limit = continuation_width;
        }
        current.push_str(tail);
        current_width = display_width(tail);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

pub fn split_display_prefix(text: &str, width: usize) -> (&str, &str) {
    if width == 0 {
        return text.split_at(0);
    }
    let mut used = 0usize;
    let mut end = 0usize;
    for (index, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = display_width(grapheme);
        if used > 0 && used + grapheme_width > width {
            break;
        }
        end = index + grapheme.len();
        used += grapheme_width;
        if used >= width {
            break;
        }
    }
    if end == 0 {
        return text.split_at(text.chars().next().map(char::len_utf8).unwrap_or(0));
    }
    text.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn status() -> Vec<Span<'static>> {
        vec![Span::styled(
            String::from("step 2/3  nothing to make"),
            palette::dim2(),
        )]
    }

    fn short_status() -> Vec<Span<'static>> {
        vec![Span::styled(String::from("step 2/3"), palette::dim2())]
    }

    fn crowded_hints() -> Vec<FooterHint> {
        vec![
            FooterHint::primary("Ctrl+G", "generate"),
            FooterHint::secondary("Enter", "toggle"),
            FooterHint::secondary("D", "drop"),
            FooterHint::ghost("↑↓", "nav"),
            quit_hint(false),
        ]
    }

    fn measure(spans: &[Span<'static>]) -> usize {
        spans.iter().map(|span| span.content.chars().count()).sum()
    }

    #[test]
    fn primary_action_survives_a_narrow_footer() {
        let line = joined(&footer_spans(status(), crowded_hints(), 72));
        assert!(
            line.contains("Ctrl+G"),
            "primary generate hint must never be dropped from a narrow footer, got: {line}"
        );
    }

    #[test]
    fn ghost_nav_drops_before_any_secondary_when_narrow() {
        let line = joined(&footer_spans(status(), crowded_hints(), 72));
        assert!(
            !line.contains("↑↓"),
            "ghost nav must be shed before a secondary when the footer is too narrow, got: {line}"
        );
    }

    #[test]
    fn quit_hint_outlives_secondaries_on_a_narrow_footer() {
        let line = joined(&footer_spans(status(), crowded_hints(), 72));
        assert!(
            line.contains("Ctrl+C"),
            "the quit hint must outlive the secondary actions when the footer is tight, got: {line}"
        );
    }

    #[test]
    fn quit_drops_whole_rather_than_clip_on_the_narrowest_bar() {
        let line = joined(&footer_spans(short_status(), crowded_hints(), 30));
        assert!(
            !line.contains("Ctrl+C"),
            "a hint that cannot fit must be dropped whole, never clipped, got: {line}"
        );
    }

    #[test]
    fn the_primary_is_the_last_hint_standing() {
        let line = joined(&footer_spans(short_status(), crowded_hints(), 30));
        assert!(
            line.contains("Ctrl+G"),
            "the bright primary action must be the last hint kept on a cramped bar, got: {line}"
        );
    }

    #[test]
    fn armed_new_batch_confirmation_outlives_every_regular_action() {
        let hints = vec![
            new_batch_hint(true),
            FooterHint::primary("Ctrl+G", "regenerate"),
            FooterHint::secondary("Enter", "tune"),
            FooterHint::ghost("↑↓", "nav"),
            quit_hint(false),
        ];
        let line = joined(&footer_spans(short_status(), hints, 30));
        assert!(
            line.contains("[Esc] again") && !line.contains("Ctrl+G"),
            "a cramped footer hid the armed Escape confirmation behind a regular action: {line}"
        );
    }

    #[test]
    fn idle_new_cards_hint_uses_the_same_color_as_quit() {
        let new_cards = new_batch_hint(false).spans();
        let quit = quit_hint(false).spans();
        assert_eq!(
            (new_cards[0].style, new_cards[1].style),
            (quit[0].style, quit[1].style),
            "idle new cards must have exactly the same muted treatment as quit"
        );
    }

    #[test]
    fn clear_words_hint_is_muted_but_outlives_plain_ghosts() {
        let clear = clear_words_hint();
        let plain = FooterHint::ghost("Ctrl+L", "languages");
        let clear_spans = clear.spans();
        let plain_spans = plain.spans();
        assert!(
            clear.tier == Tier::Ghost
                && clear.keep == 1
                && clear_spans[0].style == plain_spans[0].style
                && clear_spans[1].style == plain_spans[1].style,
            "words clear must stay visually quiet while surviving before ordinary ghost hints"
        );
    }

    #[test]
    fn even_the_primary_is_shed_when_the_status_alone_crowds_the_bar() {
        let line = joined(&footer_spans(status(), crowded_hints(), 30));
        assert!(
            !line.contains("Ctrl+G"),
            "the primary is dropped whole — never clipped — when a long status leaves no room, got: {line}"
        );
    }

    #[test]
    fn wide_footer_keeps_every_hint() {
        let line = joined(&footer_spans(status(), crowded_hints(), 200));
        assert!(
            line.contains("↑↓"),
            "a wide footer must not drop ghost hints it has room for, got: {line}"
        );
    }

    #[test]
    fn wrap_words_does_not_start_continuation_with_space() {
        let rows = wrap_words("alpha beta gamma delta", 12, 8);
        assert!(
            rows.iter().skip(1).all(|row| !row.starts_with(' ')),
            "wrapped rows must not keep leading whitespace, got: {rows:?}"
        );
    }

    #[test]
    fn complex_graphemes_keep_terminal_cell_width_and_wrap_integrity() {
        let thai = "กิ่กิ่";
        assert_eq!(
            (display_width("กิ่"), split_display_prefix(thai, 1)),
            (1, ("กิ่", "กิ่")),
            "Thai combining marks no longer stay attached to their base terminal cell"
        );
    }

    #[test]
    fn footer_never_exceeds_its_width() {
        let within = (8u16..=120).all(|width| {
            measure(&footer_spans(status(), crowded_hints(), width)) <= usize::from(width)
        });
        assert!(
            within,
            "the rendered status bar must never be wider than the terminal at any width"
        );
    }

    #[test]
    fn footer_carries_no_dot_separators() {
        let line = joined(&footer_spans(status(), crowded_hints(), 60));
        assert!(
            !line.contains('·'),
            "the status bar must separate items with blank gaps, not dots, got: {line}"
        );
    }
}
