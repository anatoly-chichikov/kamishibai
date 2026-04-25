//! Visual demo that renders every TUI state the design mockup covers.
//!
//! `cargo run --example tui_states` advances through the seven design states
//! on every keypress and exits on `q`. Intended for VHS/tmux screenshots so
//! the live render can be compared against `Kamishibai TUI A.html`.

use std::io::{self, stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, CandidateKind, CardArtifacts, CardDraft, CardPayload,
    LanguagePair, WordCandidate,
};
use kamishibai::tui::{App, ModalKind, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

fn main() -> Result<()> {
    let states = build_states();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = run(&mut terminal, &states);
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(result?)
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    states: &[(String, App)],
) -> io::Result<()> {
    let mut index = 0usize;
    loop {
        let (label, app) = &states[index];
        terminal.draw(|frame| {
            draw(frame, app);
        })?;
        let _ = label;
        if event::poll(Duration::from_secs(60))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
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
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn cached_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(ArtifactFile::new(
            "a345532c.json",
            "1.9 KB",
            true,
        )),
        ArtifactSlot::fresh(Artifact::Picture).succeeded_with(ArtifactFile::new(
            "a345532c.jpg",
            "268 KB",
            true,
        )),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(ArtifactFile::new(
            "f4206ebe.wav",
            "11.2 KB",
            true,
        )),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture)
            .attempted()
            .attempted(),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn second_retrying_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).attempted().attempted(),
    )
}

fn making_picture_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
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
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn card(term: &str, front: &str, back: &str, artifacts: CardArtifacts) -> CardDraft {
    card_with_hint(term, front, back, "", artifacts)
}

fn card_with_hint(
    term: &str,
    front: &str,
    back: &str,
    hint: &str,
    artifacts: CardArtifacts,
) -> CardDraft {
    CardDraft::new(term, pair(), CardPayload::new(front, back, hint, term))
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
    CardDraft::new(term, pair(), CardPayload::new(front, back, hint, highlight))
        .with_artifacts(artifacts)
}

fn build_states() -> Vec<(String, App)> {
    let words_seed = "whilst\nat the end\nin the end\nwreck";
    let base_words = App::new(pair()).seeded_blob(words_seed);

    let candidates = vec![
        WordCandidate::new(
            "whilst",
            CandidateKind::Word,
            "«пока, в то время как» · BrE",
            "",
        ),
        WordCandidate::new(
            "at the end",
            CandidateKind::Phrase,
            "«в конце» — о времени/месте",
            "",
        ),
        WordCandidate::new(
            "in the end",
            CandidateKind::Idiom,
            "«в итоге» — о результате",
            "",
        ),
        WordCandidate::new("wreck", CandidateKind::Word, "обломки · разрушать", ""),
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
                "whilst",
                "Пока она говорила, я в то время как думал о своём.",
                "While she was speaking, I was thinking of my own stuff.\nwhilst /waɪlst/   пока, в то время как · formal",
                "Как «while», но старомодное — в книгах и BBC.",
                "в то время как",
                cached_artifacts(),
            ),
            card(
                "at the end",
                "The meeting starts at the end of March.",
                "",
                ready_artifacts(),
            ),
            card("in the end", "", "", making_picture_artifacts()),
            card("wreck", "", "", CardArtifacts::default()),
        ])
        .card_toggle_expanded()
        .with_elapsed(Duration::from_secs(41));

    let change_this_card = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card("whilst", "", "", ready_artifacts()),
            card(
                "at the end",
                "The meeting starts at the end of March.",
                "",
                ready_artifacts(),
            ),
            card("in the end", "", "", making_picture_artifacts()),
            card("wreck", "", "", CardArtifacts::default()),
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
            card("whilst", "", "", ready_artifacts()),
            card("at the end", "", "", second_retrying_artifacts()),
            card("in the end", "", "", retrying_artifacts()),
            card("wreck", "", "", making_picture_artifacts()),
        ])
        .with_elapsed(Duration::from_secs(65));

    let failed = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_target("en")
        .cards_started(vec![
            card("whilst", "", "", ready_artifacts()),
            card("at the end", "", "", ready_artifacts()),
            card("in the end", "", "", failed_picture_artifacts()),
            card("wreck", "", "", ready_artifacts()),
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

    vec![
        (String::from("01 · Your words"), base_words),
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
