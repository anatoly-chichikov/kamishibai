//! Visual demo that renders every TUI state the design mockup covers.
//!
//! `cargo run --example tui_states` advances through the seven design states
//! on every keypress and exits on `q`. Intended for VHS/tmux screenshots so
//! the live render can be compared against `Kamishibai TUI A.html`.

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
    Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardBody, CardDraft, LanguagePair,
    WordCandidate,
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
    let mut mouse_position: Option<(u16, u16)> = None;
    loop {
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
                    KeyCode::Left | KeyCode::Char('p') => {
                        if index == 0 {
                            index = states.len() - 1;
                        } else {
                            index -= 1;
                        }
                    }
                    _ => {
                        index = (index + 1) % states.len();
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
    LanguagePair::new("en", "ru")
}

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn cached_artifacts() -> CardArtifacts {
    let tmp = std::env::temp_dir();
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
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

fn retrying_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture)
            .attempted()
            .attempted(),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn second_retrying_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).attempted().attempted(),
    )
}

fn making_picture_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).attempted(),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn failed_picture_artifacts() -> CardArtifacts {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for _ in 0..3 {
        picture = picture.attempted();
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Body).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn body_for(term: &str, target: &str, source: &str, hint: &str, highlight: &str) -> CardBody {
    CardBody::new(
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
    let body = body_for(term, front, back, "", "");
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_body(body, None)
        .with_artifacts(artifacts)
}

fn card_with_hint(
    term: &str,
    front: &str,
    back: &str,
    hint: &str,
    artifacts: CardArtifacts,
) -> CardDraft {
    let body = body_for(term, front, back, hint, "");
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_body(body, None)
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
    let body = body_for(term, front, back, hint, highlight);
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_body(body, None)
        .with_artifacts(artifacts)
}

fn build_states() -> Vec<(String, App)> {
    let words_seed = "sincerely\nat the end\nexpel\ndebuted";
    let base_words = App::new(pair()).seeded_blob(words_seed);

    let candidates = vec![
        WordCandidate::new(
            "sincerely",
            "наречие; искренне; формальный стиль, часто в письмах",
            true,
        ),
        WordCandidate::new("at the end", "фраза о времени или месте; в конце", true),
        WordCandidate::new(
            "expel",
            "глагол; смысл — исключить из учебного заведения или организации",
            true,
        ),
        WordCandidate::new(
            "debuted",
            "прошедшее время от слова «debut»; о первом публичном появлении",
            true,
        ),
    ];

    let review = App::new(pair())
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_target("en")
        .understood(candidates.clone());

    let change_something = review
        .clone()
        .with_modal(ModalKind::ChangeSomething)
        .typed('#')
        .typed('4')
        .typed(' ')
        .typed('—')
        .typed(' ')
        .typed('г')
        .typed('л')
        .typed('а')
        .typed('г')
        .typed('о')
        .typed('л');

    let base_cards = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card_with_highlight(
                "sincerely",
                "She sincerely thanked everyone for their help.",
                "Она искренне поблагодарила всех за помощь.",
                "О чувствах, выраженных честно и серьёзно.",
                "sincerely",
                cached_artifacts(),
            ),
            card(
                "at the end",
                "The meeting starts at the end of March.",
                "",
                ready_artifacts(),
            ),
            card("expel", "", "", making_picture_artifacts()),
            card("debuted", "", "", CardArtifacts::default()),
        ])
        .card_toggle_expanded()
        .with_elapsed(Duration::from_secs(41));

    let change_this_card = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card("sincerely", "", "", ready_artifacts()),
            card_with_hint(
                "at the end",
                "The meeting starts at the end of March.",
                "",
                "О временной точке завершения процесса.",
                ready_artifacts(),
            ),
            card("expel", "", "", making_picture_artifacts()),
            card("debuted", "", "", CardArtifacts::default()),
        ])
        .card_selected_next()
        .with_modal(ModalKind::ChangeThisCard)
        .typed('п')
        .typed('р')
        .typed('и')
        .typed('м')
        .typed('е')
        .typed('р')
        .typed(' ')
        .typed('п')
        .typed('о')
        .typed('п')
        .typed('р')
        .typed('о')
        .typed('щ')
        .typed('е');

    let retrying = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card("sincerely", "", "", ready_artifacts()),
            card("at the end", "", "", second_retrying_artifacts()),
            card("expel", "", "", retrying_artifacts()),
            card("debuted", "", "", making_picture_artifacts()),
        ])
        .with_elapsed(Duration::from_secs(65));

    let failed = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card("sincerely", "", "", ready_artifacts()),
            card("at the end", "", "", ready_artifacts()),
            card("expel", "", "", failed_picture_artifacts()),
            card("debuted", "", "", ready_artifacts()),
        ])
        .with_elapsed(Duration::from_secs(108));

    let done = App::new(pair())
        .with_screen(Screen::Done)
        .confirmed_target("en")
        .done_published(
            "en_2026-04-17_183029.apkg",
            "en_2026-04-17_183029.pdf",
            "kamishibai-out/",
        );

    let welcome = App::new(pair())
        .opening_welcome(KeySource::Empty, String::new())
        .welcome_advance();

    let busy_understanding = App::new(pair())
        .seeded_blob(words_seed)
        .busy_started(BusyKind::Understanding)
        .busy_elapsed(Duration::from_millis(540));
    vec![
        (String::from("00 · Welcome"), welcome),
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
