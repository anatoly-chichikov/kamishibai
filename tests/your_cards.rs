//! Integration render tests for the `Your cards` screen (04-your-cards.png)
//! and its two inline variants (retry, failure).

use std::path::PathBuf;

use kamishibai::session::{
    Artifact, ArtifactCosts, ArtifactFile, ArtifactSlot, AttemptFault, CardArtifacts, CardDraft,
    CardMeta, GenerationCost, LanguagePair,
};
use kamishibai::tui::{App, AppEvent, Screen, Side, draw, link_at, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
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

fn rejected_attempt() -> AttemptFault {
    AttemptFault::new(
        "border",
        "White border missing on: bottom",
        Some(PathBuf::from("/tmp/attempt.jpg")),
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

fn draft(term: &str, artifacts: CardArtifacts) -> CardDraft {
    CardDraft::new(
        term,
        format!("understanding for {term}"),
        LanguagePair::new("en", "ru"),
    )
    .with_meta(meta_for(term), None)
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

#[test]
fn your_cards_lists_each_card_with_term_meta_preview_head_and_artifact_check_marks() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", retrying_artifacts()),
        draft("wreck", CardArtifacts::default()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("building your cards")
            && rendered.contains("2/4 ready")
            && rendered.contains("whilst → Example with whilst.")
            && rendered.contains("at the end → Example with at the end.")
            && rendered.contains("in the end → Example with in the end.")
            && rendered.contains("wreck → Example with wreck.")
            && rendered.contains("✓ scene")
            && rendered.contains("✓ picture")
            && rendered.contains("RU → EN")
            && rendered.contains("[↑↓] nav")
            && rendered.contains("[Enter/→] toggle")
            && rendered.contains("[R] change")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("[D] drop")
            && !rendered.contains("ai is working…")
            && !rendered.contains("queued"),
        "each generated card must reveal its meta sentence on the head row right after the term: {rendered}"
    );
}

#[test]
fn your_cards_done_footer_carries_expand_change_and_regenerate_hints() {
    let app = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
        draft("in the end", ready_artifacts()),
        draft("wreck", ready_artifacts()),
    ]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("your cards")
            && rendered.contains("all done")
            && rendered.contains("[↑↓] nav")
            && rendered.contains("[Enter/→] toggle")
            && rendered.contains("[R] change")
            && rendered.contains("[Ctrl+G] regenerate")
            && !rendered.contains("new batch")
            && !rendered.contains("[D] drop")
            && !rendered.contains("ai is working…"),
        "all-done footer must offer toggle/change/regenerate hints and no new-batch or drop hooks: {rendered}"
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
        Side::RegenerateCurrent,
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
fn retry_state_shows_retrying_count_inline_inside_the_card_row() {
    let app = seeded(vec![draft("in the end", retrying_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("retry 1/3") && rendered.contains("$.1234") && rendered.contains("$0.12"),
        "retrying state must be rendered inline without leaving the your cards screen: {rendered}"
    );
}

#[test]
fn artifact_prices_stay_aligned_and_rejected_counts_follow_them() {
    let costs = ArtifactCosts::default()
        .charged(Artifact::Picture, GenerationCost::from_nanos(173_800_000));
    let recovered = draft("whilst", recovered_picture_artifacts());
    let retrying = draft("commodity", rejected_picture_artifacts_with(3)).with_costs(costs);
    let failed = draft("move on", rejected_picture_artifacts_with(4)).with_costs(costs);
    let app = seeded(vec![recovered, retrying, failed]).cards_running(Some((1, Artifact::Picture)));
    let rendered = flat(&app);
    let ready = row_containing(&rendered, "picture.jpg");
    let retry = row_containing(&rendered, "retry 3/3");
    let failed = row_containing(&rendered, "gave up after 3 retries");
    let ready_price = column_of(ready, "$.0673");
    let ready_rejected = column_of(ready, "1 rejected");
    let retry_price = column_of(retry, "$.1738");
    let retry_rejected = column_of(retry, "3 rejected");
    let failed_price = column_of(failed, "$.1738");
    let failed_rejected = column_of(failed, "4 rejected");
    assert_eq!(
        (
            retry_price,
            ready_price < ready_rejected,
            retry_price < retry_rejected,
            failed_price < failed_rejected,
        ),
        (ready_price, true, true, true),
        "artifact prices drifted between ready and retry rows or put rejected before the price: {rendered}"
    );
}

#[test]
fn failure_banner_appears_when_any_card_exhausts_its_retries() {
    let app = seeded(vec![draft("wreck", failed_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("gave up")
            && rendered.contains("✗")
            && rendered.contains("picture")
            && rendered.contains("$.3210")
            && rendered.contains("$0.32"),
        "your cards must mark the card as `gave up` and show the ✗ on the failed step: {rendered}"
    );
}

#[test]
fn vertical_arrows_navigate_and_horizontal_arrows_or_enter_toggle_the_focused_card() {
    let start = seeded(vec![
        draft("whilst", ready_artifacts()),
        draft("at the end", ready_artifacts()),
    ]);
    let after_down = transit(start.clone(), AppEvent::NavNext).0;
    let expanded = transit(after_down.clone(), AppEvent::CursorRight).0;
    let closed = transit(expanded.clone(), AppEvent::CursorLeft).0;
    let entered = transit(after_down.clone(), AppEvent::KeyEnter).0;
    assert_eq!(
        (
            start.card_selected(),
            after_down.card_selected(),
            start.card_expanded(),
            expanded.card_expanded(),
            closed.card_expanded(),
            entered.card_expanded(),
        ),
        (0, 1, false, true, false, true),
        "vertical arrows must move the cursor while horizontal arrows and Enter toggle the focused card"
    );
}

#[test]
fn expanding_a_card_keeps_the_existing_scroll_position() {
    let start = seeded(
        ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(|term| draft(term, ready_artifacts()))
            .collect(),
    );
    let selected = (0..5).fold(start, |app, _| {
        transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(8, 120)
    });
    let before = selected.body_scroll();
    let expanded = transit(selected, AppEvent::KeyEnter)
        .0
        .body_scroll_to_selection(8, 120);
    assert_eq!(
        expanded.body_scroll(),
        before,
        "expanding a visible card must not jump the scroll to the bottom of the detail pane"
    );
}

#[test]
fn expanded_card_shows_meta_preview_only_no_duplicate_artifact_pane() {
    let start = seeded(vec![draft("whilst", ready_artifacts())]);
    let expanded = transit(start, AppEvent::KeyEnter).0;
    let rendered = flat(&expanded);
    let artifact_lines = rendered.matches("scene").count();
    assert!(
        rendered.contains("target")
            && rendered.contains("source")
            && rendered.contains("hint")
            && rendered.contains("meaning")
            && artifact_lines <= 1,
        "expanded row must reveal the meta preview without duplicating the step list: {rendered}"
    );
}

fn rejected_picture_artifacts(image: &str) -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture).faulted(AttemptFault::new(
        "border",
        "White border missing on: bottom",
        Some(PathBuf::from(format!("/tmp/{image}"))),
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
fn clicking_a_rejected_frame_opens_the_archived_picture() {
    let app = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )])
    .card_revealed(0);
    let (column, row) = cell_of(&app, "attempt-0001.jpg");
    assert_eq!(
        link_at(&app, Rect::new(0, 0, 120, 50), column, row),
        Some(String::from("/tmp/attempt-0001.jpg")),
        "the rejected frame was drawn but its click target does not open the archived picture"
    );
}

#[test]
fn rejected_frame_name_reads_as_a_muted_link_not_as_struck_out_text() {
    let app = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )])
    .card_revealed(0);
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
    let app = seeded(vec![draft(
        "whilst",
        rejected_picture_artifacts("attempt-0001.jpg"),
    )])
    .card_revealed(0);
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
        rule > 20 && cell_of(&app, "context").1 < heading,
        "the rejected block must follow the card body behind a dashed rule"
    );
}

fn recovered_picture_artifacts() -> CardArtifacts {
    let picture = ArtifactSlot::fresh(Artifact::Picture)
        .faulted(AttemptFault::new(
            "border",
            "White border missing on: bottom",
            Some(PathBuf::from("/tmp/attempt-0001.jpg")),
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
fn a_finished_artifact_keeps_showing_the_attempts_it_cost() {
    let app = seeded(vec![draft("whilst", recovered_picture_artifacts())]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("✓ picture.jpg") && rendered.contains("1 rejected"),
        "a finished artifact hid the attempts that were spent to reach it: {rendered}"
    );
}
