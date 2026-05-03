//! Integration flow for the `Done` screen (08-done.png).

use kamishibai::session::LanguagePair;
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut rendered = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        rendered.push('\n');
    }
    rendered
}

fn published() -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::Done)
        .confirmed_target("en")
        .done_published(
            "en_2026-04-17_183029.apkg",
            "en_2026-04-17_183029.pdf",
            "kamishibai-out/",
        )
}

#[test]
fn done_screen_lists_short_artifact_labels_and_keyboard_hints() {
    let rendered = flat(&published());
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("APKG")
            && rendered.contains("PDF")
            && rendered.contains("[n]")
            && rendered.contains("new batch")
            && rendered.contains("RU → EN"),
        "Done must render compact APKG/PDF link labels and keyboard hints with the language chip"
    );
}

#[test]
fn new_batch_from_done_returns_to_your_words_with_language_kept() {
    let app = published();
    let (next, _) = transit(app, AppEvent::NewBatch);
    assert_eq!(
        (next.screen(), next.pair().label()),
        (Screen::YourWords, String::from("RU → EN")),
        "N on Done must return to Your words while keeping the language pair"
    );
}

#[test]
fn quit_from_done_requests_app_exit() {
    let app = published();
    let (_, side) = transit(app, AppEvent::Quit);
    assert_eq!(
        side,
        Side::ExitApp,
        "Q on Done must request the shell to exit the application"
    );
}
