//! Integration flow for the inline sentence-label editor on `Your cards`.

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactFile, ArtifactSlot, AxisSet, CardArtifacts, CardDraft, CardMeta,
    GenerationCost, LanguagePair, Register, SentenceAxis, SentenceKind, SentenceLabelSelection,
    SentenceLabels, SentenceLevel,
};
use kamishibai::tui::{
    App, AppEvent, LabelEditorRow, MousePointer, Screen, Side, draw, mouse_pointer_at,
    sentence_label_event_at, transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;

const MARKER_WIDTH_FOR_TEST: usize = 2;

fn flat(app: &App) -> String {
    flat_at(app, 120, 50)
}

fn flat_at(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
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

fn artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn priced_artifacts() -> CardArtifacts {
    let meta = ArtifactFile::new("meta.json", PathBuf::from("/tmp/meta.json"), "1 B", false)
        .with_cost(GenerationCost::from_nanos(1_500_000));
    let sound = ArtifactFile::new("audio.wav", PathBuf::from("/tmp/audio.wav"), "1 B", false)
        .with_cost(GenerationCost::from_nanos(10_000_000));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(meta),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(sound),
    )
}

fn meta(term: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source meaning of {term}"),
        term,
        format!("hint for {term} without naming it"),
        format!("usage notes for {term}"),
        format!("Example with {term}."),
    )
    .with_sentence_labels(SentenceLabels::new(
        Register::Casual,
        SentenceLevel::B1,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::default(),
    ))
}

fn card() -> CardDraft {
    card_for("whilst")
}

fn card_for(term: &str) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta(term), None)
    .with_artifacts(artifacts())
}

fn priced_card_for(term: &str) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta(term), None)
    .with_artifacts(priced_artifacts())
}

fn partial_card_for(term: &str) -> CardDraft {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    );
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta(term), None)
    .with_artifacts(artifacts)
}

fn legacy_card() -> CardDraft {
    CardDraft::new(
        "whilst",
        "understanding for whilst",
        LanguagePair::new("en", "ru"),
    )
    .with_meta(
        CardMeta::new(
            "/whilst/",
            "/whilst sentence/",
            "meaning",
            5,
            "source whilst",
            "whilst",
            "hint",
            "context",
            "Example with whilst.",
        ),
        None,
    )
    .with_artifacts(artifacts())
}

fn seeded(draft: CardDraft) -> App {
    seeded_cards(vec![draft])
}

fn seeded_cards(drafts: Vec<CardDraft>) -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(drafts)
}

fn cell_of(app: &App, needle: &str, width: u16, height: u16) -> (u16, u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = rendered[..start].chars().count();
            return (
                u16::try_from(column).expect("rendered column must fit the terminal"),
                row,
            );
        }
    }
    panic!("the rendered screen never showed '{needle}'");
}

fn carousel_geometry(rendered: &str, choice: &str) -> (usize, usize, usize) {
    let line = rendered
        .lines()
        .find(|line| line.contains(choice) && line.contains('<') && line.contains('>'))
        .expect("sentence-label choice must remain inside its carousel track");
    let left = line
        .find('<')
        .expect("sentence-label carousel must show its left chevron");
    let token = line
        .find(choice)
        .expect("sentence-label carousel must show its selected choice");
    let right = line
        .find('>')
        .expect("sentence-label carousel must show its right chevron");
    (left, token, right)
}

fn choice_indices(
    app: &App,
    terminal: Rect,
    row: u16,
    columns: impl Iterator<Item = u16>,
    axis: LabelEditorRow,
) -> Vec<usize> {
    let mut indices = Vec::new();
    for column in columns {
        let Some(AppEvent::SentenceLabelChoose(row, index)) =
            sentence_label_event_at(app, terminal, column, row)
        else {
            continue;
        };
        if row == axis && indices.last() != Some(&index) {
            indices.push(index);
        }
    }
    indices
}

fn choice_regions(
    app: &App,
    terminal: Rect,
    screen_row: u16,
    axis: LabelEditorRow,
) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    for column in 0..terminal.width {
        let Some(AppEvent::SentenceLabelChoose(row, index)) =
            sentence_label_event_at(app, terminal, column, screen_row)
        else {
            continue;
        };
        if row != axis {
            continue;
        }
        match regions.last_mut() {
            Some((last, width)) if *last == index => *width += 1,
            _ => regions.push((index, 1)),
        }
    }
    regions
}

#[test]
fn both_r_keys_are_inert_while_sentence_tags_and_space_open_the_live_editor() {
    let collapsed = seeded(card());
    let (lowercase, lowercase_side) = transit(collapsed.clone(), AppEvent::KeyChar('r'));
    let (uppercase, uppercase_side) = transit(collapsed.clone(), AppEvent::KeyChar('R'));
    let spaced = transit(collapsed.clone(), AppEvent::KeyChar(' ')).0;
    let opened = transit(
        collapsed,
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let rendered = flat(&opened);
    let ctrl = rendered
        .find("[Ctrl+G] regenerate")
        .expect("editor footer must show regeneration");
    let arrows = rendered
        .find("[← →] pick")
        .expect("editor footer must show chip navigation");
    assert!(
        lowercase.sentence_editor().is_none()
            && uppercase.sentence_editor().is_none()
            && lowercase_side == Side::None
            && uppercase_side == Side::None
            && spaced.card_expanded()
            && spaced
                .sentence_editor()
                .is_some_and(|editor| editor.row() == LabelEditorRow::Register)
            && opened.card_expanded()
            && opened
                .sentence_editor()
                .is_some_and(|editor| editor.row() == LabelEditorRow::Register)
            && opened.modal().is_none()
            && rendered.contains("how should it sound?")
            && rendered.contains("what's the desired level?")
            && rendered.contains("what kind of phrase?")
            && rendered.contains("one more thing")
            && !rendered.contains("what feels wrong?")
            && rendered.contains("casual")
            && rendered.contains("b1")
            && !rendered.contains("B1")
            && !rendered.contains("medium")
            && !rendered.contains("a2")
            && !rendered.contains("b2")
            && !rendered.contains("formal")
            && !rendered.contains("literary")
            && !rendered.contains("archaic")
            && !rendered.contains("question")
            && !rendered.contains("dialogue")
            && rendered.contains("say what should change")
            && rendered.contains("[← →] pick")
            && rendered.contains("[↑ ↓] row")
            && rendered.contains("[Ctrl+G] regenerate")
            && rendered.contains("[Esc] close")
            && !rendered.contains("[Enter]")
            && !rendered.contains("[R] change")
            && ctrl < arrows,
        "r/R emitted an action, opened the editor, or sentence tags, Space, and the live editor footer drifted apart: {rendered}"
    );
}

#[test]
fn every_carousel_surrounds_its_visible_choice_with_direction_chevrons() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let selected = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Type, 2),
    )
    .0;
    let rendered = flat(&selected);
    let ordered = [
        ("how should it sound?", "casual"),
        ("what kind of phrase?", "request"),
        ("what's the desired level?", "b1"),
    ]
    .into_iter()
    .all(|(question, choice)| {
        let line = rendered
            .lines()
            .find(|line| line.contains(question))
            .expect("sentence-label question must remain on its carousel row");
        let left = line
            .find('<')
            .expect("sentence-label carousel must show its left chevron");
        let token = line
            .find(choice)
            .expect("sentence-label carousel must show its selected choice");
        let right = line
            .find('>')
            .expect("sentence-label carousel must show its right chevron");
        left < token && token < right
    });
    assert!(
        ordered,
        "one or more sentence-label carousels omitted or misplaced their direction chevrons: {rendered}"
    );
}

#[test]
fn carousels_share_one_track_and_place_choices_along_their_progress() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let selected = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Type, 2),
    )
    .0;
    let rendered = flat(&selected);
    let geometry = [
        carousel_geometry(&rendered, "casual"),
        carousel_geometry(&rendered, "request"),
        carousel_geometry(&rendered, "b1"),
    ];
    assert!(
        geometry.iter().all(|row| row.0 == geometry[0].0)
            && geometry.iter().all(|row| row.2 == geometry[0].2)
            && geometry[0].2.saturating_sub(geometry[0].0) == 24
            && geometry[0].1 < geometry[1].1
            && geometry[0].1 < geometry[2].1,
        "sentence-label carousels did not keep aligned chevrons while placing later choices farther along the track: {geometry:?}\n{rendered}"
    );
}

#[test]
fn every_axis_moves_forward_and_keeps_one_balanced_region_per_hidden_choice() {
    let terminal = Rect::new(0, 0, 120, 50);
    let axes = [
        (
            LabelEditorRow::Register,
            ["neutral", "casual", "formal", "literary", "archaic"].as_slice(),
        ),
        (
            LabelEditorRow::Type,
            [
                "statement",
                "question",
                "request",
                "exclamation",
                "dialogue",
            ]
            .as_slice(),
        ),
        (
            LabelEditorRow::Level,
            ["a1", "a2", "b1", "b2", "c1", "c2"].as_slice(),
        ),
    ];
    let mut valid = true;
    let mut outer = None;
    for (axis, tokens) in axes {
        let mut previous = None;
        for (index, token) in tokens.iter().enumerate() {
            let app = transit(
                transit(seeded(card()), AppEvent::SentenceLabelFocus(axis)).0,
                AppEvent::SentenceLabelChoose(axis, index),
            )
            .0;
            let rendered = flat(&app);
            let geometry = carousel_geometry(&rendered, token);
            let row = cell_of(&app, token, terminal.width, terminal.height).1;
            let regions = choice_regions(&app, terminal, row, axis);
            let hidden = regions
                .iter()
                .filter_map(|(choice, width)| (*choice != index).then_some(*width))
                .collect::<Vec<_>>();
            let center = 2 * geometry.1 + token.chars().count();
            valid &= outer.is_none_or(|columns| columns == (geometry.0, geometry.2));
            valid &= previous.is_none_or(|column| column < center);
            valid &= regions.iter().map(|region| region.0).eq(0..tokens.len());
            valid &= regions.get(index).is_some_and(|region| region.0 == index);
            valid &= hidden.iter().all(|width| *width >= MARKER_WIDTH_FOR_TEST);
            valid &= hidden
                .iter()
                .max()
                .zip(hidden.iter().min())
                .is_none_or(|(maximum, minimum)| maximum - minimum <= 1);
            outer = Some((geometry.0, geometry.2));
            previous = Some(center);
        }
    }
    assert!(
        valid,
        "a sentence-label axis re-centered, lost a hidden-choice division, or unbalanced its marker widths"
    );
}

#[test]
fn carousel_track_stays_fixed_while_a_later_choice_center_advances_inside_it() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Type),
    )
    .0;
    let before = carousel_geometry(&flat(&opened), "statement");
    let changed = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Type, 3),
    )
    .0;
    let after = carousel_geometry(&flat(&changed), "exclamation");
    assert!(
        before.0 == after.0
            && before.2 == after.2
            && 2 * before.1 + "statement".chars().count()
                < 2 * after.1 + "exclamation".chars().count(),
        "changing from the first to a later choice failed to advance it inside the fixed carousel track: {before:?} -> {after:?}"
    );
}

#[test]
fn advancing_one_choice_transfers_one_marker_segment_to_the_leading_side() {
    let terminal = Rect::new(0, 0, 120, 50);
    let request = transit(
        transit(
            seeded(card()),
            AppEvent::SentenceLabelFocus(LabelEditorRow::Type),
        )
        .0,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Type, 2),
    )
    .0;
    let request_cell = cell_of(&request, "request", terminal.width, terminal.height);
    let request_left = choice_indices(
        &request,
        terminal,
        request_cell.1,
        0..request_cell.0.saturating_sub(1),
        LabelEditorRow::Type,
    );
    let request_right = choice_indices(
        &request,
        terminal,
        request_cell.1,
        request_cell.0.saturating_add(8)..terminal.width,
        LabelEditorRow::Type,
    );
    let exclamation = transit(
        request.clone(),
        AppEvent::SentenceLabelChoose(LabelEditorRow::Type, 3),
    )
    .0;
    let exclamation_cell = cell_of(&exclamation, "exclamation", terminal.width, terminal.height);
    let exclamation_left = choice_indices(
        &exclamation,
        terminal,
        exclamation_cell.1,
        0..exclamation_cell.0.saturating_sub(1),
        LabelEditorRow::Type,
    );
    let exclamation_right = choice_indices(
        &exclamation,
        terminal,
        exclamation_cell.1,
        exclamation_cell.0.saturating_add(12)..terminal.width,
        LabelEditorRow::Type,
    );
    assert_eq!(
        (
            request_left,
            request_right,
            exclamation_left,
            exclamation_right,
            2 * request_cell.0 + 7 < 2 * exclamation_cell.0 + 11,
        ),
        (vec![0, 1], vec![3, 4], vec![0, 1, 2], vec![4], true),
        "advancing one type failed to move its chip and transfer exactly one visible marker segment"
    );
}

#[test]
fn narrow_editor_wraps_three_complete_progress_tracks_to_the_same_columns() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let rendered = flat_at(&opened, 50, 50);
    let geometry = [
        carousel_geometry(&rendered, "casual"),
        carousel_geometry(&rendered, "statement"),
        carousel_geometry(&rendered, "b1"),
    ];
    assert!(
        geometry.iter().all(|row| row.0 == geometry[0].0)
            && geometry.iter().all(|row| row.2 == geometry[0].2)
            && geometry[1].1 < geometry[0].1
            && geometry[0].1 < geometry[2].1,
        "narrow editor split, misaligned, or re-centered a wrapped carousel track: {geometry:?}\n{rendered}"
    );
}

#[test]
fn both_cells_of_each_direction_chevron_move_its_own_carousel() {
    let terminal = Rect::new(0, 0, 120, 50);
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Level),
    )
    .0;
    let left = cell_of(&opened, "<", terminal.width, terminal.height);
    let right = cell_of(&opened, ">", terminal.width, terminal.height);
    let left_cells = [left, (left.0 + 1, left.1)];
    let right_cells = [(right.0 - 1, right.1), right];
    let left_events =
        left_cells.map(|cell| sentence_label_event_at(&opened, terminal, cell.0, cell.1));
    let right_events =
        right_cells.map(|cell| sentence_label_event_at(&opened, terminal, cell.0, cell.1));
    let left_pointers = left_cells.map(|cell| mouse_pointer_at(&opened, terminal, cell.0, cell.1));
    let right_pointers =
        right_cells.map(|cell| mouse_pointer_at(&opened, terminal, cell.0, cell.1));
    let moved_left = transit(
        opened.clone(),
        left_events[0]
            .clone()
            .expect("left sentence-label chevron must be clickable"),
    )
    .0;
    let moved_right = transit(
        opened,
        right_events[0]
            .clone()
            .expect("right sentence-label chevron must be clickable"),
    )
    .0;
    assert_eq!(
        (
            left_events,
            right_events,
            left_pointers,
            right_pointers,
            moved_left
                .sentence_editor()
                .map(|editor| (editor.row(), editor.selection().register())),
            moved_right
                .sentence_editor()
                .map(|editor| (editor.row(), editor.selection().register())),
        ),
        (
            [
                Some(AppEvent::SentenceLabelAdvance(
                    LabelEditorRow::Register,
                    false,
                )),
                Some(AppEvent::SentenceLabelAdvance(
                    LabelEditorRow::Register,
                    false,
                )),
            ],
            [
                Some(AppEvent::SentenceLabelAdvance(
                    LabelEditorRow::Register,
                    true,
                )),
                Some(AppEvent::SentenceLabelAdvance(
                    LabelEditorRow::Register,
                    true,
                )),
            ],
            [MousePointer::Hand, MousePointer::Hand],
            [MousePointer::Hand, MousePointer::Hand],
            Some((LabelEditorRow::Register, Some(Register::Neutral))),
            Some((LabelEditorRow::Register, Some(Register::Formal))),
        ),
        "sentence-label chevrons lost a hit cell, moved the wrong row, or skipped an adjacent choice"
    );
}

#[test]
fn chip_and_note_edits_stage_immediately_and_escape_collapses_without_rollback() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let changed = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 2),
    )
    .0;
    let note = transit(
        changed
            .clone()
            .sentence_editor_focused(LabelEditorRow::Note),
        AppEvent::KeyChar('x'),
    )
    .0;
    let closed = transit(note.clone(), AppEvent::Cancel).0;
    let changed_rewrite = changed.cards()[0]
        .rewrite()
        .expect("chip edit must stage immediately");
    let closed_rewrite = closed.cards()[0]
        .rewrite()
        .expect("Esc must retain the live pending rewrite");
    assert_eq!(
        (
            changed_rewrite.selection().register(),
            changed_rewrite.selection().level(),
            changed_rewrite.selection().kind(),
            changed_rewrite
                .selection()
                .pinned()
                .iter()
                .collect::<Vec<_>>(),
            note.cards()[0]
                .rewrite()
                .map(kamishibai::session::CardRewrite::note),
            closed.sentence_editor(),
            closed.card_expanded(),
            closed_rewrite.note(),
            closed.cards()[0].meta().is_some(),
            closed.cards()[0].artifacts().all_ready(),
        ),
        (
            Some(Register::Formal),
            Some(SentenceLevel::B1),
            Some(SentenceKind::Statement),
            vec![SentenceAxis::Register],
            Some("x"),
            None,
            false,
            "x",
            true,
            true
        ),
        "live staging lost an edit, invalidated artifacts early, or failed to collapse without rollback"
    );
}

#[test]
fn returning_to_the_baseline_with_a_blank_note_automatically_unstages() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let changed = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 2),
    )
    .0;
    let blank = transit(
        changed.sentence_editor_focused(LabelEditorRow::Note),
        AppEvent::KeyChar(' '),
    )
    .0;
    let restored = transit(
        blank,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 1),
    )
    .0;
    let editor = restored
        .sentence_editor()
        .expect("automatic unstage must keep the editor open");
    assert_eq!(
        (
            restored.cards()[0].rewrite(),
            editor.selection().register(),
            editor.selection().pinned().contains(SentenceAxis::Register),
            editor.note().value(),
            restored.cards()[0].meta().is_some(),
            restored.cards()[0].artifacts().all_ready(),
        ),
        (None, Some(Register::Casual), false, " ", true, true),
        "restoring the generated value with a whitespace-only note left a phantom pending rewrite"
    );
}

#[test]
fn enter_is_inert_but_ctrl_g_closes_the_editor_and_requests_all_pending_cards() {
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let staged = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 2),
    )
    .0;
    let (after_enter, enter_side) = transit(staged, AppEvent::KeyEnter);
    let (regenerating, regenerate_side) = transit(after_enter.clone(), AppEvent::Generate);
    assert_eq!(
        (
            enter_side,
            after_enter.sentence_editor().is_some(),
            after_enter.card_expanded(),
            after_enter.cards()[0].rewrite().is_some(),
            regenerate_side,
            regenerating.sentence_editor(),
            regenerating.card_expanded(),
            regenerating.cards()[0].rewrite().is_some(),
            regenerating.cards()[0].meta().is_some(),
            regenerating.cards()[0].artifacts().all_ready(),
        ),
        (
            Side::None,
            true,
            true,
            true,
            Side::RegenerateCards,
            None,
            false,
            true,
            true,
            true,
        ),
        "Enter committed live edits or Ctrl+G failed to hand every staged card to regeneration"
    );
}

#[test]
fn repeating_the_active_legacy_chip_restores_the_empty_baseline() {
    let opened = transit(seeded(legacy_card()), AppEvent::KeyChar(' ')).0;
    let selected = transit(
        opened,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 0),
    )
    .0;
    let reset = transit(
        selected.clone(),
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 0),
    )
    .0;
    assert_eq!(
        (
            selected.cards()[0].rewrite().is_some(),
            selected
                .sentence_editor()
                .and_then(|editor| editor.selection().register()),
            reset.cards()[0].rewrite(),
            reset
                .sentence_editor()
                .and_then(|editor| editor.selection().register()),
        ),
        (true, Some(Register::Neutral), None, None),
        "repeating an active legacy chip failed to restore the empty default"
    );
}

#[test]
fn collapsed_audio_row_tags_hit_only_their_boxes_not_the_plain_gap_or_artifacts() {
    let terminal = Rect::new(0, 0, 120, 50);
    let collapsed = seeded(priced_card_for("whilst"));
    let term = cell_of(&collapsed, "whilst", 120, 50);
    let meta = cell_of(&collapsed, "meta.json", 120, 50);
    let audio = cell_of(&collapsed, "audio.wav", 120, 50);
    let register = cell_of(&collapsed, "casual", 120, 50);
    let kind = cell_of(&collapsed, "statement", 120, 50);
    let level = cell_of(&collapsed, "b1", 120, 50);
    let padding = (register.0 - 1, register.1);
    let first_gap = (kind.0 - 2, kind.1);
    let second_gap = (level.0 - 2, level.1);
    let status_gap = (register.0 - 2, register.1);
    let arrow = (
        term.0 + u16::try_from("whilst".chars().count()).expect("term width must fit") + 1,
        term.1,
    );
    let focus = Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register));
    assert_eq!(
        (
            (
                sentence_label_event_at(&collapsed, terminal, register.0, register.1),
                mouse_pointer_at(&collapsed, terminal, register.0, register.1),
                sentence_label_event_at(&collapsed, terminal, level.0, level.1),
                mouse_pointer_at(&collapsed, terminal, level.0, level.1),
                sentence_label_event_at(&collapsed, terminal, kind.0, kind.1),
                mouse_pointer_at(&collapsed, terminal, kind.0, kind.1),
            ),
            (
                sentence_label_event_at(&collapsed, terminal, padding.0, padding.1),
                mouse_pointer_at(&collapsed, terminal, padding.0, padding.1),
                sentence_label_event_at(&collapsed, terminal, first_gap.0, first_gap.1),
                mouse_pointer_at(&collapsed, terminal, first_gap.0, first_gap.1),
                sentence_label_event_at(&collapsed, terminal, second_gap.0, second_gap.1),
                mouse_pointer_at(&collapsed, terminal, second_gap.0, second_gap.1),
            ),
            (
                sentence_label_event_at(&collapsed, terminal, arrow.0, arrow.1),
                mouse_pointer_at(&collapsed, terminal, arrow.0, arrow.1),
                sentence_label_event_at(&collapsed, terminal, status_gap.0, status_gap.1),
                mouse_pointer_at(&collapsed, terminal, status_gap.0, status_gap.1),
            ),
            (
                sentence_label_event_at(&collapsed, terminal, meta.0, meta.1),
                mouse_pointer_at(&collapsed, terminal, meta.0, meta.1),
                sentence_label_event_at(&collapsed, terminal, audio.0, audio.1),
                mouse_pointer_at(&collapsed, terminal, audio.0, audio.1),
            ),
            (meta.1, audio.1, register.1, level.1, kind.1),
        ),
        (
            (
                focus.clone(),
                MousePointer::Hand,
                focus.clone(),
                MousePointer::Hand,
                focus.clone(),
                MousePointer::Hand,
            ),
            (
                focus,
                MousePointer::Hand,
                None,
                MousePointer::Arrow,
                None,
                MousePointer::Arrow,
            ),
            (None, MousePointer::Arrow, None, MousePointer::Arrow),
            (None, MousePointer::Hand, None, MousePointer::Hand),
            (meta.1, meta.1 + 1, audio.1, audio.1, audio.1),
        ),
        "collapsed audio-row tags leaked hits into the status gap, artifact cells, inter-chip gaps, or head"
    );
}

#[test]
fn too_narrow_atomic_tags_keep_the_card_head_as_the_editor_entry() {
    let terminal = Rect::new(0, 0, 50, 30);
    let collapsed = seeded(priced_card_for("whilst"));
    let term = cell_of(&collapsed, "whilst", terminal.width, terminal.height);
    assert_eq!(
        (
            sentence_label_event_at(&collapsed, terminal, term.0, term.1),
            mouse_pointer_at(&collapsed, terminal, term.0, term.1),
        ),
        (
            Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register)),
            MousePointer::Hand,
        ),
        "hidden atomic tags left the narrow labeled card without an editor entry"
    );
}

#[test]
fn partial_narrow_card_hides_atomic_tags_and_keeps_the_card_head_entry() {
    let terminal = Rect::new(0, 0, 60, 30);
    let collapsed = seeded(partial_card_for("whilst")).cards_running(Some((0, Artifact::Sound)));
    let term = cell_of(&collapsed, "whilst", terminal.width, terminal.height);
    let rendered = flat_at(&collapsed, terminal.width, terminal.height);
    assert_eq!(
        (
            rendered.contains("casual"),
            rendered.contains("b1"),
            rendered.contains("statement"),
            sentence_label_event_at(&collapsed, terminal, term.0, term.1),
            mouse_pointer_at(&collapsed, terminal, term.0, term.1),
        ),
        (
            false,
            false,
            false,
            Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register)),
            MousePointer::Hand,
        ),
        "partial narrow layout exposed an incomplete tag summary or lost the card-head editor entry"
    );
}

#[test]
fn narrow_layout_keeps_wrapped_sentence_tags_clickable_on_the_three_artifact_rows() {
    let terminal = Rect::new(0, 0, 60, 30);
    let collapsed = seeded(priced_card_for("whilst"));
    let audio = cell_of(&collapsed, "audio", 60, 30);
    let scene = cell_of(&collapsed, "scene", 60, 30);
    let picture = cell_of(&collapsed, "picture", 60, 30);
    let register = cell_of(&collapsed, "casual", 60, 30);
    let kind = cell_of(&collapsed, "statement", 60, 30);
    let level = cell_of(&collapsed, "b1", 60, 30);
    let focus = Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register));
    assert_eq!(
        (
            (register.1, kind.1, level.1),
            sentence_label_event_at(&collapsed, terminal, register.0, register.1),
            mouse_pointer_at(&collapsed, terminal, register.0, register.1),
            sentence_label_event_at(&collapsed, terminal, level.0, level.1),
            mouse_pointer_at(&collapsed, terminal, level.0, level.1),
            sentence_label_event_at(&collapsed, terminal, kind.0, kind.1),
            mouse_pointer_at(&collapsed, terminal, kind.0, kind.1),
        ),
        (
            (audio.1, scene.1, picture.1),
            focus.clone(),
            MousePointer::Hand,
            focus.clone(),
            MousePointer::Hand,
            focus,
            MousePointer::Hand,
        ),
        "narrow layout detached wrapped sentence tags from the three plain artifact rows or their hit regions"
    );
}

#[test]
fn clicking_an_unfocused_cards_tags_selects_it_and_opens_its_editor() {
    let terminal = Rect::new(0, 0, 120, 50);
    let collapsed = seeded_cards(vec![
        priced_card_for("whilst"),
        priced_card_for("thereafter"),
    ]);
    let term = cell_of(&collapsed, "thereafter", 120, 50);
    let first_tag = cell_of(&collapsed, "casual", 120, 50);
    let tag = (first_tag.0, term.1 + 2);
    let event = sentence_label_event_at(&collapsed, terminal, tag.0, tag.1)
        .expect("the unfocused card tag must be a mouse target");
    let opened = transit(collapsed, event).0;
    assert_eq!(
        (
            opened.card_selected(),
            opened.card_expanded(),
            opened.sentence_editor().map(|editor| editor.row()),
        ),
        (1, true, Some(LabelEditorRow::Register)),
        "clicking an unfocused card tag did not select that card and open its editor"
    );
}

#[test]
fn expanded_editor_removes_the_collapsed_summary_tag_open_hit() {
    let terminal = Rect::new(0, 0, 120, 50);
    let collapsed = seeded(priced_card_for("whilst"));
    let tag = cell_of(&collapsed, "casual", 120, 50);
    let opened = transit(collapsed, AppEvent::KeyEnter).0;
    let event = sentence_label_event_at(&opened, terminal, tag.0, tag.1);
    assert_eq!(
        (
            opened.card_expanded(),
            opened.sentence_editor().is_some(),
            matches!(
                event,
                Some(AppEvent::SentenceLabelOpen(_, LabelEditorRow::Register))
            ),
        ),
        (true, true, false),
        "expanded editor left a phantom collapsed-summary open hit"
    );
}

#[test]
fn clicking_the_legacy_card_head_opens_its_unattributed_editor() {
    let terminal = Rect::new(0, 0, 120, 50);
    let collapsed = seeded(legacy_card());
    let head = cell_of(&collapsed, "whilst", 120, 50);
    let event = sentence_label_event_at(&collapsed, terminal, head.0, head.1);
    let (opened, side) = transit(
        collapsed,
        event
            .clone()
            .expect("the legacy card head must remain a mouse tuning target"),
    );
    assert_eq!(
        (
            event,
            side,
            opened.card_expanded(),
            opened.sentence_editor().map(|editor| editor.row()),
            opened
                .sentence_editor()
                .map(|editor| editor.selection().attributed()),
        ),
        (
            Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register)),
            Side::None,
            true,
            Some(LabelEditorRow::Register),
            Some(false),
        ),
        "legacy metadata had no mouse-accessible path into its empty label editor"
    );
}

#[test]
fn legacy_editor_keeps_its_questions_visible_around_empty_axes() {
    let opened = transit(seeded(legacy_card()), AppEvent::KeyChar(' ')).0;
    let rendered = flat(&opened);
    let question = cell_of(&opened, "how should it sound?", 120, 50);
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, &opened)).expect("draw");
    let buffer = terminal.backend().buffer();
    assert_eq!(
        (
            rendered.contains("what kind of phrase?"),
            rendered.contains("what's the desired level?"),
            buffer[question].fg,
        ),
        (true, true, Color::Rgb(0xe6, 0xe3, 0xda)),
        "legacy empty axes hid their carousel questions or dimmed the focused one"
    );
}

#[test]
fn mouse_hits_open_the_sentence_tags_choose_and_pin_a_chip_then_focus_the_note() {
    let terminal = Rect::new(0, 0, 120, 50);
    let collapsed = seeded(card());
    let tags_cell = cell_of(&collapsed, "casual", 120, 50);
    let tags_event = sentence_label_event_at(&collapsed, terminal, tags_cell.0, tags_cell.1);
    let (opened, tags_side) = transit(
        collapsed,
        tags_event
            .clone()
            .expect("the rendered head tags must be clickable"),
    );
    let casual = cell_of(&opened, "casual", 120, 50);
    let right = cell_of(&opened, ">", 120, 50);
    let chip_cells = [(right.0 - 4, casual.1), (right.0 - 2, casual.1)];
    let chip_events =
        chip_cells.map(|cell| sentence_label_event_at(&opened, terminal, cell.0, cell.1));
    let (chosen, chip_side) = transit(
        opened,
        chip_events[0]
            .clone()
            .expect("the farthest rendered register marker must be clickable"),
    );
    let note_cell = cell_of(&chosen, "say what should change", 120, 50);
    let note_event = sentence_label_event_at(&chosen, terminal, note_cell.0, note_cell.1);
    let (focused, note_side) = transit(
        chosen,
        note_event
            .clone()
            .expect("the rendered note field must be clickable"),
    );
    let editor = focused
        .sentence_editor()
        .expect("mouse hits must leave the editor open");
    let rewrite = focused.cards()[0]
        .rewrite()
        .expect("clicking a chip must stage its rewrite immediately");
    assert_eq!(
        (
            tags_event,
            tags_side,
            chip_events,
            chip_side,
            note_event,
            note_side,
            editor.row(),
            editor.selection().register(),
            editor.selection().pinned().contains(SentenceAxis::Register),
            rewrite.selection().register(),
            focused.cards()[0].meta().is_some(),
            focused.cards()[0].artifacts().all_ready(),
        ),
        (
            Some(AppEvent::SentenceLabelOpen(0, LabelEditorRow::Register)),
            Side::None,
            [
                Some(AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 4)),
                Some(AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 4)),
            ],
            Side::None,
            Some(AppEvent::SentenceLabelFocus(LabelEditorRow::Note)),
            Side::None,
            LabelEditorRow::Note,
            Some(Register::Archaic),
            true,
            Some(Register::Archaic),
            true,
            true,
        ),
        "mouse editing did not live-stage the chip while preserving the generated card"
    );
}

#[test]
fn edge_cells_of_the_farthest_cefr_marker_choose_c2() {
    let terminal = Rect::new(0, 0, 120, 50);
    let opened = transit(
        seeded(card()),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Level),
    )
    .0;
    let b1 = cell_of(&opened, "b1", 120, 50);
    let right = cell_of(&opened, ">", 120, 50);
    let cells = [(right.0 - 4, b1.1), (right.0 - 2, b1.1)];
    let events = cells.map(|cell| sentence_label_event_at(&opened, terminal, cell.0, cell.1));
    let chosen = transit(
        opened,
        events[0]
            .clone()
            .expect("the farthest CEFR marker must be clickable"),
    )
    .0;
    let editor = chosen
        .sentence_editor()
        .expect("choosing a CEFR marker must keep the editor open");
    assert_eq!(
        (
            events,
            editor.selection().level(),
            editor.selection().pinned().contains(SentenceAxis::Level),
        ),
        (
            [
                Some(AppEvent::SentenceLabelChoose(LabelEditorRow::Level, 5)),
                Some(AppEvent::SentenceLabelChoose(LabelEditorRow::Level, 5)),
            ],
            Some(SentenceLevel::C2),
            true,
        ),
        "the farthest CEFR marker did not expose both cells as the c2 choice"
    );
}

#[test]
fn ctrl_g_keeps_multiple_independent_pending_cards_in_one_bulk_request() {
    let first = transit(
        seeded_cards(vec![card_for("whilst"), card_for("thereafter")]),
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    )
    .0;
    let first = transit(
        first,
        AppEvent::SentenceLabelChoose(LabelEditorRow::Register, 2),
    )
    .0;
    let collapsed = transit(first, AppEvent::Cancel).0;
    let selected = transit(collapsed, AppEvent::NavNext).0;
    let second = transit(selected, AppEvent::SentenceLabelFocus(LabelEditorRow::Note)).0;
    let second = transit(second, AppEvent::KeyChar('x')).0;
    let (regenerating, side) = transit(second, AppEvent::Generate);
    assert_eq!(
        (
            side,
            regenerating.sentence_editor(),
            regenerating.cards()[0].rewrite().is_some(),
            regenerating.cards()[1].rewrite().is_some(),
            regenerating.cards()[0].meta().is_some(),
            regenerating.cards()[1].meta().is_some(),
        ),
        (Side::RegenerateCards, None, true, true, true, true),
        "Ctrl+G dropped one of the independently staged cards before bulk regeneration"
    );
}

#[test]
fn an_active_rewrite_cannot_reopen_or_read_as_pending_during_meta_generation() {
    let baseline = SentenceLabelSelection::from_labels(
        card()
            .meta()
            .and_then(CardMeta::sentence_labels)
            .expect("labeled card must expose its baseline"),
    );
    let active = card()
        .staging_rewrite(
            baseline.choosing(SentenceAxis::Register, 2),
            "make it formal",
        )
        .starting_rewrite();
    let start = seeded(active);
    let (spaced, space_side) = transit(start, AppEvent::KeyChar(' '));
    let (clicked, click_side) = transit(
        spaced,
        AppEvent::SentenceLabelFocus(LabelEditorRow::Register),
    );
    assert_eq!(
        (
            space_side,
            click_side,
            clicked.sentence_editor(),
            clicked.card_expanded(),
            clicked.cards_pending(),
            clicked.cards()[0].meta(),
            clicked.cards()[0]
                .rewrite()
                .map(kamishibai::session::CardRewrite::started),
        ),
        (Side::None, Side::None, None, false, 0, None, Some(true)),
        "active metadata generation reopened the editor or masqueraded as a staged rewrite"
    );
}
