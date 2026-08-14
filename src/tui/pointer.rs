//! Mouse pointer shape policy for the TUI shell.
//!
//! The terminal renderer owns text cells, but the host terminal still owns the
//! operating-system mouse pointer. This module keeps the pointer shape aligned
//! with TUI hit-testing: normal arrow over inert cells and hand pointer over
//! click targets.

use std::env;
use std::io::Write;

use ratatui::layout::Rect;

use super::app::App;
use super::links::{
    language_chip_at, link_at, review_event_at, sentence_label_event_at, welcome_control_at,
};
use super::screen::ModalKind;
use super::screens::modals::picker_geometry;

/// Mouse pointer shapes the TUI asks the terminal to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MousePointer {
    Arrow,
    Hand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerDialect {
    Css,
    ItermX11,
}

/// Return the mouse pointer shape for one terminal cell.
#[must_use]
pub fn mouse_pointer_at(app: &App, terminal: Rect, column: u16, row: u16) -> MousePointer {
    if clickable_at(app, terminal, column, row) {
        return MousePointer::Hand;
    }
    MousePointer::Arrow
}

/// Write one OSC 22 pointer-shape request to the terminal.
pub fn write_mouse_pointer<W: Write>(out: &mut W, pointer: MousePointer) {
    write_mouse_pointer_for(out, pointer, pointer_dialect());
}

fn write_mouse_pointer_for<W: Write>(out: &mut W, pointer: MousePointer, dialect: PointerDialect) {
    let _ = out.write_all(format!("\x1b]22;{}\x1b\\", pointer_shape(pointer, dialect)).as_bytes());
    let _ = out.flush();
}

/// Write the pointer shape only when it differs from the remembered shape.
pub fn write_mouse_pointer_once<W: Write>(
    out: &mut W,
    current: &mut MousePointer,
    next: MousePointer,
) {
    write_mouse_pointer_once_for(out, current, next, pointer_dialect());
}

fn write_mouse_pointer_once_for<W: Write>(
    out: &mut W,
    current: &mut MousePointer,
    next: MousePointer,
    dialect: PointerDialect,
) {
    if *current == next {
        return;
    }
    write_mouse_pointer_for(out, next, dialect);
    *current = next;
}

/// Restore the terminal's default pointer policy.
pub fn reset_mouse_pointer<W: Write>(out: &mut W) {
    let _ = out.write_all(b"\x1b]22;\x1b\\");
    let _ = out.flush();
}

fn clickable_at(app: &App, terminal: Rect, column: u16, row: u16) -> bool {
    if app.modal() == Some(ModalKind::PickLanguages) {
        return picker_geometry::row_at(terminal, app.picker_cursor(), column, row).is_some();
    }
    language_chip_at(app, terminal, column, row).is_some()
        || welcome_control_at(app, terminal, column, row).is_some()
        || review_event_at(app, terminal, column, row).is_some()
        || sentence_label_event_at(app, terminal, column, row).is_some()
        || link_at(app, terminal, column, row).is_some()
}

fn pointer_dialect() -> PointerDialect {
    if env_matches("TERM_PROGRAM", "iTerm.app")
        || env_matches("LC_TERMINAL", "iTerm2")
        || env::var_os("ITERM_SESSION_ID").is_some()
    {
        return PointerDialect::ItermX11;
    }
    PointerDialect::Css
}

fn env_matches(name: &str, value: &str) -> bool {
    env::var(name).map(|found| found == value).unwrap_or(false)
}

fn pointer_shape(pointer: MousePointer, dialect: PointerDialect) -> &'static str {
    match (pointer, dialect) {
        (MousePointer::Arrow, PointerDialect::Css) => "default",
        (MousePointer::Hand, PointerDialect::Css) => "pointer",
        (MousePointer::Arrow, PointerDialect::ItermX11) => "left_ptr",
        (MousePointer::Hand, PointerDialect::ItermX11) => "hand2",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::session::{
        Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, LanguagePair,
    };
    use crate::tui::{Screen, draw};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    use super::*;

    fn pair() -> LanguagePair {
        LanguagePair::new("en", "ru")
    }

    fn meta(term: &str) -> CardMeta {
        CardMeta::new(
            format!("/{term}/"),
            format!("/{term}/"),
            format!("meaning of {term}"),
            5,
            format!("source sentence with {term}"),
            term,
            format!("hint for {term}"),
            format!("context for {term}"),
            format!("{term} target"),
        )
    }

    fn file(name: &str) -> ArtifactFile {
        ArtifactFile::new(name, PathBuf::from(format!("/tmp/{name}")), "1 B", false)
    }

    fn artifacts() -> CardArtifacts {
        CardArtifacts::from_parts(
            ArtifactSlot::fresh(Artifact::Meta).succeeded_with(file("meta.local.json")),
            ArtifactSlot::fresh(Artifact::Scene).succeeded_with(file("scene.local.json")),
            ArtifactSlot::fresh(Artifact::Picture).succeeded_with(file("picture.local.jpg")),
            ArtifactSlot::fresh(Artifact::Sound).succeeded_with(file("sound.local.wav")),
        )
    }

    fn card(term: &str) -> CardDraft {
        CardDraft::new(term, format!("understanding for {term}"), pair())
            .with_meta(meta(term), None)
            .with_artifacts(artifacts())
    }

    fn terminal() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 50,
        }
    }

    fn rendered_link_cells(app: &App) -> Vec<(u16, u16)> {
        let backend = TestBackend::new(120, 50);
        let mut terminal = Terminal::new(backend).expect("backend");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let buffer = terminal.backend().buffer();
        let mut cells = Vec::new();
        for row in 0..buffer.area.height {
            for column in 0..buffer.area.width {
                let cell = &buffer[(column, row)];
                if cell.modifier.contains(Modifier::UNDERLINED) && cell.symbol() != " " {
                    cells.push((column, row));
                }
            }
        }
        cells
    }

    #[test]
    fn artifact_file_labels_get_the_hand_pointer_and_plain_cells_get_the_arrow() {
        let app = App::new(pair())
            .with_screen(Screen::YourCards)
            .confirmed_learning("en")
            .cards_started(vec![card("whilst")]);
        assert_eq!(
            (
                mouse_pointer_at(&app, terminal(), 9, 5),
                mouse_pointer_at(&app, terminal(), 10, 5),
                mouse_pointer_at(&app, terminal(), 18, 5),
                mouse_pointer_at(&app, terminal(), 19, 5),
                mouse_pointer_at(&app, terminal(), 33, 5),
                mouse_pointer_at(&app, terminal(), 7, 5),
                mouse_pointer_at(&app, terminal(), 10, 3),
            ),
            (
                MousePointer::Arrow,
                MousePointer::Hand,
                MousePointer::Hand,
                MousePointer::Arrow,
                MousePointer::Arrow,
                MousePointer::Arrow,
                MousePointer::Hand,
            ),
            "file-backed rows and the legacy tuning head must use the hand while inert cells keep the arrow"
        );
    }

    #[test]
    fn welcome_key_step_controls_get_the_hand_pointer() {
        let app = App::new(pair()).opening_welcome_at(
            crate::tui::WelcomeStage::EnterKey,
            crate::tui::KeySource::Empty,
            "",
            true,
        );
        assert_eq!(
            (
                mouse_pointer_at(&app, terminal(), 28, 12),
                mouse_pointer_at(&app, terminal(), 42, 12),
                mouse_pointer_at(&app, terminal(), 10, 9),
                mouse_pointer_at(&app, terminal(), 8, 12),
                mouse_pointer_at(&app, terminal(), 8, 20),
            ),
            (
                MousePointer::Hand,
                MousePointer::Hand,
                MousePointer::Arrow,
                MousePointer::Arrow,
                MousePointer::Arrow,
            ),
            "Welcome key step must show the hand over both chips and the arrow over the field row and empty space"
        );
    }

    #[test]
    fn welcome_key_step_rules_the_field_and_marks_the_active_step() {
        let app = App::new(pair()).opening_welcome_at(
            crate::tui::WelcomeStage::EnterKey,
            crate::tui::KeySource::Empty,
            "",
            true,
        );
        let backend = TestBackend::new(120, 50);
        let mut terminal = Terminal::new(backend).expect("backend");
        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        let buffer = terminal.backend().buffer();
        let mut rule_in_row = 0usize;
        let mut caret = false;
        for row in 0..buffer.area.height {
            let mut rule = 0usize;
            for column in 0..buffer.area.width {
                let symbol = buffer[(column, row)].symbol();
                if symbol == "─" {
                    rule += 1;
                }
                if symbol == "›" {
                    caret = true;
                }
            }
            rule_in_row = rule_in_row.max(rule);
        }
        assert_eq!(
            (rule_in_row >= 20, caret),
            (true, true),
            "the key step must draw the solid input underline and mark the active step with a caret"
        );
    }

    #[test]
    fn done_artifact_rows_get_the_hand_pointer_and_plain_cells_get_the_arrow() {
        let app = App::new(pair()).with_screen(Screen::Done).done_published(
            "deck.apkg",
            "report.pdf",
            "/tmp/kamishibai-out",
        );
        assert_eq!(
            (
                mouse_pointer_at(&app, terminal(), 6, 3),
                mouse_pointer_at(&app, terminal(), 8, 3),
                mouse_pointer_at(&app, terminal(), 13, 3),
                mouse_pointer_at(&app, terminal(), 14, 3),
                mouse_pointer_at(&app, terminal(), 16, 3),
                mouse_pointer_at(&app, terminal(), 5, 3),
                mouse_pointer_at(&app, terminal(), 40, 3),
            ),
            (
                MousePointer::Arrow,
                MousePointer::Hand,
                MousePointer::Hand,
                MousePointer::Arrow,
                MousePointer::Arrow,
                MousePointer::Arrow,
                MousePointer::Arrow,
            ),
            "done output rows must use the hand on the placeholder label and arrow outside it"
        );
    }

    #[test]
    fn rendered_underlined_file_cells_are_inside_hand_pointer_regions() {
        let apps = [
            App::new(pair())
                .with_screen(Screen::YourCards)
                .confirmed_learning("en")
                .cards_started(vec![card("whilst")]),
            App::new(pair()).with_screen(Screen::Done).done_published(
                "deck.apkg",
                "report.pdf",
                "/tmp/kamishibai-out",
            ),
        ];
        let misses = apps
            .iter()
            .flat_map(|app| {
                rendered_link_cells(app)
                    .into_iter()
                    .filter(|(column, row)| {
                        mouse_pointer_at(app, terminal(), *column, *row) != MousePointer::Hand
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert!(
            misses.is_empty(),
            "every rendered underlined file cell must hit a hand pointer region, missed {misses:?}"
        );
    }

    #[test]
    fn pointer_writer_emits_css_hand_and_arrow_only_when_shape_changes() {
        let mut output = Vec::new();
        let mut current = MousePointer::Arrow;
        write_mouse_pointer_once_for(
            &mut output,
            &mut current,
            MousePointer::Hand,
            PointerDialect::Css,
        );
        write_mouse_pointer_once_for(
            &mut output,
            &mut current,
            MousePointer::Hand,
            PointerDialect::Css,
        );
        write_mouse_pointer_once_for(
            &mut output,
            &mut current,
            MousePointer::Arrow,
            PointerDialect::Css,
        );
        assert_eq!(
            String::from_utf8(output).expect("pointer bytes must be utf-8"),
            "\x1b]22;pointer\x1b\\\x1b]22;default\x1b\\",
            "pointer writer must emit OSC 22 hand/default and avoid duplicate hover writes"
        );
    }

    #[test]
    fn pointer_writer_emits_iterm_hand_and_arrow_names() {
        let mut output = Vec::new();
        write_mouse_pointer_for(&mut output, MousePointer::Hand, PointerDialect::ItermX11);
        write_mouse_pointer_for(&mut output, MousePointer::Arrow, PointerDialect::ItermX11);
        assert_eq!(
            String::from_utf8(output).expect("pointer bytes must be utf-8"),
            "\x1b]22;hand2\x1b\\\x1b]22;left_ptr\x1b\\",
            "iTerm pointer writer must use the X11 names that iTerm maps to hand and arrow"
        );
    }
}
