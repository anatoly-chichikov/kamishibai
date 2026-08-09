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
    Artifact, ArtifactCosts, ArtifactFile, ArtifactSlot, AttemptFault, AxisSet, CardArtifacts,
    CardDraft, CardMeta, GenerationCost, LanguagePair, Register, SentenceAxis, SentenceKind,
    SentenceLabels, SentenceLevel, WordCandidate,
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
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(ArtifactFile::new(
            "a345532c.json",
            tmp.join("a345532c.json"),
            "1 B",
            true,
        )),
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

fn recovered_cached_artifacts() -> CardArtifacts {
    let tmp = std::env::temp_dir();
    let picture = ArtifactSlot::fresh(Artifact::Picture)
        .faulted(rejected_frame(
            1,
            "border",
            "White border missing on: bottom",
        ))
        .faulted(rejected_frame(
            2,
            "topology",
            "Registered panel topology was not detected",
        ))
        .succeeded_with(ArtifactFile::new(
            "a345532c.jpg",
            tmp.join("a345532c.jpg"),
            "268 KB",
            true,
        ));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(ArtifactFile::new(
            "a345532c.json",
            tmp.join("a345532c.json"),
            "1 B",
            true,
        )),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(ArtifactFile::new(
            "a345532c.json",
            tmp.join("a345532c.json"),
            "1.9 KB",
            true,
        )),
        picture,
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
        (4, "layout", "Safe text region left no room for the target"),
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

fn retry_stress_cost(kind: Artifact) -> ArtifactCosts {
    let nanos = match kind {
        Artifact::Meta => 15_400_000,
        Artifact::Sound => 2_100_000,
        Artifact::Scene => 23_000_000,
        Artifact::Picture => 173_800_000,
    };
    ArtifactCosts::default().charged(kind, GenerationCost::from_nanos(nanos))
}

fn retry_stress_file(kind: Artifact) -> ArtifactFile {
    let (name, size, nanos) = match kind {
        Artifact::Meta => ("meta.json", "1.8 KB", 15_400_000),
        Artifact::Sound => ("audio.wav", "162.4 KB", 2_100_000),
        Artifact::Scene => ("scene.json", "25.7 KB", 23_000_000),
        Artifact::Picture => ("picture.jpg", "231.6 KB", 173_800_000),
    };
    ArtifactFile::new(name, std::env::temp_dir().join(name), size, false)
        .with_cost(GenerationCost::from_nanos(nanos))
}

fn retry_stress_fault(kind: Artifact, sequence: usize) -> AttemptFault {
    match kind {
        Artifact::Meta => AttemptFault::failed(format!("metadata response {sequence} was invalid")),
        Artifact::Sound => AttemptFault::failed(format!("audio response {sequence} was empty")),
        Artifact::Scene => AttemptFault::new(
            "schema",
            format!("scene response {sequence} did not match the schema"),
            Some(std::env::temp_dir().join(format!("scene-{sequence:04}.json"))),
        ),
        Artifact::Picture => rejected_frame(
            sequence,
            "recall_text",
            "Recall judge rejected image: visible answer",
        ),
    }
}

fn retry_stress_slot(kind: Artifact, rejected: usize) -> ArtifactSlot {
    (1..=rejected).fold(ArtifactSlot::fresh(kind), |slot, sequence| {
        slot.faulted(retry_stress_fault(kind, sequence))
    })
}

fn retry_stress_ready_slot(kind: Artifact) -> ArtifactSlot {
    retry_stress_slot(kind, 0).succeeded_with(retry_stress_file(kind))
}

fn retry_stress_artifacts(kind: Artifact, rejected: usize) -> CardArtifacts {
    match kind {
        Artifact::Meta => CardArtifacts::from_parts(
            retry_stress_slot(Artifact::Meta, rejected),
            retry_stress_slot(Artifact::Scene, 0),
            retry_stress_slot(Artifact::Picture, 0),
            retry_stress_slot(Artifact::Sound, 0),
        ),
        Artifact::Sound => CardArtifacts::from_parts(
            retry_stress_ready_slot(Artifact::Meta),
            retry_stress_slot(Artifact::Scene, 0),
            retry_stress_slot(Artifact::Picture, 0),
            retry_stress_slot(Artifact::Sound, rejected),
        ),
        Artifact::Scene => CardArtifacts::from_parts(
            retry_stress_ready_slot(Artifact::Meta),
            retry_stress_slot(Artifact::Scene, rejected),
            retry_stress_slot(Artifact::Picture, 0),
            retry_stress_ready_slot(Artifact::Sound),
        ),
        Artifact::Picture => CardArtifacts::from_parts(
            retry_stress_ready_slot(Artifact::Meta),
            retry_stress_ready_slot(Artifact::Scene),
            retry_stress_slot(Artifact::Picture, rejected),
            retry_stress_ready_slot(Artifact::Sound),
        ),
    }
}

fn retry_stress_recovered_artifacts() -> CardArtifacts {
    let picture = retry_stress_slot(Artifact::Picture, 2)
        .succeeded_with(retry_stress_file(Artifact::Picture));
    CardArtifacts::from_parts(
        retry_stress_ready_slot(Artifact::Meta),
        retry_stress_ready_slot(Artifact::Scene),
        picture,
        retry_stress_ready_slot(Artifact::Sound),
    )
}

fn labels(
    register: Register,
    level: SentenceLevel,
    kind: SentenceKind,
    pinned: &[SentenceAxis],
    approx: &[SentenceAxis],
) -> SentenceLabels {
    SentenceLabels::new(
        register,
        level,
        kind,
        AxisSet::from_axes(pinned.iter().copied()),
        AxisSet::from_axes(approx.iter().copied()),
    )
}

fn fresh_labels() -> SentenceLabels {
    labels(
        Register::Casual,
        SentenceLevel::B1,
        SentenceKind::Statement,
        &[],
        &[],
    )
}

fn legacy_meta_for(
    term: &str,
    target: &str,
    source: &str,
    hint: &str,
    highlight: &str,
) -> CardMeta {
    CardMeta::new(
        term.to_string(),
        target.to_string(),
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

fn meta_for(term: &str, target: &str, source: &str, hint: &str, highlight: &str) -> CardMeta {
    legacy_meta_for(term, target, source, hint, highlight).with_sentence_labels(fresh_labels())
}

fn card(term: &str, front: &str, back: &str, artifacts: CardArtifacts) -> CardDraft {
    let meta = meta_for(term, front, back, "", "");
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn card_with_labels(
    term: &str,
    target: &str,
    source: &str,
    sentence_labels: SentenceLabels,
    artifacts: CardArtifacts,
) -> CardDraft {
    let meta = meta_for(term, target, source, "", "").with_sentence_labels(sentence_labels);
    CardDraft::new(term, format!("understanding for {term}"), pair())
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn legacy_card(term: &str, target: &str, source: &str, artifacts: CardArtifacts) -> CardDraft {
    let meta = legacy_meta_for(term, target, source, "", "");
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

fn cards_with_first(first: CardDraft) -> App {
    App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            first,
            card(
                "flâner",
                "Nous aimons flâner le long de la Seine le dimanche.",
                "",
                ready_artifacts(),
            ),
            card("canard", "", "", making_picture_artifacts()),
            card("chouette", "", "", CardArtifacts::default()),
        ])
        .with_elapsed(Duration::from_secs(41))
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

    let cards_seed = cards_with_first(card_with_highlight(
        "dépaysement",
        "Ce dépaysement l'a réveillée d'un coup.",
        "This change of scenery woke her up at once.",
        "About the jolt of feeling far from the familiar.",
        "dépaysement",
        cached_artifacts(),
    ));
    let base_cards = cards_seed.clone();
    let label_editor = cards_seed.clone().sentence_editor_opened_for_register();
    let register_pinned = label_editor.clone().sentence_editor_axis_advanced(true);
    let editor_on_note = register_pinned
        .clone()
        .sentence_editor_row_next()
        .sentence_editor_row_next()
        .sentence_editor_row_next();
    let note_typed = "make it simpler and warmer"
        .chars()
        .fold(editor_on_note, |app, symbol| {
            app.sentence_editor_typed(symbol)
        });
    let restored = register_pinned
        .clone()
        .sentence_editor_axis_advanced(false)
        .sentence_editor_closed();
    let multiple_pending = "make the second sentence more concrete"
        .chars()
        .fold(
            register_pinned
                .clone()
                .sentence_editor_closed()
                .card_revealed(1)
                .sentence_editor_opened_for_note(),
            |app, symbol| app.sentence_editor_typed(symbol),
        )
        .sentence_editor_closed();
    let regenerating_drafts = multiple_pending
        .cards()
        .iter()
        .cloned()
        .map(CardDraft::starting_rewrite)
        .collect();
    let regenerating = multiple_pending
        .clone()
        .cards_replaced(regenerating_drafts)
        .card_revealed(0)
        .cards_running(Some((0, Artifact::Meta)))
        .with_elapsed(Duration::from_secs(43));
    let regenerated = cards_with_first(card_with_labels(
        "dépaysement",
        "Ce dépaysement lui fut particulièrement bénéfique.",
        "This change of scenery proved especially beneficial to her.",
        labels(
            Register::Formal,
            SentenceLevel::B1,
            SentenceKind::Statement,
            &[SentenceAxis::Register],
            &[],
        ),
        recovered_cached_artifacts(),
    ));
    let approximate = cards_with_first(card_with_labels(
        "dépaysement",
        "Ce dépaysement lui a vraiment fait du bien.",
        "This change of scenery really did her good.",
        labels(
            Register::Formal,
            SentenceLevel::B1,
            SentenceKind::Statement,
            &[SentenceAxis::Register],
            &[SentenceAxis::Register],
        ),
        cached_artifacts(),
    ));
    let narrow = cards_with_first(card_with_labels(
        "dépaysement",
        "Quel dépaysement littéraire, se serait-elle exclamée !",
        "What a literary change of scenery, she would reportedly have exclaimed!",
        labels(
            Register::Literary,
            SentenceLevel::B2,
            SentenceKind::Exclamation,
            &[],
            &[],
        ),
        cached_artifacts(),
    ));
    let mouse_selected = cards_seed
        .clone()
        .sentence_editor_opened_for_register()
        .sentence_editor_row_next()
        .sentence_editor_axis_chosen(2);
    let legacy = cards_with_first(legacy_card(
        "dépaysement",
        "Ce dépaysement l'a réveillée d'un coup.",
        "This change of scenery woke her up at once.",
        cached_artifacts(),
    ))
    .sentence_editor_opened_for_register();

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

    let retry_stress = App::new(pair())
        .with_screen(Screen::YourCards)
        .confirmed_learning("fr")
        .cards_started(vec![
            card(
                "dépaysement",
                "Ce dépaysement commence ici.",
                "This change of scenery begins here.",
                retry_stress_artifacts(Artifact::Sound, 0),
            ),
            card(
                "flâner",
                "Nous aimons flâner le long de la Seine.",
                "We like strolling along the Seine.",
                retry_stress_artifacts(Artifact::Sound, 2),
            )
            .with_costs(retry_stress_cost(Artifact::Sound)),
            card(
                "canard",
                "Ce canard cache encore quelque chose.",
                "This newspaper story is still hiding something.",
                retry_stress_artifacts(Artifact::Scene, 3),
            )
            .with_costs(retry_stress_cost(Artifact::Scene)),
            card(
                "chouette",
                "Cette soirée était vraiment chouette.",
                "That evening was really lovely.",
                retry_stress_artifacts(Artifact::Picture, 1),
            )
            .with_costs(retry_stress_cost(Artifact::Picture)),
            card(
                "râler",
                "Il aime râler quand le train est en retard.",
                "He likes grumbling when the train is late.",
                retry_stress_recovered_artifacts(),
            ),
            card(
                "bof",
                "Bof, ce film ne m'a pas convaincu.",
                "Meh, that film did not convince me.",
                retry_stress_artifacts(Artifact::Picture, 4),
            )
            .with_costs(retry_stress_cost(Artifact::Picture)),
        ])
        .cards_running(Some((0, Artifact::Sound)))
        .with_elapsed(Duration::from_secs(142));

    let words_clear = base_words.clone().with_word_clear_pending(true);
    let review_back = review.clone();
    let stop_armed = cards_seed
        .clone()
        .cards_running(Some((2, Artifact::Picture)))
        .with_generation_stop_pending(true);
    let stopping = cards_seed
        .clone()
        .cards_running(Some((2, Artifact::Picture)))
        .generation_stop_started();
    let partial = cards_seed.clone().done_published_counted(
        "fr_2026-06-01_183029.apkg",
        "fr_2026-06-01_183029.pdf",
        "~/Documents/Kamishibai",
        2,
        2,
    );

    let done = App::new(pair())
        .with_screen(Screen::Done)
        .confirmed_learning("fr")
        .done_published(
            "fr_2026-06-01_183029.apkg",
            "fr_2026-06-01_183029.pdf",
            "~/Documents/Kamishibai",
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
        (
            String::from("03 · Your cards · S1 collapsed label tags"),
            base_cards,
        ),
        (
            String::from("03b · Your cards · S2 editor settings"),
            label_editor,
        ),
        (String::from("03c · Your cards · retrying"), retrying),
        (String::from("03d · Your cards · couldn't finish"), failed),
        (String::from("04 · Done"), done),
        (
            String::from("05 · Your cards · S3 pending register editor"),
            register_pinned,
        ),
        (
            String::from("06 · Your cards · S4 pending note editor"),
            note_typed,
        ),
        (
            String::from("07 · Your cards · S5 restored defaults"),
            restored,
        ),
        (
            String::from("08 · Your cards · S6 multiple pending"),
            multiple_pending,
        ),
        (
            String::from("09 · Your cards · S7 regenerating"),
            regenerating,
        ),
        (
            String::from("10 · Your cards · S8 regenerated"),
            regenerated,
        ),
        (
            String::from("11 · Your cards · S9 approximate pin"),
            approximate,
        ),
        (
            String::from("12 · Your cards · S10 narrow label tags"),
            narrow,
        ),
        (
            String::from("13 · Your cards · S11 mouse-selected editor"),
            mouse_selected,
        ),
        (
            String::from("14 · Your cards · S12 legacy editor settings"),
            legacy,
        ),
        (
            String::from("03e · Your cards · retry layout stress"),
            retry_stress,
        ),
        (String::from("15 · Esc · clear words armed"), words_clear),
        (String::from("16 · Esc · review back"), review_back),
        (String::from("17 · Esc · generation stop armed"), stop_armed),
        (String::from("18 · Esc · generation stopping"), stopping),
        (String::from("19 · Esc · partial publish"), partial),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn retry_layout_stress_state_keeps_attempt_history_on_card_heads() {
        let states = build_states();
        let (_, app) = states
            .get(21)
            .expect("retry layout stress state must stay at index 21");
        let backend = TestBackend::new(120, 50);
        let mut terminal = Terminal::new(backend).expect("backend must initialize");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("retry layout stress state must render");
        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            [
                "ai is working…",
                "gave up",
                "↻1",
                "↻2",
                "↻3",
                "casual",
                "statement",
                "b1",
            ]
            .into_iter()
            .all(|needle| rendered.contains(needle))
                && [
                    "retry 1/3",
                    "retry 2/3",
                    "retry 3/3",
                    "gave up after",
                    "1 ✗",
                    "2 ✗",
                    "3 ✗",
                    "4 ✗",
                    "paused",
                ]
                .into_iter()
                .all(|needle| !rendered.contains(needle))
                && app.cards_running_target() == Some((0, Artifact::Sound)),
            "retry layout stress state leaked attempt history out of the card heads:\n{rendered}"
        );
    }
}
