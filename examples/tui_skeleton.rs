//! Minimal skeleton binary that drives the locked-in state machine from
//! keystrokes.
//!
//! `tests/snapshot.rs` and `tests/keyboard.rs` cover the real ratatui render
//! path against a `TestBackend`. This binary exists so that `tests/pty.rs` can
//! exercise the same `App`/`transit` pipeline through a real pseudoterminal
//! without having to decode ANSI escape sequences. It prints plain-text
//! screen markers after every transition — rich rendering stays in the
//! library so the two test layers can evolve independently.

use std::io::{self, Read, Write, stdin, stdout};

use anyhow::Result;
use kamishibai::session::{LanguagePair, WordCandidate};
use kamishibai::tui::{App, AppEvent, Screen, Side, transit};

fn main() -> Result<()> {
    let mut app = App::new(LanguagePair::new("en", "ru"));
    render(&app)?;
    let handle = stdin();
    let mut input = handle.lock();
    let mut byte = [0u8; 1];
    loop {
        if input.read(&mut byte)? == 0 {
            break;
        }
        if byte[0] == b'q' || byte[0] == b'Q' {
            break;
        }
        let event = match byte[0] {
            0x07 => AppEvent::Generate,
            b'\r' | b'\n' => AppEvent::KeyEnter,
            0x1B => AppEvent::Cancel,
            other => AppEvent::KeyChar(other as char),
        };
        let (next, side) = transit(app.clone(), promote(event, &app));
        app = next;
        render(&app)?;
        if matches!(side, Side::RunUnderstanding) {
            app = app
                .with_screen(Screen::WhatIUnderstood)
                .confirmed_learning("en")
                .understood(vec![WordCandidate::new("a", "letter A", true)]);
            render(&app)?;
        }
        if matches!(side, Side::ExitApp) {
            break;
        }
    }
    Ok(())
}

fn render(app: &App) -> io::Result<()> {
    let title = match app.screen() {
        Screen::Welcome => "Welcome",
        Screen::YourWords => "Your words",
        Screen::WhatIUnderstood => "What I understood",
        Screen::YourCards => "Your cards",
        Screen::Done => "Done",
    };
    let mut out = stdout().lock();
    writeln!(
        out,
        "[screen] {title} :: pair={} :: modal={:?}",
        app.pair().label(),
        app.modal()
    )?;
    out.flush()
}

fn promote(event: AppEvent, app: &App) -> AppEvent {
    match (app.screen(), &event) {
        (Screen::YourCards, AppEvent::Generate) => AppEvent::BatchReady,
        _ => event,
    }
}
