//! State and rendering for the standalone Anki export conversation.

use std::io;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use super::overview::Overview;
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
    /// Write the snapshot after confirmation on the summary.
    Save,
    /// Cancel a pending publication and await cleanup.
    Cancel,
    /// Return from the standalone screen.
    Exit,
    /// Keep the screen active without an I/O effect.
    Wait,
}

enum Page {
    Checking,
    Close(String),
    Profile,
    Loading,
    Decks,
    Reading,
    Summary(Overview),
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
        self.page = Page::Summary(Overview::from_preview(&preview));
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
            Page::Summary(_) => (Page::Saving, Intent::Save),
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

    /// Paint the conversation with the main TUI's title, ink and status-bar grammar.
    pub(super) fn draw(&mut self, frame: &mut Frame, tick: usize) {
        let area = frame.area();
        frame.render_widget(Block::default().style(palette::base()), area);
        let gutter = if area.width >= 60 { 4 } else { 1 };
        let [_, header, _, body, reminder, rule, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let header = header.inner(Margin::new(gutter, 0));
        let body = body.inner(Margin::new(gutter, 0));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" export from Anki ", palette::invert()),
                Span::styled(format!("  {}", self.hint()), Ink::Detail.on(false)),
            ])),
            header,
        );
        if matches!(self.page, Page::Profile | Page::Decks) {
            self.list(frame, body);
        } else {
            let rows = wrapped(&self.text(tick), body.width);
            let maximum = u16::try_from(rows.len().saturating_sub(usize::from(body.height)))
                .unwrap_or(u16::MAX);
            self.scroll = self.scroll.min(maximum);
            let lines = rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| {
                    Line::styled(
                        row,
                        if index == 0 {
                            Ink::Subject.on(false)
                        } else {
                            Ink::Detail.on(false)
                        },
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(lines).scroll((self.scroll, 0)), body);
        }
        frame.render_widget(
            Paragraph::new("Anki stays unchanged")
                .alignment(Alignment::Right)
                .style(Ink::Aside.on(false)),
            reminder.inner(Margin::new(gutter, 0)),
        );
        frame.render_widget(
            Paragraph::new(Line::from(
                (0..rule.width)
                    .map(|column| {
                        Span::styled(
                            " ",
                            if column % 2 == 0 {
                                palette::rule().add_modifier(Modifier::CROSSED_OUT)
                            } else {
                                palette::base()
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            rule,
        );
        let footer = footer.inner(Margin::new(gutter, 0));
        frame.render_widget(Paragraph::new(self.footer(footer.width)), footer);
    }

    fn list(&mut self, frame: &mut Frame, area: Rect) {
        let profile = matches!(self.page, Page::Profile);
        let (title, instruction) = if profile {
            ("01 which profile?", "Anki is closed")
        } else if self.select().1.is_empty() {
            ("02 choose decks", "mark at least one deck with Space")
        } else {
            ("02 choose decks", "Space selects each deck separately")
        };
        let [label, list] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(title, Ink::Subject.on(false)),
                Line::styled(instruction, Ink::Detail.on(false)),
            ]),
            label,
        );
        let (items, cursor) = if profile {
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
                                list.width.saturating_sub(2),
                            )
                            .join("\n"),
                        )
                    })
                    .collect::<Vec<_>>(),
                self.choices.cursor,
            )
        };
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
    }

    fn text(&self, tick: usize) -> String {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][tick % 10];
        match &self.page {
            Page::Checking => format!("{spinner} checking Anki…"),
            Page::Close(reason) => format!(
                "01  close Anki\n\nSync, then quit desktop Anki.\nPress Enter to check again.\n\n{reason}"
            ),
            Page::Loading => format!("{spinner} reading your decks…"),
            Page::Reading => format!("{spinner} counting cards and reviews…"),
            Page::Summary(preview) => format!(
                "03  review your export\n\n{}\nto {}\nnot saved yet",
                preview.describe(),
                self.directory.display()
            ),
            Page::Saving => format!(
                "{spinner} saving and verifying your export…\n\n{}",
                self.directory.display()
            ),
            Page::Cancelling => {
                format!("{spinner} cancelling the export…\n\nRemoving the temporary file.")
            }
            Page::Saved(path) => format!(
                "✓  export saved\n\n{}\n\nIn Kamishibai web → your account:\nimport from kamishibai cli",
                path.display()
            ),
            Page::Error(reason) => format!("could not export\n\n{reason}"),
            Page::Profile | Page::Decks => String::new(),
        }
    }

    fn hint(&self) -> &str {
        match self.page {
            Page::Checking | Page::Loading | Page::Reading => "reading Anki",
            Page::Close(_) => "close Anki first",
            Page::Profile => "choose your profile",
            Page::Decks => "choose what to take",
            Page::Summary(_) => "review, then save",
            Page::Saving => "writing your file",
            Page::Cancelling => "cleaning up",
            Page::Saved(_) => "your file is ready",
            Page::Error(_) => "try again when ready",
        }
    }

    fn action(&self) -> String {
        match self.page {
            Page::Profile => String::from("[Enter] choose decks"),
            Page::Decks if !self.select().1.is_empty() => String::from("[Enter] review export"),
            Page::Decks => String::from("[Space] select deck"),
            Page::Summary(_) => {
                if self
                    .directory
                    .file_name()
                    .is_some_and(|name| name == "Downloads")
                {
                    String::from("[Enter] save to Downloads")
                } else {
                    String::from("[Enter] save ZIP")
                }
            }
            Page::Close(_) | Page::Error(_) => String::from("[Enter] check again"),
            Page::Saved(_) => String::from("[Enter / Esc] exit"),
            _ => String::new(),
        }
    }

    fn footer(&self, width: u16) -> Line<'static> {
        let action = self.action();
        let escape = match self.page {
            Page::Summary(_) | Page::Decks => "[Esc] back",
            Page::Saving => "[Esc] cancel",
            Page::Cancelling | Page::Saved(_) => "",
            _ => "[Esc] exit",
        };
        let navigation = match self.page {
            Page::Profile => "[↑↓] profile",
            Page::Decks => "[↑↓] deck  [Space] select",
            Page::Summary(_) | Page::Saved(_) | Page::Close(_) | Page::Error(_) => "[↑↓] scroll",
            _ => "",
        };
        let status = match self.page {
            Page::Checking | Page::Loading | Page::Reading => "reading",
            Page::Saving => "saving",
            Page::Cancelling => "cancelling",
            Page::Saved(_) => "saved",
            Page::Error(_) | Page::Close(_) => "paused",
            _ => "not saved yet",
        };
        let mut hints = vec![
            (action, Ink::Subject),
            (navigation.to_string(), Ink::Aside),
            (escape.to_string(), Ink::Aside),
        ];
        hints.retain(|(text, _)| !text.is_empty());
        let measure = |hints: &[(String, Ink)]| {
            hints
                .iter()
                .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
                .sum::<usize>()
                + hints.len().saturating_sub(1) * 2
        };
        if measure(&hints) > usize::from(width) {
            hints.retain(|(text, _)| text != navigation);
        }
        if measure(&hints) > usize::from(width) {
            hints.retain(|(text, _)| text != escape);
        }
        let used = measure(&hints);
        let mut spans = Vec::new();
        if used + status.len() + 2 <= usize::from(width) {
            spans.push(Span::styled(status.to_string(), Ink::Aside.on(false)));
            spans.push(Span::raw(
                " ".repeat(usize::from(width) - used - status.len()),
            ));
        } else if used < usize::from(width) {
            spans.push(Span::raw(" ".repeat(usize::from(width) - used)));
        }
        for (index, (text, ink)) in hints.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(text, ink.on(false)));
        }
        Line::from(spans)
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

    fn preview() -> Overview {
        Overview::from_preview(
            &history::validate(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/anki-history/v1.zip"),
            )
            .unwrap(),
        )
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

    fn pages() -> Vec<Page> {
        vec![
            Page::Checking,
            Page::Close("Anki is open or did not close properly".into()),
            Page::Profile,
            Page::Loading,
            Page::Decks,
            Page::Reading,
            Page::Summary(preview()),
            Page::Saving,
            Page::Cancelling,
            Page::Saved(PathBuf::from(
                "/Users/learner/Downloads/kamishibai-anki-2026-11-07.zip",
            )),
            Page::Error("cannot save Anki history inside the Anki profile or Anki2 folder".into()),
        ]
    }

    #[test]
    fn export_steps_keep_their_reviewed_rendering() {
        let rendered = pages()
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                format!("STEP {}\n{}", index + 1, render(&mut screen(page), 120, 36))
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("export_steps", rendered);
    }

    #[test]
    fn narrow_export_keeps_the_confirmation_and_next_step_readable() {
        let rendered = pages()
            .into_iter()
            .map(|page| render(&mut screen(page), 42, 16))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("export_narrow", rendered);
    }

    #[test]
    fn summary_confirmation_cannot_require_another_enter() {
        let mut screen = screen(Page::Summary(preview()));
        let first = screen.key(KeyCode::Enter);
        let second = screen.key(KeyCode::Enter);
        assert_eq!(
            (first, second),
            (Intent::Save, Intent::Wait),
            "the explicit save action required another confirmation or started twice"
        );
    }

    #[test]
    fn back_from_summary_cannot_request_a_write() {
        let results = [KeyCode::Esc, KeyCode::Left].map(|key| {
            let mut screen = screen(Page::Summary(preview()));
            (screen.key(key), matches!(screen.page, Page::Decks))
        });
        assert_eq!(
            results,
            [(Intent::Wait, true), (Intent::Wait, true)],
            "back from the summary wrote or discarded the deck choices"
        );
    }

    #[test]
    fn narrow_summary_cannot_hide_the_save_action_or_destination() {
        let rendered = render(&mut screen(Page::Summary(preview())), 42, 16);
        assert!(
            rendered.contains("[Enter] save to Downloads")
                && rendered.contains("/Users/learner/Downloads")
                && rendered.contains("not saved yet")
                && !rendered.contains("references")
                && !rendered.contains("mistakes"),
            "the narrow summary hid the decision behind technical counts or navigation"
        );
    }

    #[test]
    fn an_empty_checklist_cannot_offer_an_inert_enter() {
        let visible = [42, 120].into_iter().all(|width| {
            let mut screen = screen(Page::Decks);
            screen.key(KeyCode::Char(' '));
            let rendered = render(&mut screen, width, 16);
            rendered.contains("[Space] select deck") && !rendered.contains("[Enter]")
        });
        assert!(
            visible,
            "an empty checklist advertised an Enter action that cannot run"
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
            rendered.contains("not saved yet"),
            "scrolling moved beyond the last summary line"
        );
    }
}
