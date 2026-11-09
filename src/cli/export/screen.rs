//! State and rendering for the standalone Anki export conversation.

use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::overview::{DeckRow, Overview, deck_lines, quantity, wrapped};
use crate::anki::history::{DeckSummary, Preview};
use crate::runtime::locations::compact_path;
use crate::tui::palette::{self, Ink};
use crate::tui::{
    FooterHint, artifact_rows, back_hint, dashed_line, exit_hint, footer_bar, header, loader,
    status_sep,
};

const IMPORT_PREFIX: &str = "import at ";
const IMPORT_DOMAIN: &str = "app.kamishibai.io";

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

/// An actionable refusal, independent of console error prose.
pub(super) enum Refusal {
    /// Anki still owns the collection.
    Open,
    /// The reviewed source changed.
    Changed,
    /// Discovery found no collections in these roots.
    NoProfiles { roots: String },
    /// An explicit profile name was absent.
    NoProfile { name: String, available: String },
    /// An exact deck name was absent.
    Deck(String),
    /// A selected collection path cannot be read.
    Collection(String),
    /// Output is inside a protected Anki directory.
    Destination,
    /// The selected snapshot exceeds the file limit.
    Oversized { decks: usize },
    /// This collection schema is unsupported.
    Schema,
    /// An operational error needs diagnosis.
    Other(String),
}

struct Stop {
    refusal: Refusal,
    step: u8,
}

struct Outcome {
    path: PathBuf,
    overview: Overview,
}

enum Page {
    Checking,
    Profile,
    Loading,
    Decks,
    Reading,
    Summary(Overview),
    Saving(Overview),
    Cancelling(Overview),
    Saved(Outcome),
    Stopped(Stop),
}

struct Choices {
    profiles: Vec<PathBuf>,
    profile: usize,
    decks: Vec<DeckSummary>,
    cursor: usize,
}

struct Viewport {
    scroll: u16,
    entered: Instant,
}

impl Viewport {
    fn new(scroll: u16, entered: Instant) -> Self {
        Self { scroll, entered }
    }
}

/// A small independent state machine; it never touches the generation App.
pub(super) struct Screen {
    page: Page,
    choices: Choices,
    folder: String,
    viewport: Viewport,
}

impl Screen {
    fn new(page: Page, choices: Choices, folder: String, viewport: Viewport) -> Self {
        Self {
            page,
            choices,
            folder,
            viewport,
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
            compact_path(&directory),
            Viewport::new(0, Instant::now()),
        )
    }

    fn show(&mut self, page: Page) {
        self.page = page;
        self.viewport.scroll = 0;
        self.viewport.entered = Instant::now();
    }

    /// Show all profiles without claiming that each has passed its ownership check.
    pub(super) fn checked(&mut self, profiles: Vec<PathBuf>) {
        self.choices.profiles = profiles;
        self.choices.profile = 0;
        self.show(Page::Profile);
    }

    /// Sort exact decks by path so selection and painted tree share one order.
    pub(super) fn inspected(&mut self, mut decks: Vec<DeckSummary>) {
        decks.sort_by_key(|deck| DeckRow::from_deck(deck).order());
        self.choices.decks = decks;
        self.choices.cursor = 0;
        self.show(Page::Decks);
    }

    /// Present the summary of the snapshot held by the terminal driver.
    pub(super) fn reviewed(&mut self, preview: Preview) {
        self.show(Page::Summary(Overview::from_preview(&preview)));
    }

    /// Finish only after the archive has been verified and published.
    pub(super) fn saved(&mut self, path: PathBuf) {
        let overview = match &self.page {
            Page::Saving(overview) | Page::Cancelling(overview) => overview.clone(),
            _ => panic!("invariant: publication follows a reviewed save"),
        };
        self.show(Page::Saved(Outcome { path, overview }));
    }

    /// Keep the refusal at the step where it happened.
    pub(super) fn failed(&mut self, refusal: Refusal) {
        let step = if matches!(&refusal, Refusal::Destination) {
            3
        } else {
            self.step()
        };
        self.show(Page::Stopped(Stop { refusal, step }));
    }

    /// Copy the focused collection and exact marked deck names into a read request.
    pub(super) fn select(&self) -> (PathBuf, Vec<String>) {
        let mut names = Vec::new();
        for deck in &self.choices.decks {
            deck.include(&mut names);
        }
        (self.choices.profiles[self.choices.profile].clone(), names)
    }

    /// Interrupt the entire export, waiting for cleanup only during publication.
    pub(super) fn interrupt(&mut self) -> Intent {
        match &self.page {
            Page::Saving(overview) => {
                self.show(Page::Cancelling(overview.clone()));
                Intent::Cancel
            }
            Page::Cancelling(_) => Intent::Wait,
            _ => Intent::Exit,
        }
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
        match self.page {
            Page::Summary(_) | Page::Stopped(Stop { step: 3, .. }) => self.show(Page::Decks),
            Page::Decks => self.show(Page::Profile),
            Page::Saving(_) | Page::Cancelling(_) => return self.interrupt(),
            _ => return Intent::Exit,
        }
        Intent::Wait
    }

    fn forward(&mut self) -> Intent {
        let (page, intent) = match &self.page {
            Page::Stopped(Stop {
                refusal: Refusal::Oversized { .. },
                ..
            }) => (Page::Decks, Intent::Wait),
            Page::Stopped(Stop {
                refusal: Refusal::Other(_),
                step: 3,
            }) => (Page::Reading, Intent::Review),
            Page::Stopped(stop) if stop.refusal.retry() => (Page::Checking, Intent::Check),
            Page::Profile => (Page::Loading, Intent::Inspect),
            Page::Decks if !self.select().1.is_empty() => (Page::Reading, Intent::Review),
            Page::Summary(overview) => (Page::Saving(overview.clone()), Intent::Save),
            Page::Saved(_) => return Intent::Exit,
            _ => return Intent::Wait,
        };
        self.show(page);
        intent
    }

    fn move_to(&mut self, down: bool) {
        let (index, length) = match self.page {
            Page::Profile => (&mut self.choices.profile, self.choices.profiles.len()),
            Page::Decks => (&mut self.choices.cursor, self.choices.decks.len()),
            _ => {
                self.viewport.scroll = if down {
                    self.viewport.scroll.saturating_add(1)
                } else {
                    self.viewport.scroll.saturating_sub(1)
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

    /// Resolve only the underlined labels on the verified outcome page.
    pub(super) fn link(&self, column: u16, row: u16, width: u16) -> Option<(PathBuf, bool)> {
        let Page::Saved(outcome) = &self.page else {
            return None;
        };
        if row < 3 {
            return None;
        }
        let row = row.checked_add(self.viewport.scroll)?;
        let start = gutter(width) + 4;
        let domain = gutter(width)
            + u16::try_from(IMPORT_PREFIX.width())
                .expect("invariant: the import prefix fits terminal coordinates");
        let length = u16::try_from(IMPORT_DOMAIN.width())
            .expect("invariant: the import domain fits terminal coordinates");
        match row {
            3 if (start..start + 6).contains(&column) => {
                Some((outcome.path.parent()?.to_path_buf(), false))
            }
            4 if (start..start + 4).contains(&column) => Some((outcome.path.clone(), true)),
            9 if (domain..domain + length).contains(&column) => {
                Some((PathBuf::from(super::IMPORT_URL), false))
            }
            _ => None,
        }
    }

    /// Paint with the main TUI's existing title, loader, artifact strip and footer.
    pub(super) fn draw(&mut self, frame: &mut Frame, _tick: usize) {
        let area = frame.area();
        let actions = if matches!(self.page, Page::Decks) {
            let mut spans = Vec::new();
            for hint in self.deck_hints() {
                if !spans.is_empty() {
                    spans.push(status_sep());
                }
                spans.extend(hint.spans());
            }
            Line::from(spans)
        } else {
            Line::default()
        };
        let stacked = matches!(self.page, Page::Decks)
            && Line::from(self.status()).width() + 2 + actions.width()
                > usize::from(area.width.saturating_sub(2 * gutter(area.width)));
        frame.render_widget(Block::default().style(palette::base()), area);
        let [_, top, _, body, _, rule, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1 + u16::from(stacked)),
        ])
        .areas(area);
        let margin = Margin::new(gutter(area.width), 0);
        let top = top.inner(margin);
        let body = body.inner(margin);
        frame.render_widget(
            header("export from anki", self.hint(), None, top.width),
            top,
        );
        let mut overflow = false;
        match &self.page {
            Page::Profile | Page::Loading | Page::Decks | Page::Reading => self.list(frame, body),
            Page::Summary(overview) | Page::Saving(overview) | Page::Cancelling(overview) => {
                let lines = overview.summary(&self.folder, body.width);
                overflow = lines.len() > usize::from(body.height);
                self.paragraph(frame, body, lines);
            }
            Page::Saved(outcome) => {
                let folder = compact_path(
                    outcome
                        .path
                        .parent()
                        .expect("invariant: a saved file has a folder"),
                );
                let file = outcome
                    .path
                    .file_name()
                    .expect("invariant: a saved path names a file")
                    .to_string_lossy()
                    .into_owned();
                let mut lines = artifact_rows(&[("FOLDER", folder), ("FILE", file)]);
                lines.extend([
                    Line::default(),
                    Line::default(),
                    outcome.overview.receipt(),
                    Line::default(),
                    Line::from(vec![
                        Span::styled(IMPORT_PREFIX, Ink::Detail.on(false)),
                        Span::styled(IMPORT_DOMAIN, Ink::Subject.link(false)),
                    ]),
                ]);
                self.paragraph(frame, body, lines);
                if self.viewport.scroll <= 3 && body.height > 3 - self.viewport.scroll {
                    frame.render_widget(
                        Paragraph::new(dashed_line(0, usize::from(area.width))),
                        Rect::new(area.x, body.y + 3 - self.viewport.scroll, area.width, 1),
                    );
                }
            }
            Page::Stopped(stop) => {
                let lines = stop.refusal.lines(body.width);
                self.paragraph(frame, body, lines);
            }
            Page::Checking => {}
        }
        frame.render_widget(
            Paragraph::new(dashed_line(0, usize::from(rule.width))),
            rule,
        );
        let footer = footer.inner(margin);
        if stacked {
            let [status, keys] = Layout::vertical([Constraint::Length(1); 2]).areas(footer);
            frame.render_widget(Paragraph::new(Line::from(self.status())), status);
            frame.render_widget(Paragraph::new(actions).right_aligned(), keys);
        } else {
            frame.render_widget(
                footer_bar(self.status(), self.hints(overflow), footer.width),
                footer,
            );
        }
        if let Some((title, label)) = self.busy() {
            loader(frame, area, title, label, self.viewport.entered.elapsed());
        }
    }

    fn paragraph(&mut self, frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
        let maximum =
            u16::try_from(lines.len().saturating_sub(usize::from(area.height))).unwrap_or(u16::MAX);
        self.viewport.scroll = self.viewport.scroll.min(maximum);
        frame.render_widget(
            Paragraph::new(lines).scroll((self.viewport.scroll, 0)),
            area,
        );
    }

    fn list(&mut self, frame: &mut Frame, area: Rect) {
        let profile = matches!(self.page, Page::Profile | Page::Loading);
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
            let decks = self
                .choices
                .decks
                .iter()
                .map(DeckRow::from_deck)
                .collect::<Vec<_>>();
            (
                deck_lines(&decks, area.width)
                    .into_iter()
                    .map(ListItem::new)
                    .collect(),
                self.choices.cursor,
            )
        };
        let mut state = ListState::default()
            .with_offset(usize::from(self.viewport.scroll))
            .with_selected(Some(cursor));
        frame.render_stateful_widget(
            List::new(items)
                .style(Ink::Subject.on(false))
                .highlight_style(
                    Style::default()
                        .bg(palette::HL)
                        .add_modifier(Modifier::BOLD),
                ),
            area,
            &mut state,
        );
        self.viewport.scroll = u16::try_from(state.offset()).unwrap_or(u16::MAX);
    }

    fn step(&self) -> u8 {
        match &self.page {
            Page::Checking | Page::Profile | Page::Loading => 1,
            Page::Decks | Page::Reading => 2,
            Page::Stopped(stop) => stop.step,
            _ => 3,
        }
    }

    fn status(&self) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled(
            format!("step {}/3", self.step()),
            Ink::Aside.on(false),
        )];
        if matches!(self.page, Page::Decks | Page::Reading) {
            spans.push(status_sep());
            if self.select().1.is_empty() {
                spans.push(Span::styled("no decks chosen", Ink::Aside.on(false)));
            } else {
                let count = self
                    .choices
                    .decks
                    .iter()
                    .map(DeckRow::from_deck)
                    .map(|deck| deck.count())
                    .sum();
                let text = quantity(count, "card");
                let (number, noun) = text
                    .split_once(' ')
                    .expect("invariant: a quantity has a number and noun");
                spans.push(Span::styled(number.to_owned(), Ink::Subject.on(false)));
                spans.push(Span::styled(format!(" {noun}"), Ink::Detail.on(false)));
            }
        }
        spans
    }

    fn hint(&self) -> &str {
        match &self.page {
            Page::Checking | Page::Profile | Page::Loading => "choose your anki profile",
            Page::Decks | Page::Reading => "choose decks to export",
            Page::Summary(_) => "save for kamishibai web",
            Page::Saving(_) => "saving your file",
            Page::Cancelling(_) => "cancelling the save",
            Page::Saved(_) => "your file is ready",
            Page::Stopped(Stop {
                refusal: Refusal::Open | Refusal::Changed,
                ..
            }) => "close anki first",
            Page::Stopped(_) => "the export stopped",
        }
    }

    fn deck_hints(&self) -> Vec<FooterHint> {
        let mut hints = if self.select().1.is_empty() {
            vec![FooterHint::primary("Space", "select")]
        } else {
            vec![
                FooterHint::primary("Enter", "next"),
                FooterHint::secondary("Space", "select"),
            ]
        };
        hints.push(exit_hint("Esc", "back"));
        hints
    }

    fn hints(&self, overflow: bool) -> Vec<FooterHint> {
        let primary = FooterHint::primary;
        let navigation = || FooterHint::ghost("↑↓", "nav");
        let exit = || exit_hint("Esc", "exit");
        let back = || {
            if self.step() == 3 {
                back_hint()
            } else {
                exit()
            }
        };
        match &self.page {
            Page::Checking | Page::Loading | Page::Reading => vec![exit()],
            Page::Profile => vec![primary("Enter", "choose decks"), navigation(), exit()],
            Page::Decks => {
                let mut hints = self.deck_hints();
                hints.insert(hints.len() - 1, navigation());
                hints
            }
            Page::Summary(_) => {
                let mut hints = vec![primary("Enter", "save")];
                if overflow {
                    hints.push(FooterHint::ghost("↑↓", "scroll"));
                }
                hints.push(back_hint());
                hints
            }
            Page::Saving(_) => vec![exit_hint("Esc", "cancel")],
            Page::Cancelling(_) => Vec::new(),
            Page::Saved(_) => vec![primary("Enter", "exit")],
            Page::Stopped(Stop {
                refusal: Refusal::Oversized { .. },
                ..
            }) => vec![primary("Enter", "choose decks"), back()],
            Page::Stopped(stop) if stop.refusal.retry() => {
                vec![primary("Enter", "check again"), back()]
            }
            Page::Stopped(_) => vec![back()],
        }
    }

    fn busy(&self) -> Option<(&'static str, &'static str)> {
        match self.page {
            Page::Checking => Some((" reading anki ", "checking that anki is closed")),
            Page::Loading => Some((" reading anki ", "finding your decks")),
            Page::Reading => Some((" reading anki ", "counting cards and reviews")),
            Page::Saving(_) => Some((" saving ", "writing and checking your file")),
            Page::Cancelling(_) => Some((" cancelling ", "removing the unfinished file")),
            _ => None,
        }
    }
}

impl Refusal {
    fn retry(&self) -> bool {
        matches!(
            self,
            Self::Open | Self::Changed | Self::NoProfiles { .. } | Self::Other(_)
        )
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let (reason, actions) = match self {
            Self::Open => (
                String::from("anki is open or has crashed"),
                vec![
                    String::from("1. sync in anki"),
                    String::from("2. close anki"),
                ],
            ),
            Self::Changed => (
                String::from("the collection changed during the export"),
                vec![String::from("keep anki closed while exporting")],
            ),
            Self::NoProfiles { roots } => (
                String::from("no anki profiles found"),
                vec![
                    format!("looked in {roots}"),
                    String::from("choose your collection file with --collection"),
                ],
            ),
            Self::NoProfile { name, available } => (
                format!("no anki profile named {name}"),
                vec![format!("profiles found: {available}")],
            ),
            Self::Deck(reason) => (
                reason.clone(),
                vec![String::from("use the full deck name with --deck")],
            ),
            Self::Collection(reason) => (reason.clone(), Vec::new()),
            Self::Destination => (
                String::from("can't save inside the anki folder"),
                vec![String::from("choose another folder with --out")],
            ),
            Self::Oversized { decks: 1 } => {
                (String::from("this deck is too big to export"), Vec::new())
            }
            Self::Oversized { .. } => (
                String::from("too much to export at once"),
                vec![String::from("choose fewer decks")],
            ),
            Self::Schema => (
                String::from("kamishibai can't read this anki version yet"),
                vec![String::from("update kamishibai")],
            ),
            Self::Other(reason) => (String::from("could not export"), vec![reason.clone()]),
        };
        let mut lines = wrapped(&reason, width, Ink::Subject);
        if !actions.is_empty() {
            lines.push(Line::default());
            for action in actions {
                lines.extend(wrapped(&action, width, Ink::Detail));
            }
        }
        lines
    }
}

fn gutter(width: u16) -> u16 {
    if width >= 60 { 4 } else { 1 }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::anki::history::{self, AnkiProcess};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::io;
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
            String::from("/Users/learner/Downloads"),
            Viewport::new(0, Instant::now()),
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
            Page::Stopped(Stop {
                refusal: Refusal::Open,
                step: 1,
            }),
            Page::Profile,
            Page::Loading,
            Page::Decks,
            Page::Reading,
            Page::Summary(preview()),
            Page::Saving(preview()),
            Page::Cancelling(preview()),
            Page::Saved(Outcome {
                path: PathBuf::from("/Users/learner/Downloads/kamishibai-anki-2026-11-07.zip"),
                overview: preview(),
            }),
            Page::Stopped(Stop {
                refusal: Refusal::Destination,
                step: 3,
            }),
        ]
    }

    #[test]
    fn no_page_shows_retired_words() {
        let retired = [
            "Anki stays unchanged",
            "not saved yet",
            "words",
            "notes",
            "mistakes",
            "references",
            "bytes",
            "KiB",
            "MiB",
            "GiB",
            "about",
            "from French",
            "WAL",
            "snapshot",
            "ZIP",
            "left out",
            "01 ",
            "02 ",
            "03 ",
            "›",
            "[Enter / Esc]",
        ];
        assert!(
            [42, 120]
                .into_iter()
                .all(|width| pages().into_iter().all(|page| {
                    let rendered = render(&mut screen(page), width, 36);
                    retired.iter().all(|word| !rendered.contains(word))
                })),
            "an export page retained discarded copy or step headings"
        );
    }

    #[test]
    fn steps_cannot_leave_the_footer() {
        let rendered = render(&mut screen(Page::Decks), 100, 30);
        assert!(
            rendered.lines().last().unwrap().contains("step 2/3")
                && !rendered.contains("02 choose"),
            "the deck step left the footer"
        );
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
    fn the_saved_page_keeps_its_reviewed_rendering() {
        insta::assert_snapshot!(
            "export_saved",
            render(&mut screen(pages().remove(9)), 100, 30)
        );
    }

    #[test]
    fn the_import_link_cannot_capture_the_prefix_or_surrounding_cells() {
        let screen = screen(pages().remove(9));
        assert!(
            [100, 60, 42].into_iter().all(|width| {
                let start = gutter(width) + 10;
                screen.link(start, 9, width)
                    == Some((PathBuf::from("https://app.kamishibai.io/import"), false))
                    && (0..17).all(|offset| {
                        screen.link(start + offset, 9, width)
                            == Some((PathBuf::from("https://app.kamishibai.io/import"), false))
                    })
                    && screen.link(start - 1, 9, width).is_none()
                    && screen.link(start + 17, 9, width).is_none()
                    && screen.link(start, 8, width).is_none()
                    && screen.link(start, 10, width).is_none()
            }),
            "the import URL was missing or captured cells outside its underlined domain"
        );
    }

    #[test]
    fn the_import_line_cannot_lose_its_detail_ink_or_underlined_subject() {
        let mut screen = screen(pages().remove(9));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| screen.draw(frame, 0)).unwrap();
        let buffer = terminal.backend().buffer();
        assert!(
            buffer[(4, 9)].fg == Ink::Detail.on(false).fg.unwrap()
                && !buffer[(4, 9)].modifier.contains(Modifier::UNDERLINED)
                && buffer[(14, 9)].fg == Ink::Subject.on(false).fg.unwrap()
                && buffer[(14, 9)].modifier.contains(Modifier::UNDERLINED),
            "the import prefix lost its detail rank or the domain lost the file-link style"
        );
    }

    #[test]
    fn an_output_refusal_cannot_exit_instead_of_returning_to_chosen_decks() {
        assert!(
            [KeyCode::Esc, KeyCode::Left].into_iter().all(|key| {
                let mut screen = screen(Page::Decks);
                screen.key(KeyCode::Char(' '));
                screen.key(KeyCode::Down);
                screen.key(KeyCode::Char(' '));
                let selection = screen.select();
                screen.page = Page::Reading;
                screen.failed(Refusal::Destination);
                screen.key(key) == Intent::Wait
                    && matches!(screen.page, Page::Decks)
                    && screen.select() == selection
            }),
            "an output-folder refusal exited or discarded the chosen decks"
        );
    }

    #[test]
    fn checking_again_after_a_write_failure_cannot_reload_the_default_decks() {
        let mut screen = screen(Page::Decks);
        screen.key(KeyCode::Char(' '));
        screen.key(KeyCode::Down);
        screen.key(KeyCode::Char(' '));
        let selection = screen.select();
        screen.page = Page::Saving(preview());
        screen.failed(Refusal::Other(String::from("output path is a file")));
        let intent = screen.key(KeyCode::Enter);
        assert!(
            intent == Intent::Review
                && matches!(screen.page, Page::Reading)
                && screen.select() == selection
                && selection.1 == [String::from("French::Daily")],
            "checking again reloaded the default decks instead of reviewing the chosen deck"
        );
    }

    #[test]
    fn a_narrow_deck_page_cannot_hide_selection_or_back_keys() {
        let rendered = render(&mut screen(Page::Decks), 42, 16);
        assert!(
            rendered.contains("[Space] select") && rendered.contains("[Esc] back"),
            "a nonempty deck selection hid Space or Escape at 42 by 16"
        );
    }

    #[test]
    fn an_empty_deck_selection_cannot_lose_its_reviewed_copy_or_keys() {
        let mut screen = screen(Page::Decks);
        screen.key(KeyCode::Char(' '));
        let rendered = render(&mut screen, 42, 16);
        assert!(
            rendered.contains("no decks chosen")
                && rendered.contains("[Space] select")
                && rendered.contains("[Esc] back"),
            "an empty deck selection lost its reviewed copy, Space or Escape"
        );
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
            rendered.contains("[Enter] save")
                && rendered.contains("/Users/learner/Downloads")
                && !rendered.contains("not saved yet")
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
            rendered.contains("[Space] select") && !rendered.contains("[Enter]")
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
        let mut screen = screen(Page::Saving(preview()));
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
            rendered.contains("folder"),
            "scrolling moved beyond the last summary line"
        );
    }
    #[test]
    fn a_subdeck_sits_indented_under_its_parent() {
        let mut screen = screen(Page::Decks);
        let mut decks = decks();
        decks.reverse();
        screen.inspected(decks);
        let rendered = render(&mut screen, 100, 30);
        let rows = rendered.lines().skip(3).take(3).collect::<Vec<_>>();
        assert!(
            rows[0].contains("French")
                && rows[1].contains("    Daily")
                && rows[2].contains("Other")
                && !rendered.contains("French::Daily"),
            "a child deck lost its position or tree indentation"
        );
    }

    #[test]
    fn a_deck_without_a_listed_parent_shows_its_path() {
        let mut screen = screen(Page::Decks);
        let child = screen.choices.decks.remove(1);
        screen.inspected(vec![child]);
        assert!(
            render(&mut screen, 100, 30).contains("French / Daily"),
            "an unlisted parent disappeared from the deck path"
        );
    }

    #[test]
    fn the_status_counts_the_cards_of_chosen_decks() {
        let mut screen = screen(Page::Decks);
        let selected = render(&mut screen, 100, 30);
        screen.key(KeyCode::Char(' '));
        let empty = render(&mut screen, 100, 30);
        assert!(
            selected
                .lines()
                .last()
                .unwrap()
                .contains("step 2/3  1 card")
                && empty
                    .lines()
                    .last()
                    .unwrap()
                    .contains("step 2/3  no decks chosen"),
            "selection did not update the footer's own-card census"
        );
    }

    #[test]
    fn an_oversized_refusal_returns_to_the_chosen_decks() {
        let mut screen = screen(Page::Reading);
        let names = screen.select().1;
        screen.failed(Refusal::Oversized { decks: 1 });
        let intent = screen.key(KeyCode::Enter);
        assert!(
            intent == Intent::Wait
                && matches!(screen.page, Page::Decks)
                && screen.select().1 == names,
            "an oversized refusal restarted discovery or discarded the selected decks"
        );
    }

    #[test]
    fn refusals_decided_by_flags_offer_no_retry() {
        let refusals = [
            Refusal::Destination,
            Refusal::NoProfile {
                name: String::from("Missing"),
                available: String::from("Learner"),
            },
            Refusal::Deck(String::from("Anki deck not found: Missing")),
            Refusal::Schema,
        ];
        assert!(
            refusals.into_iter().all(|refusal| {
                let mut screen = screen(Page::Reading);
                screen.failed(refusal);
                !render(&mut screen, 100, 30).contains("[Enter]")
                    && screen.key(KeyCode::Enter) == Intent::Wait
            }),
            "a flag-defined refusal offered a retry that cannot help"
        );
    }

    #[test]
    fn the_saved_page_names_the_folder_and_the_file() {
        let mut screen = screen(Page::Saving(preview()));
        screen.saved(PathBuf::from("/exports/history.zip"));
        let rendered = render(&mut screen, 100, 30);
        assert!(
            rendered.contains("FOLDER  /exports")
                && rendered.contains("FILE    history.zip")
                && rendered.contains("✓ saved")
                && rendered
                    .lines()
                    .any(|line| line.trim() == "import at app.kamishibai.io")
                && rendered.lines().nth(8).unwrap().trim().is_empty(),
            "the verified outcome lost its artifact strip, receipt or web destination"
        );
    }

    #[test]
    fn a_stop_keeps_the_step_it_happened_on() {
        let mut screen = screen(Page::Saving(preview()));
        screen.failed(Refusal::Open);
        assert!(
            render(&mut screen, 100, 30)
                .lines()
                .last()
                .unwrap()
                .contains("step 3/3"),
            "a publication refusal returned to the profile step"
        );
    }

    #[test]
    fn an_unsafe_folder_cannot_refuse_at_the_deck_step() {
        let mut screen = screen(Page::Reading);
        screen.failed(Refusal::Destination);
        assert!(
            render(&mut screen, 60, 24)
                .lines()
                .last()
                .unwrap()
                .contains("step 3/3"),
            "the output-folder refusal was attributed to choosing decks"
        );
    }

    #[test]
    fn busy_pages_keep_the_previous_page_under_the_loader() {
        let rendered = render(&mut screen(Page::Reading), 100, 30);
        assert!(
            rendered.contains("French")
                && rendered.contains("counting cards and reviews")
                && rendered.contains("reading anki"),
            "the reading overlay erased the chosen decks or used a separate loader"
        );
    }

    #[test]
    fn unselected_learning_history_cannot_disappear_from_the_deck_list() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("collection.anki2");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE col(ver INTEGER,crt INTEGER,mod INTEGER); INSERT INTO col VALUES(18,17,29); CREATE TABLE decks(id INTEGER,name TEXT); INSERT INTO decks VALUES(31,'Old lessons'); CREATE TABLE notetypes(id INTEGER,name TEXT); INSERT INTO notetypes VALUES(13,'Basic'); CREATE TABLE fields(ntid INTEGER,ord INTEGER,name TEXT); CREATE TABLE notes(id INTEGER,guid TEXT,mid INTEGER,mod INTEGER,tags TEXT,flds TEXT); INSERT INTO notes VALUES(23,'g',13,3,'','chat'); CREATE TABLE cards(id INTEGER,nid INTEGER,did INTEGER,ord INTEGER,type INTEGER,queue INTEGER,due INTEGER,ivl INTEGER,factor INTEGER,reps INTEGER,lapses INTEGER,odid INTEGER,odue INTEGER); INSERT INTO cards VALUES(37,23,31,0,2,2,41,11,2500,5,1,0,0); CREATE TABLE revlog(id INTEGER,cid INTEGER,ease INTEGER,ivl INTEGER,lastIvl INTEGER,factor INTEGER,time INTEGER,type INTEGER); INSERT INTO revlog VALUES(101,37,1,5,3,2500,671,1),(102,37,3,7,5,2500,431,1);").unwrap();
        drop(db);
        let decks = history::inspect(&path, &[], &Closed, "unknown").unwrap();
        let rows = serde_json::to_value(&decks).unwrap();
        let mut screen = screen(Page::Decks);
        screen.inspected(decks);
        let rendered = render(&mut screen, 100, 30);
        assert!(
            rows[0]["selected"] == false
                && rows[0]["cards"] == 1
                && rows[0]["reviews"] == 2
                && rendered
                    .lines()
                    .any(|line| line.contains("Old lessons") && line.contains("1 card")),
            "unselected learning history lost its deck, card count or review count"
        );
    }

    #[test]
    fn file_links_cannot_extend_beyond_the_underlined_labels() {
        let mut screen = screen(Page::Saving(preview()));
        screen.saved(PathBuf::from("/exports/history.zip"));
        assert!(
            [100, 60, 42].into_iter().all(|width| {
                let start = gutter(width) + 4;
                screen.link(start, 3, width) == Some((PathBuf::from("/exports"), false))
                    && screen.link(start + 5, 3, width).is_some()
                    && screen.link(start + 6, 3, width).is_none()
                    && screen.link(start + 3, 4, width)
                        == Some((PathBuf::from("/exports/history.zip"), true))
                    && screen.link(start + 4, 4, width).is_none()
                    && screen.link(start, 5, width).is_none()
            }),
            "a file label resolved the wrong path or captured surrounding cells"
        );
    }

    #[test]
    fn a_short_saved_page_cannot_lose_the_last_web_instruction() {
        let mut screen = screen(Page::Saving(preview()));
        screen.saved(PathBuf::from("/exports/history.zip"));
        for _ in 0..20 {
            screen.key(KeyCode::Down);
        }
        assert!(
            render(&mut screen, 42, 16).contains("import at app.kamishibai.io"),
            "a short saved page could not scroll to the final web instruction"
        );
    }
}
