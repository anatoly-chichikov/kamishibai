//! Terminal lifecycle and event loop for the interactive TUI, plus the
//! startup flows that decide which screen a fresh run opens on.

use std::io::{Write, stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags, poll, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use super::batch::StartupCards;
use super::bridge::TuiSession;
use super::host::open_path;
use super::shell::Shell;
use crate::config::{Preferences, default_store};
use crate::runtime::locations::SystemContext;
use crate::session::{CardDraft, LanguagePair};
use crate::tui::{
    App, AppEvent, KeySource, ModalKind, MousePointer, Screen, Side, WelcomeFocus, WelcomeStage,
    draw, language_chip_at, link_at, mouse_pointer_at, picker_geometry, reset_mouse_pointer,
    scroll_body_width, scroll_viewport, sentence_label_event_at, to_app, welcome_control_at,
    write_mouse_pointer,
};

const POINTER_REFRESH: Duration = Duration::from_millis(50);
const CURSOR_COLOR_WHITE: &[u8] = b"\x1b]12;#ffffff\x07";
const CURSOR_COLOR_RESET: &[u8] = b"\x1b]112\x07";
const CURSOR_BLINK_ON: &[u8] = b"\x1b[?12h";

/// Open the interactive TUI on a fresh app derived from saved preferences.
pub(super) fn start() -> Result<()> {
    let store = default_store(&SystemContext)?;
    let preferences = store.read()?;
    let app = startup_app(&preferences);
    run_tui(app, None, None)
}

/// Open the interactive TUI on a prebuilt cards JSON batch.
pub(super) fn start_with_batch(path: PathBuf) -> Result<()> {
    let (app, drafts) = StartupCards::load(path.as_path())?.into_parts();
    run_tui(app, Some(drafts), None)
}

fn startup_app(preferences: &Preferences) -> App {
    let saved_key = preferences.api_key.clone().filter(|key| !key.is_empty());
    let pair = LanguagePair::new(
        String::from("en"),
        preferences.startup_language().to_string(),
    );
    let app = App::new(pair);
    let needs_language = preferences.requires_language_choice();
    let needs_key = saved_key.is_none();
    if needs_language || needs_key {
        let (source, key) = if let Some(saved) = saved_key.as_deref() {
            (KeySource::Restored, String::from(saved))
        } else {
            (KeySource::Empty, String::new())
        };
        let stage = if needs_language {
            WelcomeStage::PickLanguage
        } else {
            WelcomeStage::EnterKey
        };
        app.opening_welcome_at(stage, source, key, env_has_gemini_key())
    } else {
        app
    }
}

/// Return whether `GEMINI_API_KEY` is present and non-empty. The key is never
/// loaded into the Welcome buffer implicitly — this only decides whether the
/// key step offers the `load from env` action.
pub(super) fn env_has_gemini_key() -> bool {
    std::env::var("GEMINI_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// Run the TUI from one initial app state, optional startup generation batch,
/// and optional resumed on-disk session.
pub(super) fn run_tui(
    app: App,
    startup: Option<Vec<CardDraft>>,
    session: Option<TuiSession>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    execute!(out, EnterAlternateScreen)?;
    apply_text_cursor(&mut out);
    enable_hover_mouse_capture(&mut out);
    write_mouse_pointer(&mut out, MousePointer::Arrow);
    if enhanced {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
    }
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let outcome = loop_forever(&mut terminal, app, startup, session);
    if enhanced {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags).ok();
    }
    reset_mouse_pointer(terminal.backend_mut());
    disable_hover_mouse_capture(terminal.backend_mut());
    reset_text_cursor(terminal.backend_mut());
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    outcome
}

fn apply_text_cursor<W: Write>(out: &mut W) {
    execute!(out, SetCursorStyle::BlinkingBlock).ok();
    let _ = out.write_all(CURSOR_BLINK_ON);
    let _ = out.write_all(CURSOR_COLOR_WHITE);
    let _ = out.flush();
}

fn reset_text_cursor<W: Write>(out: &mut W) {
    let _ = out.write_all(CURSOR_COLOR_RESET);
    execute!(out, SetCursorStyle::DefaultUserShape).ok();
    let _ = out.flush();
}

fn enable_hover_mouse_capture<W: Write>(out: &mut W) {
    let _ = out.write_all(b"\x1b[?1006h\x1b[?1003h");
    let _ = out.flush();
}

fn disable_hover_mouse_capture<W: Write>(out: &mut W) {
    let _ = out.write_all(b"\x1b[?1003l\x1b[?1006l");
    let _ = out.flush();
}

fn loop_forever<B>(
    terminal: &mut Terminal<B>,
    app: App,
    startup: Option<Vec<CardDraft>>,
    session: Option<TuiSession>,
) -> Result<()>
where
    B: ratatui::backend::Backend + Write,
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    let mut shell = match startup {
        Some(drafts) => Shell::startup(app, drafts, session)?,
        None => Shell::new(app, session)?,
    };
    let mut mouse_position: Option<(u16, u16)> = None;
    let mut dirty = true;
    loop {
        shell.persist();
        dirty |= shell.refresh_quit_pending();
        dirty |= shell.refresh_new_batch_pending();
        dirty |= shell.refresh_destructive_escape_pending();
        let rect = terminal_rect(terminal)?;
        let (viewport, body_width) = scroll_frame(shell.app(), rect);
        dirty |= shell.reclamp_scroll(viewport, body_width);
        if dirty {
            terminal.draw(|frame| draw(frame, shell.app()))?;
            write_pointer_at(terminal, shell.app(), rect, mouse_position);
            dirty = false;
        }
        let timeout = match mouse_position {
            Some(_) => shell.poll_timeout().min(POINTER_REFRESH),
            None => shell.poll_timeout(),
        };
        if !poll(timeout)? {
            dirty |= shell.tick()?;
            write_pointer_at(terminal, shell.app(), rect, mouse_position);
            continue;
        }
        let event = read()?;
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Release && shell.app().modal().is_none() {
                    let page = i32::from(viewport.saturating_sub(1).max(1));
                    let delta = match key.code {
                        KeyCode::PageUp => Some(-page),
                        KeyCode::PageDown => Some(page),
                        KeyCode::Char('n' | 'N')
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && scroll_hotkey_screen(shell.app().screen()) =>
                        {
                            Some(1)
                        }
                        KeyCode::Char('p' | 'P')
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && scroll_hotkey_screen(shell.app().screen()) =>
                        {
                            Some(-1)
                        }
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        shell.disarm_quit();
                        dirty |= shell.disarm_new_batch();
                        dirty |= shell.disarm_destructive_escape();
                        dirty |= shell.scroll(delta, viewport, body_width);
                        dirty |= shell.tick()?;
                        continue;
                    }
                }
                let Some(event) = to_app(key) else {
                    if key.kind != KeyEventKind::Release {
                        dirty |= shell.disarm_new_batch();
                        dirty |= shell.disarm_destructive_escape();
                    }
                    continue;
                };
                if matches!(event, AppEvent::Quit) {
                    shell.disarm_new_batch();
                    shell.disarm_destructive_escape();
                    if shell.arm_quit() {
                        return Ok(());
                    }
                    dirty = true;
                    continue;
                }
                shell.disarm_quit();
                if matches!(event, AppEvent::Cancel) && shell.handle_new_batch_escape()? {
                    shell.disarm_destructive_escape();
                    dirty = true;
                    continue;
                }
                shell.disarm_new_batch();
                let follow_focus = shell.app().modal().is_none()
                    && matches!(
                        event,
                        AppEvent::KeyEnter
                            | AppEvent::KeyChar(_)
                            | AppEvent::KeyBackspace
                            | AppEvent::NavPrev
                            | AppEvent::NavNext
                            | AppEvent::CursorLeft
                            | AppEvent::CursorRight
                    );
                let side = shell.handle(event)?;
                if side == Side::ExitApp {
                    return Ok(());
                }
                dirty = true;
                if follow_focus {
                    dirty |= shell.snap_scroll_to_selection(viewport, body_width);
                }
                dirty |= shell.tick()?;
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Moved => {
                    mouse_position = Some((mouse.column, mouse.row));
                    write_pointer_at(terminal, shell.app(), rect, mouse_position);
                }
                MouseEventKind::Drag(_) => {
                    dirty |= shell.disarm_new_batch();
                    dirty |= shell.disarm_destructive_escape();
                    mouse_position = Some((mouse.column, mouse.row));
                    write_pointer_at(terminal, shell.app(), rect, mouse_position);
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    dirty |= shell.disarm_new_batch();
                    dirty |= shell.disarm_destructive_escape();
                    mouse_position = Some((mouse.column, mouse.row));
                    write_pointer_at(terminal, shell.app(), rect, mouse_position);
                    let (viewport, body_width) = scroll_frame(shell.app(), rect);
                    let delta = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                        -1
                    } else {
                        1
                    };
                    dirty |= shell.scroll(delta, viewport, body_width);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    dirty |= shell.disarm_new_batch();
                    dirty |= shell.disarm_destructive_escape();
                    mouse_position = Some((mouse.column, mouse.row));
                    write_pointer_at(terminal, shell.app(), rect, mouse_position);
                    if shell.app().modal() == Some(ModalKind::PickMyLanguage) {
                        if let Some(index) = picker_geometry::chip_at(rect, mouse.column, mouse.row)
                        {
                            let codes = crate::languages::catalog().codes();
                            if let Some(code) = codes.get(index) {
                                let event = AppEvent::SetMyLanguage(String::from(*code));
                                let side = shell.handle(event)?;
                                if side == Side::ExitApp {
                                    return Ok(());
                                }
                                dirty = true;
                                dirty |= shell.tick()?;
                            }
                        }
                    } else if language_chip_at(shell.app(), rect, mouse.column, mouse.row) {
                        let side = shell.handle(AppEvent::OpenLanguagePicker)?;
                        if side == Side::ExitApp {
                            return Ok(());
                        }
                        dirty = true;
                        dirty |= shell.tick()?;
                    } else if let Some(focus) =
                        welcome_control_at(shell.app(), rect, mouse.column, mouse.row)
                    {
                        shell.handle(AppEvent::WelcomeFocusTo(focus))?;
                        let action = match focus {
                            WelcomeFocus::Submit => AppEvent::Submit,
                            WelcomeFocus::LoadEnv => AppEvent::WelcomeLoadEnvKey,
                        };
                        let side = shell.handle(action)?;
                        if side == Side::ExitApp {
                            return Ok(());
                        }
                        dirty = true;
                        dirty |= shell.tick()?;
                    } else if let Some(event) =
                        sentence_label_event_at(shell.app(), rect, mouse.column, mouse.row)
                    {
                        let side = shell.handle(event)?;
                        if side == Side::ExitApp {
                            return Ok(());
                        }
                        dirty = true;
                        let (viewport, body_width) = scroll_frame(shell.app(), rect);
                        dirty |= shell.snap_scroll_to_selection(viewport, body_width);
                        dirty |= shell.tick()?;
                    } else if let Some(target) = link_at(shell.app(), rect, mouse.column, mouse.row)
                    {
                        let _ = open_path(target.as_str());
                    }
                }
                _ => {}
            },
            Event::Resize(_, _) => {
                dirty = true;
            }
            _ => {
                dirty |= shell.tick()?;
            }
        }
    }
}

fn terminal_rect<B>(terminal: &mut Terminal<B>) -> Result<Rect>
where
    B: ratatui::backend::Backend,
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    let area = terminal.size()?;
    Ok(Rect {
        x: 0,
        y: 0,
        width: area.width,
        height: area.height,
    })
}

fn scroll_frame(app: &App, rect: Rect) -> (u16, u16) {
    (scroll_viewport(app, rect), scroll_body_width(rect))
}

fn scroll_hotkey_screen(screen: Screen) -> bool {
    matches!(screen, Screen::YourCards | Screen::Done)
}

fn write_pointer_at<B>(
    terminal: &mut Terminal<B>,
    app: &App,
    rect: Rect,
    position: Option<(u16, u16)>,
) where
    B: ratatui::backend::Backend + Write,
{
    if let Some((column, row)) = position {
        let next = mouse_pointer_at(app, rect, column, row);
        write_mouse_pointer(terminal.backend_mut(), next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::Screen;

    #[test]
    fn env_key_is_not_loaded_at_startup() {
        let app = startup_app(&Preferences::default());
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().known().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::PickLanguage,
                KeySource::Empty,
                String::from("en"),
            ),
            "GEMINI_API_KEY must not be treated as loaded until the user asks for it"
        );
    }

    #[test]
    fn saved_key_does_not_skip_unconfirmed_language_choice() {
        let preferences = Preferences::default().with_api_key("123456789012345678901234567890");
        let app = startup_app(&preferences);
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().known().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::PickLanguage,
                KeySource::Restored,
                String::from("en"),
            ),
            "a saved API key without a confirmed language must still start on language choice"
        );
    }

    #[test]
    fn confirmed_language_and_saved_key_skip_welcome() {
        let app =
            startup_app(&Preferences::new("de").with_api_key("123456789012345678901234567890"));
        assert_eq!(
            (app.screen(), app.pair().known().to_string()),
            (Screen::YourWords, String::from("de")),
            "a confirmed language may skip Welcome only when a saved key exists"
        );
    }

    #[test]
    fn confirmed_language_without_key_starts_on_key_stage() {
        let app = startup_app(&Preferences::new("ru"));
        assert_eq!(
            (
                app.screen(),
                app.welcome().stage,
                app.welcome().source,
                app.pair().known().to_string(),
            ),
            (
                Screen::Welcome,
                WelcomeStage::EnterKey,
                KeySource::Empty,
                String::from("ru"),
            ),
            "a confirmed language with no key must ask only for the missing key"
        );
    }
}
