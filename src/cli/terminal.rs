//! Terminal lifecycle and event loop for the CLI.

use std::io::{Write, stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    Event, KeyboardEnhancementFlags, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags, poll, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use super::host::open_path;
use super::shell::Shell;
use crate::session::CardDraft;
use crate::tui::{
    App, AppEvent, ModalKind, MousePointer, Side, WelcomeFocus, draw, language_chip_at, link_at,
    mouse_pointer_at, picker_geometry, reset_mouse_pointer, scroll_body_width, scroll_viewport,
    to_app, welcome_control_at, write_mouse_pointer,
};

const POINTER_REFRESH: Duration = Duration::from_millis(50);

/// Run the TUI from one initial app state and optional startup generation batch.
pub(super) fn run_tui(app: App, startup: Option<Vec<CardDraft>>) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    execute!(out, EnterAlternateScreen)?;
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
    let outcome = loop_forever(&mut terminal, app, startup);
    if enhanced {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags).ok();
    }
    reset_mouse_pointer(terminal.backend_mut());
    disable_hover_mouse_capture(terminal.backend_mut());
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    outcome
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
) -> Result<()>
where
    B: ratatui::backend::Backend + Write,
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    let mut shell = match startup {
        Some(drafts) => Shell::startup(app, drafts)?,
        None => Shell::new(app)?,
    };
    let mut mouse_position: Option<(u16, u16)> = None;
    let mut dirty = true;
    loop {
        dirty |= shell.refresh_quit_pending();
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
                let Some(event) = to_app(key) else { continue };
                if matches!(event, AppEvent::Quit) {
                    if shell.arm_quit() {
                        return Ok(());
                    }
                    dirty = true;
                    continue;
                }
                shell.disarm_quit();
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
                MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                    mouse_position = Some((mouse.column, mouse.row));
                    write_pointer_at(terminal, shell.app(), rect, mouse_position);
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
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
