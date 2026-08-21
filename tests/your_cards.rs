//! Integration render tests for the `Your cards` screen (04-your-cards.png)
//! and its two inline variants (retry, failure).

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactCosts, ArtifactFile, ArtifactSlot, AttemptFault, AttemptPenalties,
    AttemptScorecard, AxisSet, CardArtifacts, CardDraft, CardMeta, GenerationCost, LanguagePair,
    Register, SentenceAxis, SentenceKind, SentenceLabelSelection, SentenceLabels, SentenceLevel,
};
use kamishibai::tui::{
    App, AppEvent, LabelEditorRow, Screen, Side, draw, link_at, scroll_body_width, scroll_viewport,
    transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 50);
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

fn selected_label_highlighted(app: &App, needle: &str) -> bool {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = rendered[..start].chars().count() as u16;
            return (0..needle.chars().count()).all(|offset| {
                buffer[(column + offset as u16, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f)
            });
        }
    }
    false
}

fn matching_cells(app: &App, needle: &str) -> Vec<(Color, Color, Modifier)> {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut cells = Vec::new();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        let Some(start) = rendered.find(needle) else {
            continue;
        };
        let column = u16::try_from(rendered[..start].chars().count())
            .expect("rendered column must fit the terminal");
        for offset in 0..needle.chars().count() {
            let cell = &buffer[(
                column + u16::try_from(offset).expect("needle width must fit the terminal"),
                row,
            )];
            cells.push((cell.fg, cell.bg, cell.modifier));
        }
    }
    cells
}

fn rendered_buffer(app: &App) -> Buffer {
    rendered_buffer_at(app, 120, 50)
}

fn rendered_buffer_at(app: &App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal.backend().buffer().clone()
}

fn position_of(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            return (
                u16::try_from(rendered[..start].chars().count())
                    .expect("rendered column must fit the terminal"),
                row,
            );
        }
    }
    let rendered = (0..buffer.area.height)
        .map(|row| row_text(buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    panic!("the rendered screen never showed '{needle}':\n{rendered}");
}

fn term_ink(buffer: &Buffer, term: &str) -> Color {
    let (column, row) = position_of(buffer, term);
    buffer[(column, row)].fg
}

fn row_text(buffer: &Buffer, row: u16) -> String {
    (0..buffer.area.width)
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

fn chip_has_style(buffer: &Buffer, token: &str, foreground: Color, background: Color) -> bool {
    let (column, row) = position_of(buffer, token);
    let token_width = u16::try_from(token.chars().count()).expect("token width must fit");
    let token_matches = (0..token_width).all(|offset| {
        let cell = &buffer[(column + offset, row)];
        cell.fg == foreground && cell.bg == background
    });
    token_matches
        && column > 0
        && buffer[(column - 1, row)].bg == background
        && buffer[(column + token_width, row)].bg == background
}

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn priced_file(name: &str, nanos: u64) -> ArtifactFile {
    ArtifactFile::new(name, PathBuf::from(format!("/tmp/{name}")), "1 B", false)
        .with_cost(GenerationCost::from_nanos(nanos))
}

fn cached_file(name: &str) -> ArtifactFile {
    ArtifactFile::new(name, PathBuf::from(format!("/tmp/{name}")), "1 B", true)
}

fn priced_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(priced_file("scene.json", 2_000_000)),
        ArtifactSlot::fresh(Artifact::Picture)
            .succeeded_with(priced_file("picture.jpg", 67_300_000)),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(priced_file("audio.wav", 10_000_000)),
    )
}

fn recovered_priced_artifacts(attempts: usize) -> CardArtifacts {
    let picture = (0..attempts)
        .fold(ArtifactSlot::fresh(Artifact::Picture), |slot, _| {
            slot.faulted(rejected_attempt())
        })
        .succeeded_with(priced_file("picture.jpg", 67_300_000));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(priced_file("scene.json", 2_000_000)),
        picture,
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(priced_file("audio.wav", 10_000_000)),
    )
}

fn linked_artifacts(name: &str) -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta)
            .succeeded_with(priced_file(format!("{name}.json").as_str(), 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn partial_priced_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 10_000_000)),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn cached_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(cached_file("meta.json")),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(cached_file("scene.json")),
        ArtifactSlot::fresh(Artifact::Picture).succeeded_with(cached_file("picture.jpg")),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(cached_file("audio.wav")),
    )
}

fn retrying_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture)
        .attempted_with(GenerationCost::from_nanos(123_400_000));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn audio_progress_artifacts(sound: ArtifactSlot) -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 15_400_000)),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        sound,
    )
}

fn audio_retry_slot(rejected: usize) -> ArtifactSlot {
    (0..rejected).fold(ArtifactSlot::fresh(Artifact::Sound), |slot, attempt| {
        slot.faulted(AttemptFault::failed(format!(
            "audio response {attempt} was empty"
        )))
    })
}

fn rejected_attempt() -> AttemptFault {
    AttemptFault::new(
        "topology",
        "quality score 60/100: found 1 panel region for 2 planned panels",
        Some(PathBuf::from("/tmp/attempt.jpg")),
        Some(AttemptScorecard::new(
            60,
            false,
            AttemptPenalties::new(40, 0, 0, 0),
        )),
    )
}

fn rejected_picture_artifacts_with(attempts: usize) -> CardArtifacts {
    let picture = (0..attempts).fold(ArtifactSlot::fresh(Artifact::Picture), |slot, _| {
        slot.faulted(rejected_attempt())
    });
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded_with(priced_file("meta.json", 15_400_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(priced_file("scene.json", 23_100_000)),
        picture,
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(priced_file("audio.wav", 2_500_000)),
    )
}

fn failed_artifacts() -> CardArtifacts {
    let mut picture = ArtifactSlot::fresh(Artifact::Picture);
    for nanos in [60_000_000, 150_000_000, 240_000_000, 321_000_000] {
        picture = picture.attempted_with(GenerationCost::from_nanos(nanos));
    }
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn meta_for(term: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source sentence with {term}"),
        term,
        format!("vivid hint for {term}"),
        format!("usage notes for {term}"),
        format!("Example with {term}."),
    )
}

fn labeled_meta_for(term: &str) -> CardMeta {
    meta_for(term).with_sentence_labels(SentenceLabels::new(
        Register::Casual,
        SentenceLevel::B1,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::default(),
    ))
}

fn draft(term: &str, artifacts: CardArtifacts) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for(term), None)
    .with_artifacts(artifacts)
}

fn labeled_draft(term: &str, artifacts: CardArtifacts) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(labeled_meta_for(term), None)
    .with_artifacts(artifacts)
}

fn seeded(drafts: Vec<CardDraft>) -> App {
    App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(drafts)
}

fn row_containing<'a>(rendered: &'a str, needle: &str) -> &'a str {
    rendered
        .lines()
        .find(|line| line.contains(needle))
        .expect("rendered card row must exist")
}

fn column_of(row: &str, needle: &str) -> usize {
    let offset = row.find(needle).expect("rendered card cell must exist");
    row[..offset].chars().count()
}

fn columns_of(buffer: &Buffer, needle: &str) -> Vec<u16> {
    (0..buffer.area.height)
        .filter_map(|row| {
            let rendered = row_text(buffer, row);
            rendered.find(needle).map(|start| {
                u16::try_from(rendered[..start].chars().count())
                    .expect("rendered column must fit the terminal")
            })
        })
        .collect()
}

#[test]
fn your_cards_lists_each_card_with_term_meta_preview_head_and_artifact_check_marks() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    let ctrl = rendered
        .find("[Ctrl+G] regenerate")
        .expect("card footer must show regeneration");
    let tune = rendered
        .find("[Enter] tune")
        .expect("card footer must show tuning");
    let target = rendered
        .find("whilst → Example with whilst.")
        .expect("first card head must be visible");
    let picture = rendered
        .find("✓ picture")
        .expect("first card picture step must be visible");
    assert!(
        rendered.contains("building your cards")
            && rendered.contains("2/4 ready")
            && rendered.contains("whilst")
            && rendered.contains("Example with whilst.")
            && rendered.contains("at the end")
            && rendered.contains("Example with at the end.")
            && rendered.contains("in the end")
            && rendered.contains("Example with in the end.")
            && rendered.contains("wreck")
            && rendered.contains("Example with wreck.")
            && rendered.contains("whilst → Example with whilst.")
            && rendered.contains("✓ scene")
            && rendered.contains("✓ picture")
            && rendered.contains("RU → EN")
            && rendered.contains("[Tab] next")
            && rendered.contains("[Enter] tune")
            && !rendered.contains("[Enter] toggle")
            && !rendered.contains("[Space] tune")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[R] change")
            && !rendered.contains("[D] drop")
            && !rendered.contains("ai is working…")
            && !rendered.contains("queued")
            && ctrl < tune
            && target < picture,
        "each generated card must keep its target in the head before artifact steps: {rendered}"
    );
}

#[test]
fn a_finished_batch_drops_the_jump_hint_and_keeps_plain_navigation() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
    ]);
    let rendered = flat(&app);
    assert!(
        !rendered.contains("[Tab] next") && rendered.contains("[↑↓] nav"),
        "a batch with nothing left to build still advertised the unfinished jump: {rendered}"
    );
}

#[test]
fn your_cards_done_footer_carries_one_tune_and_regenerate_hint() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", ready_artifacts()),
        draft("wreck", ready_artifacts()),
    ])
    .done_published("/tmp/cards.apkg", "/tmp/cards.pdf", "/tmp");
    let rendered = flat(&app);
    let new_cards = rendered.find("[Esc] new cards").unwrap_or(usize::MAX);
    let quit = rendered.find("[Ctrl+C] quit").unwrap_or(usize::MAX);
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("[↑↓] nav")
            && rendered.contains("[Enter] tune")
            && !rendered.contains("[Enter] toggle")
            && !rendered.contains("[Space] tune")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[R] change")
            && rendered.contains("[Esc] new cards")
            && new_cards < quit
            && !rendered.contains("[D] drop")
            && !rendered.contains("ai is working…"),
        "published footer must offer new cards before quit and omit stale toggle, Space, or drop hooks: {rendered}"
    );
}

#[test]
fn your_cards_shows_card_asset_and_total_costs_when_finished() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("whilst → Example with whilst.  $.0808")
            && rendered.contains("meta.json           1 B  $.0015")
            && rendered.contains("audio.wav           1 B  $.0100")
            && rendered.contains("scene.json          1 B  $.0020")
            && rendered.contains("picture.jpg         1 B  $.0673")
            && rendered.contains("$0.08")
            && !rendered.contains("total cost"),
        "finished cards must show detailed costs and a simplified total in dollars: {rendered}"
    );
}

#[test]
fn your_cards_footer_shows_total_cost_before_every_card_finishes() {
    let app = seeded(vec![
        draft("whilst", partial_priced_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("building your cards")
            && rendered.contains("0/2 ready")
            && rendered.contains("$0.01")
            && !rendered.contains("all done"),
        "generation footer must increment the dollar total as soon as fresh artifact costs arrive: {rendered}"
    );
}

#[test]
fn your_cards_footer_hides_total_cost_until_money_is_spent() {
    let app = seeded(vec![draft("whilst", cached_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("your cards") && !rendered.contains("$0.00"),
        "zero-cost or fully cached cards must not render a footer dollar total: {rendered}"
    );
}

#[test]
fn armed_generation_stop_makes_escape_the_only_primary_action() {
    let app = seeded(vec![draft("whilst", partial_priced_artifacts())])
        .with_generation_stop_pending(true);
    let rendered = flat(&app);
    assert!(
        rendered.contains("[Esc] again")
            && !rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[Enter] tune"),
        "armed generation stop competed with generation actions: {rendered}"
    );
}

#[test]
fn draining_generation_says_stopping_without_offering_more_work() {
    let app = seeded(vec![draft("whilst", partial_priced_artifacts())]).generation_stop_started();
    let rendered = flat(&app);
    assert!(
        rendered.contains("stopping…")
            && !rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[Enter] tune")
            && !rendered.contains("[Esc] again"),
        "draining generation looked active or offered another action: {rendered}"
    );
}

#[test]
fn partial_publish_is_a_settled_view_with_outputs_and_durable_tally() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("wreck", partial_priced_artifacts()),
    ])
    .done_published_counted("/tmp/cards.apkg", "/tmp/cards.pdf", "/tmp", 1, 1);
    let rendered = flat(&app);
    assert!(
        rendered.contains("your cards")
            && rendered.contains("some cards didn't make it")
            && rendered.contains("1/2 ready")
            && rendered.contains("1 omitted")
            && rendered.contains("APKG")
            && rendered.contains("PDF")
            && !rendered.contains("building your cards"),
        "partial publish did not render as a settled output-bearing batch: {rendered}"
    );
}

#[test]
fn partial_publish_reserves_and_links_the_same_banner_rows() {
    let terminal = Rect::new(0, 0, 120, 30);
    let cards = vec![
        draft("whilst", ready_artifacts()),
        draft("wreck", partial_priced_artifacts()),
    ];
    let building = seeded(cards.clone());
    let partial =
        seeded(cards).done_published_counted("/tmp/cards.apkg", "/tmp/cards.pdf", "/tmp", 1, 1);
    let buffer = rendered_buffer_at(&partial, terminal.width, terminal.height);
    let (column, row) = position_of(&buffer, "APKG");
    assert_eq!(
        (
            scroll_viewport(&building, terminal) - scroll_viewport(&partial, terminal),
            link_at(&partial, terminal, column, row),
        ),
        (4, Some(String::from("/tmp/cards.apkg"))),
        "partial banner rendering, scrolling, and hit geometry diverged"
    );
}

#[test]
fn your_cards_marks_cached_artifacts_next_to_the_file_metadata() {
    let app = seeded(vec![draft("whilst", cached_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("meta.json           1 B  cached"),
        "cached artifact rows must say cached in the same dim metadata slot as prices: {rendered}"
    );
}

#[test]
fn selected_card_cost_keeps_the_row_highlight_background() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    assert!(
        selected_label_highlighted(&app, "  $.0808"),
        "selected card cost must not punch a dark gap through the row highlight"
    );
}

#[test]
fn ctrl_g_on_ready_cards_requests_regeneration() {
    let app = seeded(vec![draft("whilst", ready_artifacts())]);
    let (_, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        side,
        Side::RegenerateCards,
        "Ctrl+G on ready cards must request regeneration so publish can be rebuilt"
    );
}

#[test]
fn untouched_card_shows_only_a_dim_term_with_no_step_rows() {
    let single = CardDraft::new(
        "ancient",
        "understanding for ancient",
        LanguagePair::new("en", "ru"),
    );
    let app = seeded(vec![single]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("ancient")
            && !rendered.contains("meta")
            && !rendered.contains("audio")
            && !rendered.contains("scene")
            && !rendered.contains("picture")
            && !rendered.contains("queued"),
        "untouched card must collapse to its term row alone, no artifact lines: {rendered}"
    );
}

#[test]
fn retry_state_moves_its_spent_attempt_count_to_the_card_head() {
    let app = seeded(vec![draft("in the end", retrying_artifacts())]);
    let rendered = flat(&app);
    let head = row_containing(&rendered, "in the end →");
    let picture = row_containing(&rendered, "picture");
    assert!(
        head.contains("$.1234  ↻1")
            && picture.contains("$.1234")
            && !picture.contains("retry")
            && !picture.contains("paused")
            && !picture.contains('✗')
            && rendered.contains("$0.12"),
        "retrying state duplicated its attempt count outside the card head: {rendered}"
    );
}

#[test]
fn meta_less_unmetered_retry_keeps_its_count_on_the_card_head() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).faulted(AttemptFault::failed("invalid metadata")),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    );
    let card = CardDraft::new(
        "ancient",
        "understanding for ancient",
        LanguagePair::new("en", "ru"),
    )
    .with_artifacts(artifacts);
    let rendered = flat(&seeded(vec![card]));
    let head = row_containing(&rendered, "ancient");
    assert!(
        head.contains("ancient  ↻1") && !head.contains('$'),
        "an unmetered metadata retry lost its aggregate card-head count: {rendered}"
    );
}

#[test]
fn meta_less_retry_badge_uses_the_last_available_head_cells() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).faulted(AttemptFault::failed("invalid metadata")),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    );
    let card = CardDraft::new(
        "ancient",
        "understanding for ancient",
        LanguagePair::new("en", "ru"),
    )
    .with_artifacts(artifacts);
    let exact = rendered_buffer_at(&seeded(vec![card.clone()]), 26, 20);
    let narrow = rendered_buffer_at(&seeded(vec![card]), 25, 20);
    assert_eq!(
        (
            row_text(&exact, position_of(&exact, "ancient").1).contains("ancient  ↻1"),
            row_text(&narrow, position_of(&narrow, "ancient").1).contains('↻'),
        ),
        (true, false),
        "a meta-less retry badge did not use the exact remaining head width"
    );
}

#[test]
fn narrow_retry_suffix_wraps_with_the_head_without_shifting_artifact_links() {
    let app = seeded(vec![labeled_draft(
        "interdependently",
        recovered_priced_artifacts(2),
    )]);
    let terminal = Rect::new(0, 0, 60, 30);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, head_row) = position_of(&buffer, "interdependently →");
    let (meta_column, meta_row) = position_of(&buffer, "meta.json");
    let head = row_text(&buffer, head_row);
    assert_eq!(
        (
            head.contains("$.0808  ↻2"),
            meta_row.saturating_sub(head_row),
            link_at(&app, terminal, meta_column, meta_row),
        ),
        (true, 2, Some(String::from("/tmp/meta.json"))),
        "the combined cost and retry suffix drifted from wrapped-head layout or its artifact hit map"
    );
}

#[test]
fn card_head_aggregates_spent_attempts_across_every_artifact() {
    let spent = |kind, count| {
        (0..count)
            .fold(ArtifactSlot::fresh(kind), |slot, _| {
                slot.faulted(rejected_attempt())
            })
            .succeeded()
    };
    let artifacts = CardArtifacts::from_parts(
        spent(Artifact::Meta, 1),
        spent(Artifact::Scene, 2),
        spent(Artifact::Picture, 3),
        spent(Artifact::Sound, 2),
    );
    let app = seeded(vec![draft("whilst", artifacts)]);
    let rendered = flat(&app);
    let head = row_containing(&rendered, "whilst →");
    assert!(
        head.contains("  ↻8")
            && rendered.matches("↻8").count() == 1
            && !rendered.contains("retry 1/3")
            && !rendered.contains("retry 2/3")
            && !rendered.contains("retry 3/3")
            && !rendered.contains("paused")
            && !rendered.contains("1 ✗")
            && !rendered.contains("2 ✗")
            && !rendered.contains("3 ✗"),
        "the card head failed to aggregate spent attempts across artifact rows: {rendered}"
    );
}

#[test]
fn retry_rows_keep_only_current_work_while_card_heads_keep_the_history() {
    let costs = ArtifactCosts::default()
        .charged(Artifact::Picture, GenerationCost::from_nanos(173_800_000));
    let recovered = draft("whilst", recovered_picture_artifacts());
    let retrying = draft("commodity", rejected_picture_artifacts_with(3)).with_costs(costs);
    let failed = draft("move on", rejected_picture_artifacts_with(4)).with_costs(costs);
    let app = seeded(vec![recovered, retrying, failed]).cards_running(Some((1, Artifact::Picture)));
    let rendered = flat(&app);
    let ready = row_containing(&rendered, "picture.jpg");
    let active = row_containing(&rendered, "ai is working…");
    let failed = row_containing(&rendered, "gave up");
    let recovered_head = row_containing(&rendered, "whilst →");
    let active_head = row_containing(&rendered, "commodity →");
    let failed_head = row_containing(&rendered, "move on →");
    assert!(
        recovered_head.contains("$.0673  ↻1")
            && active_head.contains("$.2148  ↻3")
            && failed_head.contains("$.2148  ↻3")
            && ready.contains("$.0673")
            && active.contains("picture")
            && !active.contains("retry")
            && !active.contains("$.1738")
            && failed.contains("$.1738")
            && !failed.contains("after")
            && !rendered.contains("paused")
            && !rendered.contains("1 ✗")
            && !rendered.contains("3 ✗")
            && !rendered.contains("4 ✗"),
        "artifact rows retained retry history instead of leaving it on their card heads: {rendered}"
    );
}

#[test]
fn failure_banner_appears_when_any_card_exhausts_its_retries() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("gave up")
            && !rendered.contains("gave up after")
            && rendered.contains("✗")
            && rendered.contains("picture")
            && rendered.contains("$.3210")
            && rendered.contains("$0.32"),
        "your cards must mark the card as `gave up` and show the ✗ on the failed step: {rendered}"
    );
}

#[test]
fn failure_footer_omits_the_duplicate_terminal_count() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    let footer = row_containing(&rendered, "step 3/3");
    assert!(
        !footer.contains('✗') && !footer.contains("gave up"),
        "failure footer repeated a terminal count already visible on its artifact row: {footer}"
    );
}

#[test]
fn enter_opens_the_editor_while_enter_and_escape_close_it() {
    let start = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("at the end", ready_artifacts()),
    ]);
    let after_down = transit(start.clone(), AppEvent::NavNext).0;
    let opened = transit(after_down.clone(), AppEvent::KeyEnter).0;
    let moved_inside = transit(opened.clone(), AppEvent::CursorLeft).0;
    let (entered_inside, enter_side) = transit(opened.clone(), AppEvent::KeyEnter);
    let closed = transit(opened.clone(), AppEvent::Cancel).0;
    assert_eq!(
        (
            (
                start.card_selected(),
                after_down.card_selected(),
                start.card_expanded(),
                opened.card_expanded(),
                moved_inside.card_expanded(),
                entered_inside.card_expanded(),
                closed.card_expanded(),
            ),
            (
                start.sentence_editor().is_none(),
                opened.sentence_editor().is_some(),
                moved_inside.sentence_editor().is_some(),
                entered_inside.sentence_editor().is_some(),
                closed.sentence_editor().is_none(),
                enter_side,
            ),
        ),
        (
            (0, 1, false, true, true, false, false),
            (true, true, true, false, true, Side::None),
        ),
        "tune controls failed to open on Enter or collapse on Enter/Escape"
    );
}

#[test]
fn right_arrow_on_a_collapsed_tunable_card_keeps_the_editor_closed() {
    let start = seeded(vec![labeled_draft("whilst", ready_artifacts())]);
    let pressed = transit(start, AppEvent::CursorRight).0;
    assert!(
        pressed.sentence_editor().is_none() && !pressed.card_expanded(),
        "a side arrow opened the card editor even though only Enter may open it"
    );
}

#[test]
fn opening_an_editor_anchors_the_card_head_when_focused_content_fits() {
    let terminal = Rect::new(0, 0, 120, 18);
    let start = seeded(
        ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(|term| draft(term, ready_artifacts()))
            .collect(),
    );
    let viewport = scroll_viewport(&start, terminal);
    let body_width = scroll_body_width(terminal);
    let selected = (0..4).fold(start, |app, _| {
        transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(viewport, body_width)
    });
    let opened = transit(selected, AppEvent::KeyEnter)
        .0
        .body_scroll_to_selection(viewport, body_width);
    let buffer = rendered_buffer_at(&opened, terminal.width, terminal.height);
    let (_, card_row) = position_of(&buffer, "five");
    assert_eq!(
        (
            card_row,
            opened.card_expanded(),
            opened.sentence_editor().is_some(),
        ),
        (3, true, true),
        "an editor that fits opened with its card head stranded near the viewport bottom"
    );
}

#[test]
fn expanding_a_card_keeps_the_focused_editor_row_visible_when_it_cannot_fit() {
    let start = seeded(
        ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(|term| draft(term, ready_artifacts()))
            .collect(),
    );
    let selected = (0..5).fold(start, |app, _| {
        transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(6, 120)
    });
    let before = selected.body_scroll();
    let expanded = transit(selected, AppEvent::KeyEnter)
        .0
        .body_scroll_to_selection(6, 120);
    assert_eq!(
        (
            expanded.body_scroll(),
            expanded.card_expanded(),
            expanded.sentence_editor().is_some(),
        ),
        (before.saturating_add(2), true, true),
        "an editor taller than the viewport did not retain focused-row scroll fallback"
    );
}

#[test]
fn expanded_card_shows_meta_preview_only_no_duplicate_artifact_pane() {
    let start = seeded(vec![draft("whilst", ready_artifacts())]);
    let expanded = transit(start, AppEvent::KeyEnter).0;
    let rendered = flat(&expanded);
    let artifact_lines = rendered.matches("scene").count();
    assert!(
        expanded.card_expanded()
            && expanded.sentence_editor().is_some()
            && rendered.contains("the phrase")
            && rendered.contains("in your language")
            && rendered.contains("a visual clue")
            && rendered.contains("word meaning")
            && rendered.contains("word pronunciation")
            && rendered.contains("phrase pronunciation")
            && rendered.contains("worth learning")
            && rendered.contains("the right context")
            && !rendered.contains("what will you recall?")
            && !rendered.contains("what does it say?")
            && !rendered.contains("what might help?")
            && !rendered.contains("what does the word mean?")
            && !rendered.contains("how do you say the word?")
            && !rendered.contains("how does the phrase sound?")
            && !rendered.contains("how useful is the word?")
            && !rendered.contains("when does it fit?")
            && !rendered.contains("target")
            && !rendered.contains("meaning · pronunciation · transcription · importance")
            && artifact_lines <= 1,
        "opening must reveal one editor-backed meta preview without duplicating the step list: {rendered}"
    );
}

#[test]
fn expanded_editor_replaces_the_chip_grid_with_focused_question_carousels() {
    let app = transit(
        seeded(vec![labeled_draft("whilst", ready_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let sound = position_of(&buffer, "how should it sound?");
    let kind = position_of(&buffer, "what kind of phrase?");
    let level = position_of(&buffer, "what's the desired level?");
    let note = position_of(&buffer, "one more thing");
    let casual = position_of(&buffer, "casual");
    let b1 = position_of(&buffer, "b1");
    let statement = position_of(&buffer, "statement");
    let fg = Color::Rgb(0xe6, 0xe3, 0xda);
    let bg = Color::Rgb(0x0e, 0x0e, 0x10);
    let dim2 = Color::Rgb(0x5a, 0x59, 0x53);
    let rule = Color::Rgb(0x2a, 0x2a, 0x2d);
    let highlight = Color::Rgb(0x1c, 0x1c, 0x1f);
    let casual_end = casual.0 + u16::try_from("casual".chars().count()).expect("casual width");
    let b1_end = b1.0 + u16::try_from("b1".chars().count()).expect("b1 width");
    let statement_end =
        statement.0 + u16::try_from("statement".chars().count()).expect("statement width");
    assert_eq!(
        (
            (
                buffer[sound].fg,
                buffer[kind].fg,
                buffer[level].fg,
                buffer[note].fg,
            ),
            (
                buffer[casual].fg,
                buffer[casual].bg,
                buffer[(casual.0 - 3, casual.1)].bg,
                buffer[(casual.0 - 2, casual.1)].bg,
                buffer[(casual_end + 1, casual.1)].bg,
                buffer[(casual_end + 2, casual.1)].bg,
                buffer[(casual_end + 3, casual.1)].bg,
                buffer[(casual_end + 4, casual.1)].bg,
                buffer[(casual_end + 5, casual.1)].bg,
                buffer[(casual_end + 6, casual.1)].bg,
            ),
            (
                (
                    buffer[(b1.0 - 5, b1.1)].bg,
                    buffer[(b1.0 - 4, b1.1)].bg,
                    buffer[(b1.0 - 3, b1.1)].bg,
                    buffer[(b1.0 - 2, b1.1)].bg,
                    buffer[(b1_end + 1, b1.1)].bg,
                    buffer[(b1_end + 2, b1.1)].bg,
                    buffer[(b1_end + 3, b1.1)].bg,
                    buffer[(b1_end + 4, b1.1)].bg,
                    buffer[(b1_end + 5, b1.1)].bg,
                    buffer[(b1_end + 6, b1.1)].bg,
                ),
                (
                    buffer[(statement_end + 1, statement.1)].bg,
                    buffer[(statement_end + 2, statement.1)].bg,
                    buffer[(statement_end + 3, statement.1)].bg,
                    buffer[(statement_end + 4, statement.1)].bg,
                    buffer[(statement_end + 5, statement.1)].bg,
                    buffer[(statement_end + 6, statement.1)].bg,
                    buffer[(statement_end + 7, statement.1)].bg,
                    buffer[(statement_end + 8, statement.1)].bg,
                ),
            ),
            (
                rendered.contains("neutral"),
                rendered.contains("formal"),
                rendered.contains("a2"),
                rendered.contains("b2"),
                rendered.contains("a1"),
                rendered.contains("c1"),
                rendered.contains("c2"),
                rendered.contains("B1"),
                rendered.contains("question"),
                rendered.contains("dialogue"),
            ),
        ),
        (
            (fg, dim2, dim2, dim2),
            (bg, fg, dim2, dim2, dim2, dim2, dim2, rule, rule, rule,),
            (
                (dim2, dim2, dim2, dim2, dim2, dim2, dim2, rule, rule, rule,),
                (dim2, dim2, dim2, rule, rule, rule, highlight, highlight),
            ),
            (
                false, false, false, false, false, false, false, false, false, false
            ),
        ),
        "expanded editor kept the academic chip grid or lost its focused gradient carousel"
    );
}

#[test]
fn moving_down_moves_the_white_question_focus_to_the_next_carousel() {
    let opened = transit(
        seeded(vec![labeled_draft("whilst", ready_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let app = transit(opened, AppEvent::NavNext).0;
    let buffer = rendered_buffer(&app);
    let sound = position_of(&buffer, "how should it sound?");
    let kind = position_of(&buffer, "what kind of phrase?");
    assert_eq!(
        (
            app.sentence_editor().map(|editor| editor.row()),
            buffer[sound].fg,
            buffer[sound].modifier.contains(Modifier::BOLD),
            buffer[kind].fg,
            buffer[kind].modifier.contains(Modifier::BOLD),
        ),
        (
            Some(LabelEditorRow::Type),
            Color::Rgb(0x5a, 0x59, 0x53),
            false,
            Color::Rgb(0xe6, 0xe3, 0xda),
            true,
        ),
        "moving down left the white question focus on the previous carousel"
    );
}

#[test]
fn collapsed_priced_artifacts_put_all_tags_on_audio_after_three_plain_gaps() {
    let terminal = Rect::new(0, 0, 120, 50);
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (cost_column, _) = position_of(&buffer, "$.0015");
    let (register_column, register_row) = position_of(&buffer, "casual");
    let (kind_column, kind_row) = position_of(&buffer, "statement");
    let (level_column, level_row) = position_of(&buffer, "b1");
    let (_, audio_row) = position_of(&buffer, "audio.wav");
    let (_, scene_row) = position_of(&buffer, "scene.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    let dim2 = Color::Rgb(0x5a, 0x59, 0x53);
    let ink = Color::Rgb(0x0e, 0x0e, 0x10);
    let tag = Color::Rgb(0x8b, 0x8a, 0x83);
    assert_eq!(
        (
            (
                rendered.contains("sentence:"),
                (meta_row..=picture_row).any(|row| row_text(&buffer, row).contains('│')),
                buffer[(cost_column, meta_row)].fg,
                head_row + 1 == meta_row,
            ),
            (
                register_column
                    == cost_column
                        + u16::try_from("$.0015".chars().count()).expect("cost width must fit")
                        + 4,
                buffer[(register_column - 2, audio_row)].symbol(),
                buffer[(register_column - 2, audio_row)].bg != tag,
            ),
            (
                (audio_row, scene_row, picture_row),
                (register_row, kind_row, level_row),
                (register_column, kind_column, level_column),
            ),
            (
                chip_has_style(&buffer, "casual", ink, tag),
                chip_has_style(&buffer, "statement", ink, tag),
                chip_has_style(&buffer, "b1", ink, tag),
                link_at(&app, terminal, picture_column, picture_row),
            ),
        ),
        (
            (false, false, dim2, true),
            (true, " ", true),
            (
                (meta_row + 1, meta_row + 2, meta_row + 3),
                (audio_row, audio_row, audio_row),
                (
                    cost_column
                        + u16::try_from("$.0015".chars().count()).expect("cost width must fit")
                        + 4,
                    register_column + 9,
                    register_column + 21,
                ),
            ),
            (true, true, true, Some(String::from("/tmp/picture.jpg"))),
        ),
        "collapsed priced artifacts retained chrome or detached the three tags from audio after three plain gaps"
    );
}

#[test]
fn audio_progress_keeps_collapsed_tags_in_one_column() {
    let ready = labeled_draft(
        "ready",
        audio_progress_artifacts(
            ArtifactSlot::fresh(Artifact::Sound)
                .succeeded_with(priced_file("audio.wav", 2_100_000)),
        ),
    );
    let active = labeled_draft(
        "active",
        audio_progress_artifacts(ArtifactSlot::fresh(Artifact::Sound)),
    );
    let retry = labeled_draft("retry", audio_progress_artifacts(audio_retry_slot(2))).with_costs(
        ArtifactCosts::default().charged(Artifact::Sound, GenerationCost::from_nanos(2_100_000)),
    );
    let recovered = labeled_draft(
        "recovered",
        audio_progress_artifacts(
            audio_retry_slot(2).succeeded_with(priced_file("audio.wav", 2_100_000)),
        ),
    );
    let app =
        seeded(vec![ready, active, retry, recovered]).cards_running(Some((1, Artifact::Sound)));
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let active_row = row_containing(&rendered, "ai is working…");
    let retry_row = rendered
        .lines()
        .find(|line| line.contains("· audio") && line.contains("$.0021"))
        .expect("inactive audio retry row must remain visible");
    let recovered_row = rendered
        .lines()
        .find(|line| line.contains("audio.wav") && line.contains("$.0021"))
        .expect("recovered audio row must remain visible");
    assert_eq!(
        (
            columns_of(&buffer, "casual"),
            column_of(active_row, "ai is working…") < column_of(active_row, "casual"),
            column_of(retry_row, "$.0021") < column_of(retry_row, "casual"),
            column_of(recovered_row, "$.0021") < column_of(recovered_row, "casual"),
            rendered.matches("↻2").count(),
            rendered.contains("retry 2/3"),
            rendered.contains("paused"),
            rendered.contains("2 ✗"),
        ),
        (
            vec![45, 45, 45, 45],
            true,
            true,
            true,
            2,
            false,
            false,
            false
        ),
        "audio progress moved its collapsed labels or left volatile status before them:\n{rendered}"
    );
}

#[test]
fn narrow_audio_retry_keeps_cost_and_hides_the_whole_tag_summary() {
    let draft = labeled_draft("retry", audio_progress_artifacts(audio_retry_slot(2))).with_costs(
        ArtifactCosts::default().charged(Artifact::Sound, GenerationCost::from_nanos(2_100_000)),
    );
    let app = seeded(vec![draft]);
    let buffer = rendered_buffer_at(&app, 70, 30);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let audio_row = (meta_row..buffer.area.height)
        .find(|row| row_text(&buffer, *row).contains("· audio"))
        .expect("inactive audio retry row must remain visible");
    let artifacts = (meta_row..=audio_row)
        .map(|row| row_text(&buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        (
            artifacts.contains("$.0021"),
            artifacts.contains("retry 2/3"),
            artifacts.contains("paused"),
            artifacts.contains("2 ✗"),
            artifacts.contains("casual"),
            artifacts.contains("statement"),
            artifacts.contains("b1"),
        ),
        (true, false, false, false, false, false, false),
        "narrow audio retry clipped its cost or sliced the compact tag summary:\n{artifacts}"
    );
}

#[test]
fn recovered_picture_keeps_collapsed_tags_at_the_audio_anchor() {
    let app = seeded(vec![labeled_draft("whilst", recovered_priced_artifacts(2))]);
    let buffer = rendered_buffer(&app);
    let (audio_cost_column, audio_row) = position_of(&buffer, "$.0100");
    let (register_column, register_row) = position_of(&buffer, "casual");
    let (_, picture_row) = position_of(&buffer, "picture.jpg");
    let picture = row_text(&buffer, picture_row);
    assert_eq!(
        (
            register_column,
            register_row,
            picture_row,
            picture.contains("2 ✗"),
        ),
        (
            audio_cost_column
                + u16::try_from("$.0100".chars().count()).expect("cost width must fit")
                + 4,
            audio_row,
            audio_row + 2,
            false,
        ),
        "a recovered picture moved collapsed sentence tags or retained a local attempt tally"
    );
}

#[test]
fn narrow_recovered_picture_uses_all_three_tag_rows_without_a_local_tally() {
    let buffer = rendered_buffer_at(
        &seeded(vec![labeled_draft("whilst", recovered_priced_artifacts(2))]),
        60,
        30,
    );
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (_, audio_row) = position_of(&buffer, "audio.wav");
    let (_, scene_row) = position_of(&buffer, "scene.json");
    let (_, picture_row) = position_of(&buffer, "picture.jpg");
    let (_, register_row) = position_of(&buffer, "casual");
    let (_, kind_row) = position_of(&buffer, "statement");
    let (_, level_row) = position_of(&buffer, "b1");
    let artifact_rows = (meta_row..=picture_row)
        .map(|row| row_text(&buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        (
            (register_row, kind_row, level_row),
            (audio_row, scene_row, picture_row),
            artifact_rows.contains("2 ✗"),
        ),
        (
            (audio_row, scene_row, picture_row),
            (meta_row + 1, meta_row + 2, meta_row + 3),
            false,
        ),
        "narrow sentence tags failed to reclaim the rows freed by the local tally"
    );
}

#[test]
fn collapsed_cached_artifacts_put_all_tags_after_the_common_status_gap() {
    let app = seeded(vec![labeled_draft("whilst", cached_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (cached_column, _) = position_of(&buffer, "cached");
    let (_, audio_row) = position_of(&buffer, "audio");
    let (_, scene_row) = position_of(&buffer, "scene");
    let (_, picture_row) = position_of(&buffer, "picture");
    let (register_column, register_row) = position_of(&buffer, "casual");
    let (_, kind_row) = position_of(&buffer, "statement");
    let (_, level_row) = position_of(&buffer, "b1");
    let dim2 = Color::Rgb(0x5a, 0x59, 0x53);
    assert_eq!(
        (
            (
                rendered.contains("sentence:"),
                (meta_row..=picture_row).any(|row| row_text(&buffer, row).contains('│')),
                buffer[(cached_column, meta_row)].fg,
            ),
            (
                register_column
                    == cached_column
                        + u16::try_from("cached".chars().count()).expect("cached width must fit")
                        + 4,
                buffer[(register_column - 2, audio_row)].symbol(),
            ),
            (
                (audio_row, scene_row, picture_row),
                (register_row, kind_row, level_row),
            ),
        ),
        (
            (false, false, dim2),
            (true, " "),
            (
                (meta_row + 1, meta_row + 2, meta_row + 3),
                (audio_row, audio_row, audio_row),
            ),
        ),
        "cached artifacts retained chrome or failed to align all collapsed tags after audio status"
    );
}

#[test]
fn narrow_collapsed_tags_wrap_only_across_the_three_plain_artifact_rows() {
    let terminal = Rect::new(0, 0, 60, 30);
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (_, audio_row) = position_of(&buffer, "audio.wav");
    let (_, scene_row) = position_of(&buffer, "scene.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    let (_, register_row) = position_of(&buffer, "casual");
    let (_, kind_row) = position_of(&buffer, "statement");
    let (_, level_row) = position_of(&buffer, "b1");
    let artifact_rows = (meta_row..=picture_row)
        .map(|row| row_text(&buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        (
            (
                rendered.contains("sentence:"),
                artifact_rows.contains('│'),
                (audio_row, scene_row, picture_row),
            ),
            (register_row, kind_row, level_row),
            link_at(&app, terminal, picture_column, picture_row),
        ),
        (
            (false, false, (meta_row + 1, meta_row + 2, meta_row + 3)),
            (audio_row, scene_row, picture_row),
            Some(String::from("/tmp/picture.jpg")),
        ),
        "narrow collapsed tags escaped the audio-to-picture rows or retained sentence chrome"
    );
}

#[test]
fn too_narrow_collapsed_card_hides_the_atomic_tag_summary() {
    let buffer = rendered_buffer_at(
        &seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        50,
        30,
    );
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (_, picture_row) = position_of(&buffer, "picture.jpg");
    let artifact_rows = (meta_row..=picture_row)
        .map(|row| row_text(&buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        (
            artifact_rows.contains('│'),
            artifact_rows.contains("casual"),
            artifact_rows.contains("b1"),
            artifact_rows.contains("statement"),
        ),
        (false, false, false, false),
        "too-narrow collapsed layout sliced an atomic tag beside the artifact status"
    );
}

#[test]
fn expanded_sentence_editor_starts_below_the_complete_artifact_block() {
    let terminal = Rect::new(0, 0, 120, 50);
    let app = transit(
        seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (meta_column, _) = position_of(&buffer, "meta.json");
    let (_, audio_row) = position_of(&buffer, "audio.wav");
    let (_, scene_row) = position_of(&buffer, "scene.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    let (sound_column, sound_row) = position_of(&buffer, "how should it sound?");
    let (kind_column, kind_row) = position_of(&buffer, "what kind of phrase?");
    let (level_column, level_row) = position_of(&buffer, "what's the desired level?");
    let (note_column, note_row) = position_of(&buffer, "one more thing");
    let section_has_no_pipe =
        (meta_row..=note_row).all(|row| !row_text(&buffer, row).contains('│'));
    assert_eq!(
        (
            (
                app.card_expanded(),
                app.sentence_editor().is_some(),
                rendered.contains("sentence:"),
                section_has_no_pipe,
                head_row + 1 == meta_row,
                sound_row == picture_row + 2
                    && row_text(&buffer, picture_row + 1).trim().is_empty(),
            ),
            (
                (audio_row, scene_row, picture_row),
                (sound_row, kind_row, level_row, note_row),
            ),
            (
                meta_column,
                sound_column,
                kind_column,
                level_column,
                note_column,
            ),
            link_at(&app, terminal, picture_column, picture_row),
        ),
        (
            (true, true, false, true, true, true),
            (
                (meta_row + 1, meta_row + 2, meta_row + 3),
                (
                    picture_row + 2,
                    picture_row + 3,
                    picture_row + 4,
                    picture_row + 5,
                ),
            ),
            (
                meta_column,
                meta_column,
                meta_column,
                meta_column,
                meta_column,
            ),
            Some(String::from("/tmp/picture.jpg")),
        ),
        "expanded editor remained beside the artifacts or retained the old sentence pane"
    );
}

#[test]
fn narrow_expanded_sentence_editor_starts_below_the_artifact_block() {
    let terminal = Rect::new(0, 0, 50, 50);
    let app = transit(
        seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let rendered = flat(&app);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (meta_column, _) = position_of(&buffer, "meta.json");
    let (_, audio_row) = position_of(&buffer, "audio.wav");
    let (_, scene_row) = position_of(&buffer, "scene.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    let (sound_column, sound_row) = position_of(&buffer, "how should it sound?");
    let (kind_column, kind_row) = position_of(&buffer, "what kind of phrase?");
    let (level_column, level_row) = position_of(&buffer, "what's the desired level?");
    let (_, statement_row) = position_of(&buffer, "statement");
    let (_, b1_row) = position_of(&buffer, "b1");
    let (note_column, note_row) = position_of(&buffer, "one more thing");
    let section_has_no_pipe =
        (meta_row..=note_row).all(|row| !row_text(&buffer, row).contains('│'));
    assert_eq!(
        (
            (
                rendered.contains("sentence:"),
                section_has_no_pipe,
                !row_text(&buffer, meta_row - 1).trim().is_empty(),
                sound_row == picture_row + 2
                    && row_text(&buffer, picture_row + 1).trim().is_empty(),
                (audio_row, scene_row, picture_row),
            ),
            (
                (sound_column, kind_column, level_column, note_column),
                (
                    sound_row,
                    kind_row,
                    statement_row,
                    level_row,
                    b1_row,
                    note_row,
                ),
            ),
            link_at(&app, terminal, picture_column, picture_row),
        ),
        (
            (
                false,
                true,
                true,
                true,
                (meta_row + 1, meta_row + 2, meta_row + 3),
            ),
            (
                (meta_column, meta_column, meta_column, meta_column),
                (
                    picture_row + 2,
                    picture_row + 4,
                    picture_row + 5,
                    picture_row + 6,
                    picture_row + 7,
                    picture_row + 8,
                ),
            ),
            Some(String::from("/tmp/picture.jpg")),
        ),
        "narrow expanded editor did not begin below the artifacts without the old sentence pane"
    );
}

#[test]
fn closed_pending_card_shows_the_staged_tags_and_bulk_regeneration_footer() {
    let staged = seeded(vec![labeled_draft("whilst", ready_artifacts())])
        .sentence_editor_opened_for_register()
        .sentence_editor_axis_chosen(2);
    let app = transit(staged, AppEvent::Cancel).0;
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let formal = position_of(&buffer, "formal");
    let ink = Color::Rgb(0x0e, 0x0e, 0x10);
    let gray = Color::Rgb(0x8b, 0x8a, 0x83);
    let white = Color::Rgb(0xe6, 0xe3, 0xda);
    assert!(
        app.cards()[0].rewrite().is_some()
            && !app.card_expanded()
            && app.sentence_editor().is_none()
            && app.cards()[0].meta().is_some()
            && app.cards()[0].artifacts().all_ready()
            && rendered.contains("formal")
            && rendered.contains("b1")
            && rendered.contains("statement")
            && rendered.contains("casual")
            && rendered.contains("· aimed for")
            && chip_has_style(&buffer, "casual", ink, gray)
            && chip_has_style(&buffer, "formal", ink, white)
            && !buffer[formal].modifier.contains(Modifier::BOLD)
            && chip_has_style(&buffer, "b1", ink, gray)
            && chip_has_style(&buffer, "statement", ink, gray)
            && rendered.contains("1 pending")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[R] change")
            && !rendered.contains("[Enter] regenerate"),
        "closed pending state hid its staged tags or invalidated the generated card too early: {rendered}"
    );
}

#[test]
fn pending_card_strikes_only_the_target_sentence_and_mutes_the_rest() {
    let app = seeded(vec![labeled_draft("whilst", ready_artifacts())])
        .sentence_editor_opened_for_register()
        .sentence_editor_axis_chosen(2);
    let buffer = rendered_buffer(&app);
    let target = matching_cells(&app, "Example with whilst.");
    let source = matching_cells(&app, "source sentence with whilst");
    let step = matching_cells(&app, "✓ meta");
    let (term_column, term_row) = position_of(&buffer, "whilst");
    let term = (0.."whilst".chars().count())
        .map(|offset| {
            let cell = &buffer[(
                term_column + u16::try_from(offset).expect("term width must fit"),
                term_row,
            )];
            (cell.fg, cell.bg, cell.modifier)
        })
        .collect::<Vec<_>>();
    let head = row_text(&buffer, term_row);
    let ink = Color::Rgb(0x0e, 0x0e, 0x10);
    let chip = Color::Rgb(0xe6, 0xe3, 0xda);
    assert_eq!(
        (
            !target.is_empty()
                && target.iter().all(|(fg, _, modifier)| {
                    *fg == Color::Rgb(0x8b, 0x8a, 0x83) && modifier.contains(Modifier::CROSSED_OUT)
                }),
            !term.is_empty()
                && term.iter().all(|(fg, _, modifier)| {
                    *fg == Color::Rgb(0x8b, 0x8a, 0x83) && !modifier.contains(Modifier::CROSSED_OUT)
                }),
            !source.is_empty()
                && source.iter().all(|(fg, _, modifier)| {
                    *fg == Color::Rgb(0x8b, 0x8a, 0x83) && !modifier.contains(Modifier::CROSSED_OUT)
                }),
            !step.is_empty()
                && step.iter().all(|(fg, _, modifier)| {
                    *fg == Color::Rgb(0x8b, 0x8a, 0x83) && !modifier.contains(Modifier::CROSSED_OUT)
                }),
            chip_has_style(&buffer, "formal", ink, chip),
            chip_has_style(&buffer, "b1", ink, chip),
            chip_has_style(&buffer, "statement", ink, chip),
            !head.contains("formal") && !head.contains("b1") && !head.contains("statement"),
        ),
        (true, true, true, true, true, true, true, true),
        "pending editor struck non-sentence content, exposed summary tags, or dimmed selected choices"
    );
}

#[test]
fn the_term_stays_gray_until_the_card_holds_its_last_artifact() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("terroir", ready_artifacts()),
        draft("wreck", partial_priced_artifacts()),
        draft("bof", failed_artifacts()),
    ]);
    let buffer = rendered_buffer(&app);
    let white = Color::Rgb(0xe6, 0xe3, 0xda);
    let gray = Color::Rgb(0x8b, 0x8a, 0x83);
    assert_eq!(
        (
            term_ink(&buffer, "terroir"),
            term_ink(&buffer, "wreck"),
            term_ink(&buffer, "bof"),
        ),
        (white, gray, gray),
        "the card term lit up before the card held all four artifacts"
    );
}

#[test]
fn active_rewrite_shows_normal_generation_without_pending_tags_or_count() {
    let baseline = SentenceLabelSelection::from_labels(
        labeled_meta_for("whilst")
            .sentence_labels()
            .expect("labeled metadata must expose its baseline"),
    );
    let active = labeled_draft("whilst", ready_artifacts())
        .staging_rewrite(
            baseline.choosing(SentenceAxis::Register, 2),
            "make it formal",
        )
        .starting_rewrite();
    let app = seeded(vec![active]).cards_running(Some((0, Artifact::Meta)));
    let rendered = flat(&app);
    assert!(
        app.cards_pending() == 0
            && !app.card_expanded()
            && app.sentence_editor().is_none()
            && app.cards()[0]
                .rewrite()
                .is_some_and(kamishibai::session::CardRewrite::started)
            && rendered.contains("ai is working…")
            && !rendered.contains("pending")
            && !rendered.contains("formal")
            && !rendered.contains("casual")
            && !rendered.contains("b1")
            && !rendered.contains("statement")
            && !rendered.contains("Example with whilst.")
            && !rendered.contains("[Enter] tune")
            && !rendered.contains("[Space] tune"),
        "active rewrite retained staged styling or exposed stale generated metadata: {rendered}"
    );
}

#[test]
fn a_long_editor_note_keeps_its_cursor_at_the_narrow_body_edge() {
    let rect = Rect::new(0, 0, 40, 24);
    let opened =
        seeded(vec![labeled_draft("whilst", ready_artifacts())]).sentence_editor_opened_for_note();
    let viewport = scroll_viewport(&opened, rect);
    let body_width = scroll_body_width(rect);
    let app = "this note is deliberately much wider than the terminal"
        .chars()
        .fold(opened, App::sentence_editor_typed)
        .body_scroll_to_selection(viewport, body_width);
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, &app)).expect("draw");
    let cursor = terminal
        .get_cursor_position()
        .expect("editor cursor must remain readable");
    let body_left = rect.x + rect.width.saturating_sub(body_width) / 2;
    assert_eq!(
        cursor.x,
        body_left + body_width.saturating_sub(1),
        "a long inline note did not clamp its cursor like the shared modal TextField"
    );
}

#[test]
fn best_effort_axes_show_the_actual_value_and_requested_target() {
    let request = SentenceLabelSelection::empty().choosing(SentenceAxis::Register, 2);
    let labels = request.reconciled(SentenceLabels::new(
        Register::Casual,
        SentenceLevel::B2,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::from_axes([SentenceAxis::Register]),
    ));
    let draft = CardDraft::new(
        "whilst",
        "understanding for whilst",
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for("whilst").with_sentence_labels(labels), None)
    .with_artifacts(ready_artifacts());
    let app = seeded(vec![draft]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let actual = position_of(&buffer, "casual");
    let aimed = position_of(&buffer, "aimed for");
    let requested = position_of(&buffer, "formal");
    let ink = Color::Rgb(0x0e, 0x0e, 0x10);
    let gray = Color::Rgb(0x8b, 0x8a, 0x83);
    let white = Color::Rgb(0xe6, 0xe3, 0xda);
    let quiet = Color::Rgb(0x5a, 0x59, 0x53);
    let background = Color::Rgb(0x0e, 0x0e, 0x10);
    assert!(
        rendered.contains("· aimed for")
            && !rendered.contains('≈')
            && rendered.contains("b2")
            && rendered.contains("statement")
            && actual.1 == aimed.1
            && aimed.1 == requested.1
            && actual.0 < aimed.0
            && aimed.0 < requested.0
            && chip_has_style(&buffer, "casual", ink, gray)
            && chip_has_style(&buffer, "formal", ink, white)
            && matching_cells(&app, "aimed for")
                .iter()
                .all(|(fg, bg, _)| *fg == quiet && *bg == background)
            && !buffer[requested].modifier.contains(Modifier::BOLD)
            && chip_has_style(&buffer, "b2", ink, gray)
            && chip_has_style(&buffer, "statement", ink, gray),
        "the compact tags hid the actual value, target, or restrained best-effort styling: {rendered}"
    );
}

#[test]
fn legacy_best_effort_axes_name_only_the_known_target() {
    let labels = SentenceLabels::new(
        Register::Archaic,
        SentenceLevel::B2,
        SentenceKind::Statement,
        AxisSet::from_axes([SentenceAxis::Register]),
        AxisSet::from_axes([SentenceAxis::Register]),
    );
    let draft = CardDraft::new(
        "whilst",
        "understanding for whilst",
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for("whilst").with_sentence_labels(labels), None)
    .with_artifacts(ready_artifacts());
    let app = seeded(vec![draft]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    assert!(
        rendered.contains("aimed for")
            && rendered.contains("archaic")
            && !rendered.contains('≈')
            && chip_has_style(
                &buffer,
                "archaic",
                Color::Rgb(0x0e, 0x0e, 0x10),
                Color::Rgb(0xe6, 0xe3, 0xda),
            ),
        "a legacy best-effort target invented an actual value or fell back to an opaque symbol: {rendered}"
    );
}

#[test]
fn open_editor_keeps_the_target_selected_and_names_the_current_actual_value() {
    let request = SentenceLabelSelection::empty().choosing(SentenceAxis::Register, 2);
    let labels = request.reconciled(SentenceLabels::new(
        Register::Casual,
        SentenceLevel::B2,
        SentenceKind::Statement,
        AxisSet::default(),
        AxisSet::from_axes([SentenceAxis::Register]),
    ));
    let draft = CardDraft::new(
        "whilst",
        "understanding for whilst",
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for("whilst").with_sentence_labels(labels), None)
    .with_artifacts(ready_artifacts());
    let app = seeded(vec![draft]).sentence_editor_opened_for_register();
    let row = row_containing(flat(&app).as_str(), "how should it sound?").to_string();
    assert!(
        row.contains("formal") && row.contains("current  casual") && !row.contains('≈'),
        "the editor confused its requested target with the current generated value: {row}"
    );
}

fn rejected_picture_artifacts(image: &str) -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture).faulted(AttemptFault::new(
        "topology",
        "quality score 60/100: found 1 panel region for 2 planned panels",
        Some(PathBuf::from(format!("/tmp/{image}"))),
        Some(AttemptScorecard::new(
            60,
            false,
            AttemptPenalties::new(40, 0, 0, 0),
        )),
    ));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn cell_of(app: &App, needle: &str) -> (u16, u16) {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            return (rendered[..start].chars().count() as u16, row);
        }
    }
    panic!("the rendered screen never showed '{needle}'");
}

#[test]
fn collapsed_tags_keep_the_meta_artifact_link_on_its_rendered_row() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let (artifact_column, artifact_row) = cell_of(&app, "meta.json");
    let (_, audio_row) = cell_of(&app, "audio.wav");
    let (_, tags_row) = cell_of(&app, "casual");
    assert_eq!(
        (
            tags_row,
            link_at(
                &app,
                Rect::new(0, 0, 120, 50),
                artifact_column,
                artifact_row,
            ),
        ),
        (audio_row, Some(String::from("/tmp/meta.json"))),
        "the collapsed audio-row tags shifted or swallowed the meta artifact link"
    );
}

#[test]
fn expanded_editor_below_artifacts_keeps_the_meta_link_on_its_rendered_row() {
    let app = transit(
        seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let (artifact_column, artifact_row) = cell_of(&app, "meta.json");
    let (_, picture_row) = cell_of(&app, "picture.jpg");
    let (editor_column, editor_row) = cell_of(&app, "how should it sound?");
    assert_eq!(
        (
            app.card_expanded(),
            app.sentence_editor().is_some(),
            editor_column,
            editor_row,
            link_at(
                &app,
                Rect::new(0, 0, 120, 50),
                artifact_column,
                artifact_row,
            ),
        ),
        (
            true,
            true,
            artifact_column,
            picture_row + 2,
            Some(String::from("/tmp/meta.json")),
        ),
        "the editor below the artifacts shifted or swallowed the meta artifact link"
    );
}

#[test]
fn narrow_sentence_tags_keep_the_downstream_picture_link_aligned() {
    let terminal = Rect::new(0, 0, 50, 30);
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    assert_eq!(
        (
            picture_row,
            link_at(&app, terminal, picture_column, picture_row),
        ),
        (meta_row + 3, Some(String::from("/tmp/picture.jpg"))),
        "wrapped sentence tags detached the downstream picture link from its rendered row"
    );
}

#[test]
fn expanded_sentence_editor_keeps_the_downstream_picture_link_aligned() {
    let terminal = Rect::new(0, 0, 120, 50);
    let app = transit(
        seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let buffer = rendered_buffer(&app);
    let (_, meta_row) = position_of(&buffer, "meta.json");
    let (picture_column, picture_row) = position_of(&buffer, "picture.jpg");
    assert_eq!(
        (
            picture_row,
            link_at(&app, terminal, picture_column, picture_row),
        ),
        (meta_row + 3, Some(String::from("/tmp/picture.jpg"))),
        "expanded sentence editor detached the downstream picture link from its rendered row"
    );
}

#[test]
fn scrolled_sentence_sections_keep_the_selected_artifact_link_aligned() {
    let terminal = Rect::new(0, 0, 120, 16);
    let selected = (0..3).fold(
        seeded(vec![
            labeled_draft("one", linked_artifacts("one")),
            labeled_draft("two", linked_artifacts("two")),
            labeled_draft("three", linked_artifacts("three")),
            labeled_draft("four", linked_artifacts("four")),
        ]),
        |app, _| transit(app, AppEvent::NavNext).0,
    );
    let viewport = scroll_viewport(&selected, terminal);
    let body_width = scroll_body_width(terminal);
    let app = selected.body_scroll_to_selection(viewport, body_width);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, term_row) = position_of(&buffer, "four");
    let artifact_row = term_row + 1;
    assert_eq!(
        (
            app.body_scroll() > 0,
            row_text(&buffer, artifact_row).contains("meta.json"),
            link_at(&app, terminal, 10, artifact_row),
        ),
        (true, true, Some(String::from("/tmp/four.json"))),
        "scrolling a card with collapsed tags detached its artifact click target from the file row"
    );
}

#[test]
fn clicking_a_rejected_frame_opens_the_archived_picture() {
    let start = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )]);
    let app = transit(start, AppEvent::KeyEnter).0;
    let (column, row) = cell_of(&app, "attempt-0001.jpg");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), column, row),
        Some(String::from("/tmp/attempt-0001.jpg")),
        "the rejected frame was drawn but its click target does not open the archived picture"
    );
}

#[test]
fn rejected_frame_name_reads_as_a_muted_link_not_as_struck_out_text() {
    let start = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )]);
    let app = transit(start, AppEvent::KeyEnter).0;
    let (column, row) = cell_of(&app, "attempt-0001.jpg");
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, &app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let name = "attempt-0001.jpg".len() as u16;
    assert!(
        (0..name).all(|offset| {
            let cell = &buffer[(column + offset, row)];
            cell.modifier.contains(Modifier::UNDERLINED)
                && !cell.modifier.contains(Modifier::CROSSED_OUT)
        }) && !buffer[(column + name, row)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "the rejected frame must read as a muted underlined link that stops at its name"
    );
}

#[test]
fn rejected_block_sits_below_the_card_behind_a_dashed_rule() {
    let start = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )]);
    let app = transit(start, AppEvent::KeyEnter).0;
    let (_, heading) = cell_of(&app, "rejected attempts");
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, &app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let rule = (0..120u16)
        .filter(|column| {
            buffer[(*column, heading - 1)]
                .modifier
                .contains(Modifier::CROSSED_OUT)
        })
        .count();
    assert!(
        rule > 20 && cell_of(&app, "the right context").1 < heading,
        "the rejected block must follow the card body behind a dashed rule"
    );
}

fn recovered_picture_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture)
        .faulted(AttemptFault::new(
            "topology",
            "quality score 60/100: found 1 panel region for 2 planned panels",
            Some(PathBuf::from("/tmp/attempt-0001.jpg")),
            Some(AttemptScorecard::new(
                60,
                false,
                AttemptPenalties::new(40, 0, 0, 0),
            )),
        ))
        .succeeded_with(priced_file("picture.jpg", 67_300_000));
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        picture,
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

#[test]
fn a_finished_artifact_keeps_attempt_history_only_in_the_card_head_and_details() {
    let collapsed = seeded(vec![draft("whilst", recovered_picture_artifacts())]);
    let rendered = flat(&collapsed);
    let head = row_containing(&rendered, "whilst →");
    let picture = row_containing(&rendered, "picture.jpg");
    let expanded = flat(&transit(collapsed, AppEvent::KeyEnter).0);
    assert!(
        head.contains("$.0673  ↻1")
            && picture.contains("✓ picture.jpg")
            && !picture.contains("1 ✗")
            && !picture.contains("1 rejected")
            && expanded.contains("rejected attempts")
            && expanded.contains("picture 1"),
        "a finished artifact duplicated or hid its attempt history: {rendered}"
    );
}

#[test]
fn the_footer_states_the_progress_of_a_live_batch_only_as_the_ready_count() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    let footer = row_containing(&rendered, "step 3/3");
    assert!(
        footer.contains("2/4 ready") && !footer.contains("building"),
        "the status bar repeated the ready count as a second progress number: {footer}"
    );
}

#[test]
fn the_footer_keeps_regenerate_when_the_census_crowds_a_narrow_bar() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let buffer = rendered_buffer_at(&app, 72, 40);
    let footer = (0..buffer.area.height)
        .map(|row| row_text(&buffer, row))
        .find(|row| row.contains("step 3/3"))
        .expect("narrow status bar must exist");
    assert!(
        footer.contains("[Ctrl+G] regenerate"),
        "a crowded status bar shed the screen's main action: {footer}"
    );
}

#[test]
fn tab_walks_only_the_unfinished_cards_and_wraps() {
    let drafts = (0..20)
        .map(|index| {
            let artifacts = if index == 7 || index == 13 {
                CardArtifacts::default()
            } else {
                ready_artifacts()
            };
            draft(&format!("word-{index:02}"), artifacts)
        })
        .collect();
    let start = seeded(drafts);
    let first = transit(start.clone(), AppEvent::NextUnfinished).0;
    let second = transit(first.clone(), AppEvent::NextUnfinished).0;
    let wrapped = transit(second.clone(), AppEvent::NextUnfinished).0;
    let backwards = transit(second.clone(), AppEvent::PreviousUnfinished).0;
    assert_eq!(
        (
            first.card_selected(),
            second.card_selected(),
            wrapped.card_selected(),
            backwards.card_selected()
        ),
        (7, 13, 7, 7),
        "the jump key walked finished cards instead of cycling the unfinished ones"
    );
}

#[test]
fn tab_is_inert_while_the_sentence_editor_is_open() {
    let start = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("wreck", CardArtifacts::default()),
    ]);
    let opened = transit(start, AppEvent::KeyEnter).0;
    let jumped = transit(opened.clone(), AppEvent::NextUnfinished).0;
    assert_eq!(
        (jumped.card_selected(), jumped.sentence_editor().is_some()),
        (opened.card_selected(), true),
        "the jump key moved the cursor out from under an open editor"
    );
}

#[test]
fn following_moves_the_selection_onto_the_card_the_engine_started() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", CardArtifacts::default()),
        draft("wreck", CardArtifacts::default()),
    ])
    .cards_running(Some((3, Artifact::Meta)));
    assert_eq!(
        (app.card_selected(), app.following_card()),
        (3, Some(3)),
        "the view stayed behind while the engine moved on to another card"
    );
}

#[test]
fn scrolling_away_stops_the_viewport_following_the_engine() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", CardArtifacts::default()),
        draft("wreck", CardArtifacts::default()),
    ])
    .body_scrolled(3, 10, 100)
    .cards_running(Some((3, Artifact::Meta)));
    assert_eq!(
        (app.card_selected(), app.following_card()),
        (0, None),
        "the viewport chased the engine after the reader had scrolled away"
    );
}

#[test]
fn an_open_card_stops_the_viewport_following_the_engine() {
    let opened = transit(
        seeded(vec![
            draft("whilst", ready_artifacts()),
            draft("wreck", CardArtifacts::default()),
        ]),
        AppEvent::KeyEnter,
    )
    .0
    .cards_running(Some((1, Artifact::Meta)));
    assert_eq!(
        opened.following_card(),
        None,
        "the viewport moved out from under a card the reader had opened"
    );
}
