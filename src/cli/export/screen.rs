//! State and rendering for the standalone Anki export conversation.

use std::io;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::anki::history::{DeckSummary, Preview};
use crate::tui::palette::{self, Ink};
use crate::tui::wrap_words;

/// One requested operation; only the terminal driver performs I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    /// Recheck ownership and discover available profiles.
    Check,
    /// Read deck counts from the focused profile.
    Inspect,
    /// Prepare the selected snapshot without writing output.
    Review,
    /// Write the snapshot after its separate confirmation.
    Save,
    /// Cancel a pending publication and await cleanup.
    Cancel,
    /// Return from the standalone screen.
    Exit,
    /// Keep the screen active without an I/O effect.
    Wait,
}

#[derive(Clone)]
enum Page {
    Checking,
    Close(String),
    Profile,
    Loading,
    Decks,
    Reading,
    Summary(Preview),
    Confirm(Preview),
    Saving,
    Cancelling,
    Saved(PathBuf),
    Error(String),
}

struct Choices {
    profiles: Vec<PathBuf>,
    profile: usize,
    decks: Vec<DeckSummary>,
    cursor: usize,
}

/// A small independent state machine; it never touches the generation App.
pub(super) struct Screen {
    page: Page,
    choices: Choices,
    directory: PathBuf,
    scroll: u16,
}

impl Screen {
    fn new(page: Page, choices: Choices, directory: PathBuf, scroll: u16) -> Self {
        Self {
            page,
            choices,
            directory,
            scroll,
        }
    }

    /// Begin with a visible Anki ownership check before reading any collection.
    pub(super) fn from_directory(directory: PathBuf) -> Self {
        Self::new(
            Page::Checking,
            Choices {
                profiles: Vec::new(),
                profile: 0,
                decks: Vec::new(),
                cursor: 0,
            },
            directory,
            0,
        )
    }

    /// Show all closed profiles without implicitly choosing among several.
    pub(super) fn checked(&mut self, profiles: Vec<PathBuf>) {
        self.choices.profiles = profiles;
        self.choices.profile = 0;
        self.page = Page::Profile;
    }

    /// Present exact home decks with the previous command-line/default selection.
    pub(super) fn inspected(&mut self, decks: Vec<DeckSummary>) {
        self.choices.decks = decks;
        self.choices.cursor = 0;
        self.page = Page::Decks;
    }

    /// Present the summary of the snapshot held by the terminal driver.
    pub(super) fn reviewed(&mut self, preview: Preview) {
        self.page = Page::Summary(preview);
        self.scroll = 0;
    }

    /// Finish only after the archive has been verified and published.
    pub(super) fn saved(&mut self, path: PathBuf) {
        self.page = Page::Saved(path);
        self.scroll = 0;
    }

    /// Keep failures visible and allow a fresh closed-Anki check before retrying.
    pub(super) fn failed(&mut self, reason: String, open: bool) {
        self.page = if open {
            Page::Close(reason)
        } else {
            Page::Error(reason)
        };
        self.scroll = 0;
    }

    /// Copy the explicitly focused collection and marked deck names into a read request.
    pub(super) fn select(&self) -> (PathBuf, Vec<String>) {
        let mut names = Vec::new();
        for deck in &self.choices.decks {
            deck.include(&mut names);
        }
        (self.choices.profiles[self.choices.profile].clone(), names)
    }

    /// Apply navigation and return an effect only at a deliberate forward step.
    pub(super) fn key(&mut self, key: KeyCode) -> Intent {
        match key {
            KeyCode::Esc | KeyCode::Left => return self.back(),
            KeyCode::Up | KeyCode::Down => self.move_to(key == KeyCode::Down),
            KeyCode::Char(' ') if matches!(self.page, Page::Decks) => {
                if let Some(deck) = self.choices.decks.get_mut(self.choices.cursor) {
                    deck.toggle();
                }
            }
            KeyCode::Enter | KeyCode::Right => return self.forward(),
            _ => {}
        }
        Intent::Wait
    }

    fn back(&mut self) -> Intent {
        self.scroll = 0;
        self.page = match &self.page {
            Page::Summary(_) => Page::Decks,
            Page::Confirm(preview) => Page::Summary(preview.clone()),
            Page::Decks => Page::Profile,
            Page::Saving => {
                self.page = Page::Cancelling;
                return Intent::Cancel;
            }
            Page::Cancelling => return Intent::Wait,
            _ => return Intent::Exit,
        };
        Intent::Wait
    }

    fn forward(&mut self) -> Intent {
        self.scroll = 0;
        let (page, intent) = match &self.page {
            Page::Close(_) | Page::Error(_) => (Page::Checking, Intent::Check),
            Page::Profile => (Page::Loading, Intent::Inspect),
            Page::Decks if !self.select().1.is_empty() => (Page::Reading, Intent::Review),
            Page::Summary(preview) => (Page::Confirm(preview.clone()), Intent::Wait),
            Page::Confirm(_) => (Page::Saving, Intent::Save),
            Page::Saved(_) => return Intent::Exit,
            _ => return Intent::Wait,
        };
        self.page = page;
        intent
    }

    fn move_to(&mut self, down: bool) {
        let (index, length) = match self.page {
            Page::Profile => (&mut self.choices.profile, self.choices.profiles.len()),
            Page::Decks => (&mut self.choices.cursor, self.choices.decks.len()),
            _ => {
                self.scroll = if down {
                    self.scroll.saturating_add(1)
                } else {
                    self.scroll.saturating_sub(1)
                };
                return;
            }
        };
        *index = if down {
            index.saturating_add(1).min(length.saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
    }

    /// Paint the current step using the main TUI's ink, focus and title styles.
    pub(super) fn draw(&mut self, frame: &mut Frame, tick: usize) {
        let area = frame.area();
        frame.render_widget(Block::default().style(palette::base()), area);
        let area = area.inner(Margin::new(if area.width >= 60 { 4 } else { 1 }, 1));
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" export from Anki ", palette::invert()),
                Span::styled(format!("  {}", self.title()), Ink::Detail.on(false)),
            ])),
            header,
        );
        if matches!(self.page, Page::Profile | Page::Decks) {
            let (items, cursor) = if matches!(self.page, Page::Profile) {
                (
                    self.choices
                        .profiles
                        .iter()
                        .map(|path| {
                            ListItem::new(
                                path.parent()
                                    .and_then(|path| path.file_name())
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    self.choices.profile,
                )
            } else {
                (
                    self.choices
                        .decks
                        .iter()
                        .map(|deck| {
                            ListItem::new(
                                wrapped(
                                    &written(|out| deck.write(out)),
                                    body.width.saturating_sub(2),
                                )
                                .join("\n"),
                            )
                        })
                        .collect::<Vec<_>>(),
                    self.choices.cursor,
                )
            };
            let [label, list] =
                Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(body);
            let text = if matches!(self.page, Page::Profile) {
                "Anki is closed · choose your profile"
            } else if self.select().1.is_empty() {
                "choose decks · mark at least one deck with Space"
            } else {
                "choose decks · Space marks each exact deck"
            };
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .style(Ink::Detail.on(false)),
                label,
            );
            let mut state = ListState::default()
                .with_offset(usize::from(self.scroll))
                .with_selected(Some(cursor));
            frame.render_stateful_widget(
                List::new(items)
                    .style(Ink::Subject.on(false))
                    .highlight_style(Ink::Subject.on(true))
                    .highlight_symbol("› "),
                list,
                &mut state,
            );
            self.scroll = u16::try_from(state.offset()).unwrap_or(u16::MAX);
        } else {
            let text = wrapped(&self.text(tick), body.width);
            let lines = text.len();
            let paragraph = Paragraph::new(text.join("\n")).style(Ink::Detail.on(false));
            let maximum =
                u16::try_from(lines.saturating_sub(usize::from(body.height))).unwrap_or(u16::MAX);
            self.scroll = self.scroll.min(maximum);
            frame.render_widget(paragraph.scroll((self.scroll, 0)), body);
        }
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n{}",
                "─".repeat(usize::from(footer.width)),
                self.hint()
            ))
            .style(Ink::Detail.on(false))
            .wrap(Wrap { trim: false }),
            footer,
        );
    }

    fn title(&self) -> &str {
        match self.page {
            Page::Checking => "1 / 5 · check",
            Page::Close(_) => "1 / 5 · close Anki",
            Page::Profile => "1 / 5 · profile",
            Page::Loading | Page::Decks => "2 / 5 · decks",
            Page::Reading | Page::Summary(_) => "3 / 5 · review",
            Page::Confirm(_) => "4 / 5 · confirm",
            Page::Saving | Page::Cancelling | Page::Saved(_) => "5 / 5 · file",
            Page::Error(_) => "export paused",
        }
    }

    fn text(&self, tick: usize) -> String {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][tick % 10];
        match &self.page {
            Page::Checking => format!("{spinner} checking Anki…"),
            Page::Close(reason) => format!(
                "close Anki\n\nSync on your devices, then quit desktop Anki normally.\nWhen it is closed, press Enter to check again.\n\n{reason}"
            ),
            Page::Loading => format!("{spinner} reading decks and review counts…"),
            Page::Reading => format!("{spinner} preparing your export summary…"),
            Page::Summary(preview) => format!(
                "review your export\n\n{}\nSave to: {}\n\nNothing is written until you confirm.",
                written(|out| preview.summarize(out)),
                self.directory.display()
            ),
            Page::Confirm(_) => format!(
                "save this file?\n\nYour selected cards, review history and referenced media\nwill be saved to:\n\n{}\n\nAnki stays unchanged.\nPress Enter to save.",
                self.directory.display()
            ),
            Page::Saving => format!(
                "{spinner} saving and verifying your export…\n\n{}",
                self.directory.display()
            ),
            Page::Cancelling => format!(
                "{spinner} cancelling the export…\n\nRemoving the temporary file before exiting."
            ),
            Page::Saved(path) => format!(
                "export saved\n\n{}\n\nin the web: import from kamishibai cli",
                path.display()
            ),
            Page::Error(reason) => format!(
                "could not export\n\n{reason}\n\nPress Enter to check again, or Esc to exit."
            ),
            Page::Profile | Page::Decks => String::new(),
        }
    }

    fn hint(&self) -> &str {
        match self.page {
            Page::Profile => "[↑↓] profile   [Enter] next   [Esc] exit",
            Page::Decks => "[↑↓] deck   [Space] mark   [Enter] review   [Esc] back",
            Page::Summary(_) => "[↑↓] scroll   [Enter] next   [Esc] back",
            Page::Confirm(_) => "[Enter] save   [Esc] back",
            Page::Close(_) => "[Enter] ready · check again   [Esc] exit",
            Page::Saved(_) => "[↑↓] scroll   [Enter / Esc] exit",
            Page::Saving => "[Esc] cancel before the file is published",
            Page::Cancelling => "waiting for temporary-file cleanup",
            Page::Error(_) => "[Enter] retry   [Esc] exit",
            _ => "[Esc] exit without saving",
        }
    }
}

fn wrapped(text: &str, width: u16) -> Vec<String> {
    text.lines()
        .flat_map(|line| wrap_words(line, usize::from(width), usize::from(width)))
        .collect()
}

fn written(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
    let mut bytes = Vec::new();
    write(&mut bytes).expect("invariant: writing a summary into Vec cannot fail");
    String::from_utf8(bytes).expect("invariant: summary contains only UTF-8 text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anki::history::{self, AnkiProcess};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::Path;

    struct Closed;

    impl AnkiProcess for Closed {
        fn running(&self, _: &Path) -> io::Result<bool> {
            Ok(false)
        }
    }

    fn decks() -> Vec<DeckSummary> {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("collection.anki2");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE col(ver INTEGER,crt INTEGER,mod INTEGER);
            INSERT INTO col VALUES(18,17,29);
            CREATE TABLE decks(id INTEGER,name TEXT);
            INSERT INTO decks VALUES(7,'French'),(19,'French'||char(31)||'Daily'),(31,'Other');
            CREATE TABLE notetypes(id INTEGER,name TEXT);
            INSERT INTO notetypes VALUES(13,'Kamishibai Vocabulary Model FR');
            CREATE TABLE fields(ntid INTEGER,ord INTEGER,name TEXT);
            CREATE TABLE notes(id INTEGER,guid TEXT,mid INTEGER,mod INTEGER,tags TEXT,flds TEXT);
            INSERT INTO notes VALUES(23,'g',13,3,'','chat');
            CREATE TABLE cards(id INTEGER,nid INTEGER,did INTEGER,ord INTEGER,type INTEGER,queue INTEGER,due INTEGER,ivl INTEGER,factor INTEGER,reps INTEGER,lapses INTEGER,odid INTEGER,odue INTEGER);
            INSERT INTO cards VALUES(37,23,7,0,2,2,41,11,2500,5,1,0,0);
            CREATE TABLE revlog(id INTEGER,cid INTEGER,ease INTEGER,ivl INTEGER,lastIvl INTEGER,factor INTEGER,time INTEGER,type INTEGER);
            INSERT INTO revlog VALUES(101,37,1,5,3,2500,671,1);",
        ).unwrap();
        drop(db);
        history::inspect(&path, &[], &Closed, "unknown").unwrap()
    }

    fn preview() -> Preview {
        history::validate(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anki-history/v1.zip"),
        )
        .unwrap()
    }

    fn screen(page: Page) -> Screen {
        Screen::new(
            page,
            Choices {
                profiles: vec![
                    PathBuf::from("/Anki2/User 1/collection.anki2"),
                    PathBuf::from("/Anki2/User 2/collection.anki2"),
                ],
                profile: 0,
                decks: decks(),
                cursor: 0,
            },
            PathBuf::from("/Users/learner/Downloads"),
            0,
        )
    }

    fn render(screen: &mut Screen, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| screen.draw(frame, 0)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    #[test]
    fn export_steps_keep_their_reviewed_rendering() {
        let pages = [
            Page::Checking,
            Page::Close("Anki is open or did not close properly".into()),
            Page::Profile,
            Page::Loading,
            Page::Decks,
            Page::Reading,
            Page::Summary(preview()),
            Page::Confirm(preview()),
            Page::Saving,
            Page::Cancelling,
            Page::Saved(PathBuf::from(
                "/Users/learner/Downloads/kamishibai-anki-2026-11-07.zip",
            )),
            Page::Error("cannot save Anki history inside the Anki profile or Anki2 folder".into()),
        ];
        let rendered = pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                format!("STEP {}\n{}", index + 1, render(&mut screen(page), 80, 20))
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("export_steps", rendered);
    }

    #[test]
    fn narrow_export_keeps_the_confirmation_and_next_step_readable() {
        let pages = [
            Page::Confirm(preview()),
            Page::Saved(PathBuf::from(
                "/Users/learner/Downloads/kamishibai-anki-2026-11-07.zip",
            )),
        ];
        let rendered = pages
            .into_iter()
            .map(|page| render(&mut screen(page), 42, 16))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("export_narrow", rendered);
    }

    #[test]
    fn summary_cannot_start_saving_without_a_second_confirmation() {
        let mut screen = screen(Page::Summary(preview()));
        let first = screen.key(KeyCode::Enter);
        let second = screen.key(KeyCode::Enter);
        assert_eq!(
            (first, second),
            (Intent::Wait, Intent::Save),
            "review skipped the separate save confirmation"
        );
    }

    #[test]
    fn back_from_confirmation_cannot_request_a_write() {
        let mut screen = screen(Page::Confirm(preview()));
        let intent = screen.key(KeyCode::Esc);
        assert!(
            intent == Intent::Wait && matches!(screen.page, Page::Summary(_)),
            "back from confirmation wrote or discarded the review"
        );
    }

    #[test]
    fn an_empty_selection_cannot_fall_back_to_default_decks() {
        let mut screen = screen(Page::Decks);
        screen.key(KeyCode::Char(' '));
        assert_eq!(
            screen.key(KeyCode::Enter),
            Intent::Wait,
            "an empty user selection fell back to default decks"
        );
    }

    #[test]
    fn deck_navigation_cannot_select_a_descendant_implicitly() {
        let mut screen = screen(Page::Decks);
        screen.key(KeyCode::Char(' '));
        screen.key(KeyCode::Down);
        screen.key(KeyCode::Char(' '));
        assert_eq!(
            screen.select().1,
            vec!["French::Daily"],
            "choosing a subdeck also chose its parent"
        );
    }

    #[test]
    fn choosing_another_profile_cannot_reuse_the_previous_profile() {
        let mut screen = screen(Page::Profile);
        screen.key(KeyCode::Down);
        assert_eq!(
            screen.select().0,
            PathBuf::from("/Anki2/User 2/collection.anki2"),
            "profile navigation retained the previous source"
        );
    }

    #[test]
    fn cancelling_an_active_save_waits_for_temporary_file_cleanup() {
        let mut screen = screen(Page::Saving);
        let first = screen.key(KeyCode::Esc);
        let second = screen.key(KeyCode::Esc);
        assert_eq!(
            (first, second),
            (Intent::Cancel, Intent::Wait),
            "cancellation exited before the save worker cleaned up"
        );
    }

    #[test]
    fn summary_scrolling_cannot_hide_all_content() {
        let mut screen = screen(Page::Summary(preview()));
        for _ in 0..300 {
            screen.key(KeyCode::Down);
        }
        let rendered = render(&mut screen, 42, 12);
        assert!(
            rendered.contains("until you confirm"),
            "scrolling moved beyond the last summary line"
        );
    }
}
