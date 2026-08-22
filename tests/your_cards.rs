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

fn term_is_bold(buffer: &Buffer, term: &str) -> bool {
    let (column, row) = position_of(buffer, term);
    buffer[(column, row)].modifier.contains(Modifier::BOLD)
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

fn cached_priced_file(name: &str, nanos: u64) -> ArtifactFile {
    cached_file(name).with_cost(GenerationCost::from_nanos(nanos))
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
fn your_cards_lists_each_card_with_term_meta_preview_head_and_step_rows() {
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
        .find("✓ manga")
        .expect("first card manga row must be visible");
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
            && rendered.contains("✓ voice")
            && !rendered.contains("picture")
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
        "each generated card must keep its target in the head before its step rows: {rendered}"
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
fn step_rows_carry_their_own_incremental_costs() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    let scene = row_containing(&rendered, "scene");
    let voice = row_containing(&rendered, "voice");
    let manga = row_containing(&rendered, "manga");
    assert!(
        rendered.contains("whilst → Example with whilst.  $.0808")
            && scene.contains("$.0035")
            && voice.contains("$.0100")
            && manga.contains("$.0673")
            && rendered.contains("$0.08")
            && !rendered.contains("meta.json")
            && !rendered.contains("1 B")
            && !rendered.contains("total cost"),
        "the scene row must fold the metadata and composition spend while voice and manga keep their own: {rendered}"
    );
}

#[test]
fn an_expanded_card_keeps_the_same_three_step_rows() {
    let app = transit(
        seeded(vec![draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let rendered = flat(&app);
    assert!(
        rendered.contains("✓ scene")
            && rendered.contains("✓ voice")
            && rendered.contains("✓ manga")
            && !rendered.contains("meta.json")
            && !rendered.contains("1 B"),
        "the expanded card must keep the same three step rows without file names or sizes: {rendered}"
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
fn a_free_row_names_the_cache_hit_instead_of_leaving_its_value_blank() {
    let app = seeded(vec![draft("whilst", cached_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("✓ scene  cached") && !rendered.contains('$'),
        "a row that cost nothing because its artifact came back from the cache must say so: {rendered}"
    );
}

#[test]
fn a_priced_row_keeps_only_the_money_even_when_its_file_was_a_cache_hit() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta)
            .succeeded_with(cached_priced_file("meta.json", 1_500_000)),
        ArtifactSlot::fresh(Artifact::Scene).succeeded_with(cached_file("scene.json")),
        ArtifactSlot::fresh(Artifact::Picture).succeeded_with(cached_file("picture.jpg")),
        ArtifactSlot::fresh(Artifact::Sound).succeeded_with(cached_file("audio.wav")),
    );
    let rendered = flat(&seeded(vec![draft("whilst", artifacts)]));
    let scene = row_containing(&rendered, "scene");
    assert!(
        scene.contains("$.0015") && !scene.contains("cached"),
        "a row that recorded a price must state the money alone, never the cache note beside it: {rendered}"
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
            && !rendered.contains("manga")
            && !rendered.contains("picture")
            && !rendered.contains("queued"),
        "untouched card must collapse to its term row alone, no artifact lines: {rendered}"
    );
}

#[test]
fn retry_state_moves_its_spent_attempt_count_to_the_card_head() {
    let app = seeded(vec![draft("in the end", retrying_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, manga_row) = position_of(&buffer, "manga");
    let head = row_containing(&rendered, "in the end →");
    assert_eq!(
        (
            head.contains("$.1234  ↻1"),
            buffer[(8, manga_row)].symbol(),
            row_text(&buffer, manga_row).contains("$.1234"),
            rendered.contains("retry"),
            rendered.contains("$0.12"),
        ),
        (true, "·", true, false, true),
        "retrying state must keep the spent count on the head and a dot on its manga row: {rendered}"
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
fn narrow_retry_suffix_wraps_with_the_head_without_shifting_the_step_rows() {
    let app = seeded(vec![labeled_draft(
        "interdependently",
        recovered_priced_artifacts(2),
    )]);
    let terminal = Rect::new(0, 0, 60, 30);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, head_row) = position_of(&buffer, "interdependently →");
    let (scene_column, scene_row) = position_of(&buffer, "scene");
    let head = row_text(&buffer, head_row);
    assert_eq!(
        (
            head.contains("$.0808  ↻2"),
            scene_row.saturating_sub(head_row),
            link_at(&app, terminal, scene_column, scene_row),
        ),
        (true, 2, Some(String::from("/tmp"))),
        "the combined cost and retry suffix drifted from wrapped-head layout or the scene row hit map"
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
    let active = row_containing(&rendered, "ai is working…");
    let failed = row_containing(&rendered, "✗ manga");
    let recovered_head = row_containing(&rendered, "whilst →");
    let active_head = row_containing(&rendered, "commodity →");
    let failed_head = row_containing(&rendered, "move on →");
    assert!(
        recovered_head.contains("$.0673  ↻1")
            && active_head.contains("$.2148  ↻3")
            && failed_head.contains("$.2148  ↻3")
            && active.contains("manga")
            && !active.contains("retry")
            && !active.contains("$.1738")
            && !failed.contains("gave up")
            && failed.contains("$.1738")
            && !rendered.contains("paused")
            && !rendered.contains("1 ✗")
            && !rendered.contains("3 ✗"),
        "manga rows retained retry history instead of leaving it on their card heads: {rendered}"
    );
}

#[test]
fn failure_banner_appears_when_any_card_exhausts_its_retries() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    let manga = row_containing(&rendered, "✗ manga");
    assert!(
        manga.contains("$.3210")
            && !rendered.contains("gave up")
            && row_containing(&rendered, "wreck →").contains("$.3210  ↻3")
            && rendered.contains("$0.32"),
        "your cards must mark the failed manga row with a bare ✗ and its own cost: {rendered}"
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
fn enter_opens_the_editor_while_enter_closes_and_escape_peels_layers() {
    let start = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("at the end", ready_artifacts()),
    ]);
    let after_down = transit(start.clone(), AppEvent::NavNext).0;
    let opened = transit(after_down.clone(), AppEvent::KeyEnter).0;
    let moved_inside = transit(opened.clone(), AppEvent::CursorLeft).0;
    let (entered_inside, enter_side) = transit(opened.clone(), AppEvent::KeyEnter);
    let parked = transit(opened.clone(), AppEvent::Cancel).0;
    let collapsed = transit(parked.clone(), AppEvent::Cancel).0;
    assert_eq!(
        (
            (
                start.card_selected(),
                after_down.card_selected(),
                start.card_expanded(),
                opened.card_expanded(),
                moved_inside.card_expanded(),
                entered_inside.card_expanded(),
                parked.card_expanded(),
                collapsed.card_expanded(),
            ),
            (
                start.sentence_editor().is_none(),
                opened.sentence_editor().is_some(),
                moved_inside.sentence_editor().is_some(),
                entered_inside.sentence_editor().is_some(),
                parked.sentence_editor().is_none(),
                enter_side,
            ),
        ),
        (
            (0, 1, false, true, true, false, true, false),
            (true, true, true, false, true, Side::None),
        ),
        "tune controls failed to open on Enter, close on Enter, or peel editor then expansion on Escape"
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
    let artifact_lines = rendered.matches("manga").count();
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
fn the_tag_summary_sits_on_the_voice_row_at_the_fixed_column() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    let (register_column, register_row) = position_of(&buffer, "casual");
    let (kind_column, kind_row) = position_of(&buffer, "statement");
    let (level_column, level_row) = position_of(&buffer, "b1");
    let ink = Color::Rgb(0x0e, 0x0e, 0x10);
    let tag = Color::Rgb(0x8b, 0x8a, 0x83);
    assert_eq!(
        (
            (register_row, kind_row, level_row),
            (register_column, kind_column, level_column),
            rendered.contains("sentence:"),
            chip_has_style(&buffer, "casual", ink, tag),
            chip_has_style(&buffer, "statement", ink, tag),
            chip_has_style(&buffer, "b1", ink, tag),
        ),
        (
            (head_row + 2, head_row + 2, head_row + 2),
            (26, 35, 47),
            false,
            true,
            true,
            true,
        ),
        "the collapsed tag summary must sit on the voice row at the fixed tag column"
    );
}

#[test]
fn audio_progress_keeps_one_tag_column_and_hides_only_the_busy_rows_summary() {
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
    let (_, ready_head) = position_of(&buffer, "ready →");
    let (_, retry_head) = position_of(&buffer, "retry →");
    let (_, recovered_head) = position_of(&buffer, "recovered →");
    assert_eq!(
        (
            columns_of(&buffer, "casual"),
            active_row.contains("casual"),
            buffer[(8, ready_head + 2)].symbol(),
            buffer[(8, retry_head + 2)].symbol(),
            buffer[(8, recovered_head + 2)].symbol(),
            rendered.matches("↻2").count(),
            rendered.matches("$.0021").count(),
        ),
        (vec![26, 26, 26], false, "✓", "·", "✓", 2, 3),
        "audio progress must keep one tag column on the idle voice rows and drop the busy row's summary:\n{rendered}"
    );
}

#[test]
fn a_recovered_picture_keeps_a_plain_check_and_its_history_on_the_head() {
    let app = seeded(vec![labeled_draft("whilst", recovered_priced_artifacts(2))]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    assert_eq!(
        (
            buffer[(8, head_row + 3)].symbol(),
            row_containing(&rendered, "whilst →").contains("↻2"),
            rendered.contains("2 ✗"),
        ),
        ("✓", true, false),
        "a recovered picture must render a plain ready manga row while the head keeps the history"
    );
}

#[test]
fn collapsed_cached_artifacts_keep_tags_beside_their_cache_note() {
    let app = seeded(vec![labeled_draft("whilst", cached_artifacts())]);
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    let (register_column, register_row) = position_of(&buffer, "casual");
    assert_eq!(
        (
            rendered.matches("cached").count(),
            rendered.contains("meta.json"),
            (register_column, register_row),
            buffer[(10, head_row + 1)].symbol(),
        ),
        (3, false, (26, head_row + 2), "s"),
        "cached artifacts must collapse to ready rows that name the cache hit and keep the tag column"
    );
}

#[test]
fn too_narrow_collapsed_card_hides_the_atomic_tag_summary() {
    let buffer = rendered_buffer_at(
        &seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        38,
        30,
    );
    let (_, scene_row) = position_of(&buffer, "scene");
    let rows = (scene_row..scene_row + 3)
        .map(|row| row_text(&buffer, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        (
            rows.contains("scene"),
            rows.contains("voice"),
            rows.contains("manga"),
            rows.contains("casual"),
            rows.contains("statement"),
            rows.contains("b1"),
        ),
        (true, true, true, false, false, false),
        "too-narrow collapsed layout must keep the step rows and hide the whole tag summary"
    );
}

#[test]
fn ready_step_labels_render_as_underlined_links() {
    let app = seeded(vec![draft("whilst", priced_artifacts())]);
    let scene = matching_cells(&app, "scene");
    assert!(
        !scene.is_empty()
            && scene
                .iter()
                .all(|(_, _, modifier)| modifier.contains(Modifier::UNDERLINED)),
        "a ready row label with a known target must render as an underlined link"
    );
}

#[test]
fn only_started_rows_render_and_queued_rows_stay_hidden() {
    let app = seeded(vec![draft("whilst", partial_priced_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("✓ scene")
            && !rendered.contains("voice")
            && !rendered.contains("manga")
            && !rendered.contains("queued"),
        "rows that never started must stay hidden while the scene row appears first: {rendered}"
    );
}

#[test]
fn scene_and_picture_work_share_one_spinner_on_the_manga_row() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    );
    let app = seeded(vec![draft("whilst", artifacts)]).cards_running(Some((0, Artifact::Scene)));
    let rendered = flat(&app);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    let manga = row_text(&buffer, head_row + 3);
    assert_eq!(
        (
            buffer[(8, head_row + 3)].symbol() != " ",
            manga.contains("manga"),
            manga.contains("ai is working…"),
            rendered.contains("picture"),
        ),
        (true, true, true, false),
        "scene and picture work must spin on the single manga row with the working phrase beside it"
    );
}

#[test]
fn a_ready_scene_with_a_queued_picture_keeps_a_dim_dot_trace() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    );
    let app = seeded(vec![draft("whilst", artifacts)]);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    assert_eq!(
        buffer[(8, head_row + 3)].symbol(),
        "·",
        "a paid ready scene with its picture still owed must leave a dim dot on the manga row"
    );
}

#[test]
fn a_discarded_artifact_renders_a_dim_slash_on_its_manga_row() {
    let artifacts = CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).discard(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    );
    let app = seeded(vec![draft("whilst", artifacts)]);
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst →");
    assert_eq!(
        buffer[(8, head_row + 3)].symbol(),
        "⊘",
        "a discarded artifact must keep its dim slash on the manga row"
    );
}

#[test]
fn a_staged_rewrite_mutes_the_step_rows_and_keeps_the_staged_tags_visible() {
    let staged = seeded(vec![labeled_draft("whilst", ready_artifacts())])
        .sentence_editor_opened_for_register()
        .sentence_editor_axis_chosen(2);
    let parked = transit(staged, AppEvent::Cancel).0;
    let app = transit(parked, AppEvent::Cancel).0;
    let buffer = rendered_buffer(&app);
    let (_, head_row) = position_of(&buffer, "whilst");
    let gray = Color::Rgb(0x8b, 0x8a, 0x83);
    let scene_label = &buffer[(10, head_row + 1)];
    assert_eq!(
        (
            scene_label.symbol(),
            scene_label.fg,
            row_text(&buffer, head_row + 2).contains("formal"),
        ),
        ("s", gray, true),
        "a staged rewrite must mute the step rows while the staged tags stay visible"
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
    let (scene_column, scene_row) = position_of(&buffer, "scene");
    let (_, voice_row) = position_of(&buffer, "voice");
    let (manga_column, manga_row) = position_of(&buffer, "manga");
    let (sound_column, sound_row) = position_of(&buffer, "how should it sound?");
    let (kind_column, kind_row) = position_of(&buffer, "what kind of phrase?");
    let (level_column, level_row) = position_of(&buffer, "what's the desired level?");
    let (note_column, note_row) = position_of(&buffer, "one more thing");
    let section_has_no_pipe =
        (scene_row..=note_row).all(|row| !row_text(&buffer, row).contains('│'));
    assert_eq!(
        (
            (
                app.card_expanded(),
                app.sentence_editor().is_some(),
                rendered.contains("sentence:"),
                section_has_no_pipe,
                head_row + 1 == scene_row,
                sound_row == manga_row + 2 && row_text(&buffer, manga_row + 1).trim().is_empty(),
            ),
            (
                (voice_row, manga_row),
                (sound_row, kind_row, level_row, note_row),
            ),
            (sound_column, kind_column, level_column, note_column),
            link_at(&app, terminal, manga_column, manga_row),
        ),
        (
            (true, true, false, true, true, true),
            (
                (scene_row + 1, scene_row + 2),
                (manga_row + 2, manga_row + 3, manga_row + 4, manga_row + 5),
            ),
            (scene_column, scene_column, scene_column, scene_column),
            Some(String::from("/tmp/picture.jpg")),
        ),
        "expanded editor remained beside the step rows or retained the old sentence pane"
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
    let (scene_column, scene_row) = position_of(&buffer, "scene");
    let (_, voice_row) = position_of(&buffer, "voice");
    let (manga_column, manga_row) = position_of(&buffer, "manga");
    let (sound_column, sound_row) = position_of(&buffer, "how should it sound?");
    let (kind_column, kind_row) = position_of(&buffer, "what kind of phrase?");
    let (level_column, level_row) = position_of(&buffer, "what's the desired level?");
    let (_, statement_row) = position_of(&buffer, "statement");
    let (_, b1_row) = position_of(&buffer, "b1");
    let (note_column, note_row) = position_of(&buffer, "one more thing");
    let section_has_no_pipe =
        (scene_row..=note_row).all(|row| !row_text(&buffer, row).contains('│'));
    assert_eq!(
        (
            (
                rendered.contains("sentence:"),
                section_has_no_pipe,
                !row_text(&buffer, scene_row - 1).trim().is_empty(),
                sound_row == manga_row + 2 && row_text(&buffer, manga_row + 1).trim().is_empty(),
                (voice_row, manga_row),
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
            link_at(&app, terminal, manga_column, manga_row),
        ),
        (
            (false, true, true, true, (scene_row + 1, scene_row + 2)),
            (
                (scene_column, scene_column, scene_column, scene_column,),
                (
                    manga_row + 2,
                    manga_row + 4,
                    manga_row + 5,
                    manga_row + 6,
                    manga_row + 7,
                    manga_row + 8,
                ),
            ),
            Some(String::from("/tmp/picture.jpg")),
        ),
        "narrow expanded editor did not begin below the step rows without the old sentence pane"
    );
}

#[test]
fn closed_pending_card_shows_the_staged_tags_and_bulk_regeneration_footer() {
    let staged = seeded(vec![labeled_draft("whilst", ready_artifacts())])
        .sentence_editor_opened_for_register()
        .sentence_editor_axis_chosen(2);
    let parked = transit(staged, AppEvent::Cancel).0;
    let app = transit(parked, AppEvent::Cancel).0;
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
    let step = matching_cells(&app, "✓ scene");
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
fn a_built_card_turns_its_term_bold_and_lights_its_sentence() {
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
            (
                term_ink(&buffer, "terroir"),
                term_ink(&buffer, "wreck"),
                term_ink(&buffer, "bof"),
            ),
            (
                term_is_bold(&buffer, "terroir"),
                term_is_bold(&buffer, "wreck"),
                term_is_bold(&buffer, "bof"),
            ),
            (
                term_ink(&buffer, "Example with terroir."),
                term_ink(&buffer, "Example with wreck."),
            ),
        ),
        ((white, gray, gray), (true, false, false), (white, gray)),
        "brightness and weight must read as built rather than as focus"
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
fn clicking_the_scene_label_row_targets_the_cards_cache_folder() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let (scene_column, scene_row) = cell_of(&app, "scene");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), scene_column, scene_row),
        Some(String::from("/tmp")),
        "the scene row must target the folder holding the card's assets"
    );
}

#[test]
fn clicking_the_voice_label_opens_the_audio_file() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let (voice_column, voice_row) = cell_of(&app, "voice");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), voice_column, voice_row),
        Some(String::from("/tmp/audio.wav")),
        "the voice row must target the generated audio file"
    );
}

#[test]
fn clicking_the_manga_label_opens_the_rendered_page() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let (manga_column, manga_row) = cell_of(&app, "manga");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), manga_column, manga_row),
        Some(String::from("/tmp/picture.jpg")),
        "the manga row must target the rendered picture"
    );
}

#[test]
fn the_cell_after_a_row_label_is_not_a_link() {
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let (_, scene_row) = cell_of(&app, "scene");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), 15, scene_row),
        None,
        "the gap after a row label must stay inert"
    );
}

#[test]
fn expanded_editor_below_step_rows_keeps_the_scene_link_on_its_rendered_row() {
    let app = transit(
        seeded(vec![labeled_draft("whilst", priced_artifacts())]),
        AppEvent::KeyEnter,
    )
    .0;
    let (scene_column, scene_row) = cell_of(&app, "scene");
    let (_, manga_row) = cell_of(&app, "manga");
    let (editor_column, editor_row) = cell_of(&app, "how should it sound?");
    assert_eq!(
        (
            app.card_expanded(),
            app.sentence_editor().is_some(),
            editor_column,
            editor_row,
            link_at(&app, Rect::new(0, 0, 120, 50), scene_column, scene_row),
        ),
        (
            true,
            true,
            scene_column,
            manga_row + 2,
            Some(String::from("/tmp")),
        ),
        "the editor below the step rows shifted or swallowed the scene link"
    );
}

#[test]
fn a_narrow_card_keeps_the_manga_label_clickable() {
    let terminal = Rect::new(0, 0, 45, 30);
    let app = seeded(vec![labeled_draft("whilst", priced_artifacts())]);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (manga_column, manga_row) = position_of(&buffer, "manga");
    assert_eq!(
        link_at(&app, terminal, manga_column, manga_row),
        Some(String::from("/tmp/picture.jpg")),
        "the narrow card lost the manga label click target"
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
    let (_, scene_row) = position_of(&buffer, "scene");
    let (manga_column, manga_row) = position_of(&buffer, "manga");
    assert_eq!(
        (manga_row, link_at(&app, terminal, manga_column, manga_row),),
        (scene_row + 2, Some(String::from("/tmp/picture.jpg"))),
        "expanded sentence editor detached the downstream manga link from its rendered row"
    );
}

#[test]
fn scrolled_cards_keep_the_selected_scene_link_aligned() {
    let terminal = Rect::new(0, 0, 120, 10);
    let selected = (0..5).fold(
        seeded(vec![
            labeled_draft("one", linked_artifacts("one")),
            labeled_draft("two", linked_artifacts("two")),
            labeled_draft("three", linked_artifacts("three")),
            labeled_draft("four", linked_artifacts("four")),
            labeled_draft("five", linked_artifacts("five")),
            labeled_draft("six", linked_artifacts("six")),
        ]),
        |app, _| transit(app, AppEvent::NavNext).0,
    );
    let viewport = scroll_viewport(&selected, terminal);
    let body_width = scroll_body_width(terminal);
    let app = selected.body_scroll_to_selection(viewport, body_width);
    let buffer = rendered_buffer_at(&app, terminal.width, terminal.height);
    let (_, term_row) = position_of(&buffer, "six →");
    let scene_row = term_row + 1;
    assert_eq!(
        (
            app.body_scroll() > 0,
            row_text(&buffer, scene_row).contains("scene"),
            link_at(&app, terminal, 11, scene_row),
        ),
        (true, true, Some(String::from("/tmp"))),
        "scrolling a collapsed card detached its scene click target from its row"
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
    let manga = row_containing(&rendered, "manga");
    let expanded = flat(&transit(collapsed, AppEvent::KeyEnter).0);
    assert!(
        head.contains("$.0673  ↻1")
            && manga.contains("$.0673")
            && !rendered.contains("picture.jpg")
            && !rendered.contains("1 ✗")
            && !rendered.contains("1 rejected")
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
fn tab_parks_the_editor_and_jumps_to_the_next_unfinished_card() {
    let start = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("wreck", CardArtifacts::default()),
    ]);
    let opened = transit(start, AppEvent::KeyEnter).0;
    let jumped = transit(opened.clone(), AppEvent::NextUnfinished).0;
    assert_eq!(
        (
            jumped.card_selected(),
            jumped.sentence_editor().is_none(),
            jumped.card_expanded_at(0),
        ),
        (1, true, true),
        "the jump key failed to park the open editor and land on the unfinished card"
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

#[test]
fn down_from_the_note_row_parks_the_editor_on_the_next_card_head() {
    let opened = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("at the end", ready_artifacts()),
    ])
    .sentence_editor_opened_for_note();
    let walked = transit(opened, AppEvent::NavNext).0;
    assert_eq!(
        (
            walked.card_selected(),
            walked.sentence_editor().is_none(),
            walked.card_expanded_at(0),
        ),
        (1, true, true),
        "walking below the note row failed to park the editor on the next card head"
    );
}

#[test]
fn up_from_the_register_row_parks_the_editor_on_this_card_head() {
    let opened = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("at the end", ready_artifacts()),
    ])
    .sentence_editor_opened_for_register();
    let walked = transit(opened, AppEvent::NavPrev).0;
    assert_eq!(
        (
            walked.card_selected(),
            walked.sentence_editor().is_none(),
            walked.card_expanded_at(0),
        ),
        (0, true, true),
        "walking above the register row failed to park the editor on its own card head"
    );
}

#[test]
fn walking_back_onto_a_parked_card_reenters_its_editor_at_the_note_row() {
    let parked = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        labeled_draft("at the end", ready_artifacts()),
    ])
    .sentence_editor_opened_for_note();
    let below = transit(parked, AppEvent::NavNext).0;
    let back = transit(below, AppEvent::NavPrev).0;
    assert_eq!(
        (
            back.card_selected(),
            back.sentence_editor()
                .map(kamishibai::tui::SentenceLabelsEditor::row),
            back.card_expanded_at(0),
        ),
        (0, Some(LabelEditorRow::Note), true),
        "walking back onto a parked card stopped at the head instead of reentering its tune rows"
    );
}

#[test]
fn down_from_a_parked_expanded_head_enters_its_editor_on_register() {
    let parked = transit(
        seeded(vec![
            labeled_draft("whilst", ready_artifacts()),
            labeled_draft("at the end", ready_artifacts()),
        ])
        .sentence_editor_opened_for_register(),
        AppEvent::Cancel,
    )
    .0;
    let entered = transit(parked, AppEvent::NavNext).0;
    assert_eq!(
        (
            entered.card_selected(),
            entered
                .sentence_editor()
                .map(kamishibai::tui::SentenceLabelsEditor::row),
        ),
        (0, Some(LabelEditorRow::Register)),
        "walking down from an expanded head skipped its own tune rows"
    );
}

#[test]
fn down_at_the_last_note_row_stays_inside_the_editor() {
    let opened =
        seeded(vec![labeled_draft("whilst", ready_artifacts())]).sentence_editor_opened_for_note();
    let pressed = transit(opened, AppEvent::NavNext).0;
    assert!(
        pressed
            .sentence_editor()
            .is_some_and(|editor| editor.row() == LabelEditorRow::Note),
        "the walk fell off the bottom of the last card and cycled its editor"
    );
}

#[test]
fn an_expanded_card_with_an_active_rewrite_keeps_its_previous_meta_visible() {
    let rewriting = labeled_draft("whilst", ready_artifacts())
        .staging_rewrite(
            kamishibai::session::SentenceLabelSelection::empty()
                .choosing(SentenceAxis::Register, 2),
            "make it formal",
        )
        .starting_rewrite();
    let app = seeded(vec![rewriting]).cards_running(Some((0, Artifact::Meta)));
    let opened = transit(app, AppEvent::KeyEnter).0;
    let rendered = flat(&opened);
    assert!(
        rendered.contains("the phrase")
            && rendered.contains("Example with whilst.")
            && !rendered.contains("meta not generated yet"),
        "expanding a card mid-rewrite hid its previous meta instead of showing it struck: {rendered}"
    );
}

#[test]
fn c_collapses_all_from_a_carousel_row_of_the_open_editor() {
    let opened = seeded(vec![labeled_draft("whilst", ready_artifacts())])
        .sentence_editor_opened_for_register();
    let collapsed = transit(opened, AppEvent::KeyChar('c')).0;
    assert!(
        !collapsed.any_card_expanded() && collapsed.sentence_editor().is_none(),
        "collapse all was swallowed by a carousel row that owns no typing"
    );
}

#[test]
fn enter_on_a_parked_tunable_head_reopens_the_editor() {
    let parked = transit(
        seeded(vec![labeled_draft("whilst", ready_artifacts())])
            .sentence_editor_opened_for_register(),
        AppEvent::Cancel,
    )
    .0;
    let reopened = transit(parked, AppEvent::KeyEnter).0;
    assert!(
        reopened.sentence_editor().is_some() && reopened.card_expanded(),
        "Enter on a parked tunable head collapsed the card instead of reopening its editor"
    );
}

#[test]
fn a_parked_card_renders_its_meta_preview_with_the_compact_tag_summary() {
    let parked = transit(
        seeded(vec![labeled_draft("whilst", ready_artifacts())])
            .sentence_editor_opened_for_register(),
        AppEvent::Cancel,
    )
    .0;
    let rendered = flat(&parked);
    assert!(
        parked.card_expanded()
            && parked.sentence_editor().is_none()
            && rendered.contains("casual")
            && rendered.contains("b1")
            && rendered.contains("statement")
            && rendered.contains("the phrase")
            && !rendered.contains("how should it sound?"),
        "a parked card lost its tag summary or meta preview, or kept unfocused carousels: {rendered}"
    );
}

#[test]
fn c_collapses_every_parked_card_on_your_cards() {
    let first_parked = transit(
        seeded(vec![
            labeled_draft("whilst", ready_artifacts()),
            labeled_draft("at the end", ready_artifacts()),
        ])
        .sentence_editor_opened_for_note(),
        AppEvent::NavNext,
    )
    .0;
    let second_open = transit(first_parked, AppEvent::KeyEnter).0;
    let second_parked = transit(second_open, AppEvent::Cancel).0;
    let collapsed = transit(second_parked, AppEvent::KeyChar('c')).0;
    assert!(
        !collapsed.any_card_expanded(),
        "collapse all left a parked card expanded"
    );
}

#[test]
fn c_is_swallowed_while_the_note_row_owns_typing() {
    let opened =
        seeded(vec![labeled_draft("whilst", ready_artifacts())]).sentence_editor_opened_for_note();
    let typed = transit(opened, AppEvent::KeyChar('c')).0;
    assert!(
        typed
            .sentence_editor()
            .is_some_and(|editor| editor.note().value().contains('c'))
            && typed.card_expanded(),
        "a printable key leaked through the note row into collapse all"
    );
}

#[test]
fn c_expands_every_card_when_none_is_expanded() {
    let collapsed = seeded(vec![
        labeled_draft("whilst", ready_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let expanded = transit(collapsed, AppEvent::KeyChar('c')).0;
    assert!(
        expanded.card_expanded_at(0)
            && expanded.card_expanded_at(1)
            && expanded.sentence_editor().is_none(),
        "the collapse toggle failed to expand every card from the fully collapsed view"
    );
}
