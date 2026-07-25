//! Visual demo that renders every TUI state the design mockup covers.
//!
//! `cargo run --example tui_states` walks the design states. Navigation is
//! absolute, not cumulative: type a state index then `Space` to jump straight
//! to it (e.g. `5` then `Space` shows state 5). A bare `Space` with no digits
//! queued steps forward one, `←`/`p` steps back, and `q` exits. `Enter` only
//! clears the queued digits, so the stray Return the shell injects when it
//! launches the binary can neither drift nor contaminate the index. Absolute
//! jumps keep VHS screenshots reproducible — a dropped or coalesced keystroke
//! cannot accumulate across the run. Intended for VHS/tmux screenshots so the
//! live render can be compared against `Kamishibai TUI A.html`.

use std::io::{self, stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::MouseEventKind;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, AttemptFault, CardArtifacts, CardDraft, CardMeta,
    LanguagePair, WordCandidate,
};
use kamishibai::tui::{
    App, BusyKind, KeySource, ModalKind, MousePointer, Screen, draw, mouse_pointer_at,
    reset_mouse_pointer, write_mouse_pointer,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

const POINTER_REFRESH: Duration = Duration::from_millis(50);

fn main() -> Result<()> {
    let states = build_states();
    enable_raw_mode()?;
    let mut out = stdout();
    out.execute(EnterAlternateScreen)?;
    enable_hover_mouse_capture(&mut out);
    write_mouse_pointer(&mut out, MousePointer::Arrow);
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;
    let result = run(&mut terminal, &states);
    reset_mouse_pointer(terminal.backend_mut());
    disable_hover_mouse_capture(terminal.backend_mut());
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(result?)
}

fn enable_hover_mouse_capture<W: io::Write>(out: &mut W) {
    let _ = out.write_all(b"\x1b[?1006h\x1b[?1003h");
    let _ = out.flush();
}

fn disable_hover_mouse_capture<W: io::Write>(out: &mut W) {
    let _ = out.write_all(b"\x1b[?1003l\x1b[?1006l");
    let _ = out.flush();
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    states: &[(String, App)],
) -> io::Result<()> {
    let mut index = 0usize;
    let mut pending = String::new();
    let mut mouse_position: Option<(u16, u16)> = None;
    let mut force_redraw = true;
    loop {
        if force_redraw {
            terminal.clear()?;
            force_redraw = false;
        }
        let (label, app) = &states[index];
        terminal.draw(|frame| {
            draw(frame, app);
        })?;
        let size = terminal.size()?;
        let rect = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        if let Some((column, row)) = mouse_position {
            let next = mouse_pointer_at(app, rect, column, row);
            write_mouse_pointer(terminal.backend_mut(), next);
        }
        let _ = label;
        let timeout = if mouse_position.is_some() {
            POINTER_REFRESH
        } else {
            Duration::from_secs(60)
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char(c) if c.is_ascii_digit() => pending.push(c),
                    KeyCode::Char(' ') => {
                        if let Ok(target) = pending.parse::<usize>() {
                            if target < states.len() {
                                index = target;
                            }
                        } else {
                            index = (index + 1) % states.len();
                        }
                        pending.clear();
                        force_redraw = true;
                    }
                    KeyCode::Enter => pending.clear(),
                    KeyCode::Left | KeyCode::Char('p') => {
                        pending.clear();
                        if index == 0 {
                            index = states.len() - 1;
                        } else {
                            index -= 1;
                        }
                        force_redraw = true;
                    }
                    _ => {
                        pending.clear();
                        index = (index + 1) % states.len();
                        force_redraw = true;
                    }
                },
                Event::Mouse(mouse)
                    if matches!(
                        mouse.kind,
                        MouseEventKind::Moved
                            | MouseEventKind::Drag(_)
                            | MouseEventKind::Down(_)
                            | MouseEventKind::ScrollUp
                            | MouseEventKind::ScrollDown
                    ) =>
                {
                    mouse_position = Some((mouse.column, mouse.row));
                    let size = terminal.size()?;
                    let rect = Rect {
                        x: 0,
                        y: 0,
                        width: size.width,
                        height: size.height,
                    };
                    let next = mouse_pointer_at(app, rect, mouse.column, mouse.row);
                    write_mouse_pointer(terminal.backend_mut(), next);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn pair() -> LanguagePair {
    LanguagePair::new("fr", "en")
}

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn cached_artifacts() -> CardArtifacts {
    let tmp = std::env::temp_dir();
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(ArtifactFile::new(
            "a345532c.json",
            tmp.join("a345532c.json"),
            "1.9 KB",
            true,
        )),
        ArtifactSlot::fresh(Artifact::Picture).succeeded_with(ArtifactFile::new(
            "a345532c.jpg",
            tmp.join("a345532c.jpg"),
            "268 KB",
            true,
        )),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(ArtifactFile::new(
            "f4206ebe.wav",
            tmp.join("f4206ebe.wav"),
            "11.2 KB",
            true,
        )),
    )
}

fn rejected_frame(sequence: usize, category: &str, reason: &str) -> AttemptFault {
    AttemptFault::new(
        category,
        reason,
        Some(std::env::temp_dir().join(format!("attempt-{sequence:04}.jpg"))),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture)
            .faulted(rejected_frame(
                1,
                "border",
                "White border missing on: bottom",
            ))
            .faulted(rejected_frame(
                2,
                "topology",
                "Registered panel topology was not detected",
            )),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn second_retrying_artifacts() -> CardArtifacts {
    let sound = ArtifactSlot::fresh(Artifact::Sound)
        .faulted(AttemptFault::failed("the voice response carried no audio"))
        .faulted(AttemptFault::failed("the voice response carried no audio"));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        sound,
    )
}

fn making_picture_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).faulted(rejected_frame(
            1,
            "border",
            "White border missing on: top, left",
        )),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn failed_picture_artifacts() -> CardArtifacts {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for (sequence, category, reason) in [
        (1, "border", "White border missing on: bottom"),
        (2, "topology", "Registered panel topology was not detected"),
        (
            3,
            "recall_text",
            "Recall judge rejected image: visible answer",
        ),
    ] {
        picture = picture.faulted(rejected_frame(sequence, category, reason));
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn meta_for(term: &str, target: &str, source: &str, hint: &str, highlight: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{target}/"),
        format!("translation of {term}"),
        7,
        source.to_string(),
        if highlight.is_empty() {
            term
        } else {
            highlight
        }
        .to_string(),
        hint.to_string(),
        format!("usage notes for {term}"),
        target.to_string(),
    )
}

fn card(term: &str, front: &str, back: &str, artifacts: CardArtifacts) -> CardDraft {
    let meta = meta_for(term, front, back, "", "");
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn card_with_hint(
    term: &str,
    front: &str,
    back: &str,
    hint: &str,
    artifacts: CardArtifacts,
) -> CardDraft {
    let meta = meta_for(term, front, back, hint, "");
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn card_with_highlight(
    term: &str,
    front: &str,
    back: &str,
    hint: &str,
    highlight: &str,
    artifacts: CardArtifacts,
) -> CardDraft {
    let meta = meta_for(term, front, back, hint, highlight);
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn build_states() -> Vec<(String, App)> {
    let words_seed = "dépaysement\nflâner\ncanard\nchouette";
    let base_words = App::new(pair()).seeded_blob(words_seed);

    let candidates = vec![
        WordCandidate::new(
            "dépaysement",
            "noun; the unsettled, refreshing feeling of being somewhere unfamiliar",
            true,
        ),
        WordCandidate::new(
            "flâner",
            "verb; to stroll without aim, savouring the wander itself",
            true,
        ),
        WordCandidate::new(
            "canard",
            "noun; a duck — and, informally, a planted newspaper hoax",
            true,
        ),
        WordCandidate::new(
            "chouette",
            "noun an owl; colloquially an adjective meaning neat or lovely",
            true,
        ),
    ];

    let review = App::new(pair())
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("fr")
        .understood(candidates.clone());

    let change_something = review
        .clone()
        .with_modal(ModalKind::ChangeSomething)
        .typed('#')
        .typed('3')
        .typed(' ')
        .typed('—')
        .typed(' ')
        .typed('n')
        .typed('o')
        .typed('u')
        .typed('n');

    let base_cards = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            card_with_highlight(
                "dépaysement",
                "Ce dépaysement l'a réveillée d'un coup.",
                "This change of scenery woke her up at once.",
                "About the jolt of feeling far from the familiar.",
                "dépaysement",
                cached_artifacts(),
            ),
            card(
                "flâner",
                "Nous aimons flâner le long de la Seine le dimanche.",
                "",
                ready_artifacts(),
            ),
            card("canard", "", "", making_picture_artifacts()),
            card("chouette", "", "", CardArtifacts::default()),
        ])
        .card_toggle_expanded()
        .with_elapsed(Duration::from_secs(41));

    let change_this_card = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            card("dépaysement", "", "", ready_artifacts()),
            card_with_hint(
                "flâner",
                "Nous aimons flâner le long de la Seine le dimanche.",
                "",
                "About wandering slowly with no destination in mind.",
                ready_artifacts(),
            ),
            card("canard", "", "", making_picture_artifacts()),
            card("chouette", "", "", CardArtifacts::default()),
        ])
        .card_selected_next()
        .with_modal(ModalKind::ChangeThisCard)
        .typed('m')
        .typed('a')
        .typed('k')
        .typed('e')
        .typed(' ')
        .typed('i')
        .typed('t')
        .typed(' ')
        .typed('s')
        .typed('i')
        .typed('m')
        .typed('p')
        .typed('l')
        .typed('e');

    let retrying = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            card("dépaysement", "", "", ready_artifacts()),
            card("flâner", "", "", second_retrying_artifacts()),
            card("canard", "", "", retrying_artifacts()),
            card("chouette", "", "", making_picture_artifacts()),
        ])
        .with_elapsed(Duration::from_secs(65));

    let failed = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            card("dépaysement", "", "", ready_artifacts()),
            card("flâner", "", "", ready_artifacts()),
            card("canard", "", "", failed_picture_artifacts()),
            card("chouette", "", "", ready_artifacts()),
        ])
        .with_elapsed(Duration::from_secs(108));

    let done = App::new(pair())
        .with_screen(Screen::Done)
        .confirmed_learning("fr")
        .done_published(
            "fr_2026-06-01_183029.apkg",
            "fr_2026-06-01_183029.pdf",
            "kamishibai-out/",
        );

    let welcome_no_env = App::new(pair())
        .opening_welcome(KeySource::Empty, String::new(), false)
        .welcome_advance();

    let welcome_env = App::new(pair())
        .opening_welcome(KeySource::Empty, String::new(), true)
        .welcome_advance()
        .welcome_focus_next();

    let busy_understanding = App::new(pair())
        .seeded_blob(words_seed)
        .busy_started(BusyKind::Understanding)
        .busy_elapsed(Duration::from_millis(540));
    vec![
        (String::from("00 · Welcome · no env key"), welcome_no_env),
        (
            String::from("00b · Welcome · env key available"),
            welcome_env,
        ),
        (String::from("01 · Your words"), base_words),
        (
            String::from("01b · Busy · understanding"),
            busy_understanding,
        ),
        (String::from("02 · What I understood"), review),
        (
            String::from("02b · Change something · modal"),
            change_something,
        ),
        (String::from("03 · Your cards"), base_cards),
        (
            String::from("03b · Change this card · modal"),
            change_this_card,
        ),
        (String::from("03c · Your cards · retrying"), retrying),
        (String::from("03d · Your cards · couldn't finish"), failed),
        (String::from("04 · Done"), done),
    ]
}
