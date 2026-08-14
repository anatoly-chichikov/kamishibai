//! Language pair must be visible on every fullscreen state and the user must
//! be able to change `my language` without leaving the flow. The change has
//! to persist across app restarts via the preference store.

use kamishibai::config::{PreferenceStore, Preferences};
use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, LanguagePair, WordCandidate,
};
use kamishibai::tui::{
    App, AppEvent, KeySource, LanguageChoice, PickerSection, Screen, Side, WelcomeStage, draw,
    learning_target, to_app, transit,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::tempdir;

fn flat(app: &App) -> String {
    let backend = TestBackend::new(120, 24);
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

fn ready() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

#[test]
fn app_debug_redacts_the_welcome_key() {
    let app = App::new(LanguagePair::new("en", "ru")).welcome_paste_key("debug-secret-welcome");
    let rendered = format!("{app:?}");
    assert_eq!(
        (
            rendered.contains("debug-secret-welcome"),
            rendered.contains("[REDACTED]")
        ),
        (false, true),
        "App Debug exposed the Welcome API key"
    );
}

fn meta_for(term: &str) -> CardMeta {
    CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source for {term}"),
        term,
        format!("hint for {term}"),
        format!("context for {term}"),
        format!("Example with {term}."),
    )
}

fn base() -> App {
    App::new(LanguagePair::new("en", "ru"))
}

/// One bare key press, through the real input mapper.
fn press(code: crossterm::event::KeyCode) -> AppEvent {
    to_app(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ))
    .expect("the key must map to an event")
}

/// A reviewed batch: the only state where changing the pair rereads words.
fn reviewed() -> App {
    base()
        .with_screen(Screen::WhatIUnderstood)
        .understood(vec![WordCandidate::new("chat", "a cat", true)])
}

/// A pick that moves only the known half and leaves detection in charge.
fn known_choice(known: &str) -> LanguageChoice {
    LanguageChoice::new(known.to_uppercase(), learning_target(None))
}

/// A pick that also pins the learning half.
fn pinned_choice(known: &str, learning: &str) -> LanguageChoice {
    LanguageChoice::new(known.to_uppercase(), learning_target(Some(learning)))
}

#[test]
fn ctrl_l_on_welcome_language_step_cycles_the_language() {
    let app = App::new(LanguagePair::new("en", "en")).opening_welcome(
        KeySource::Env,
        "123456789012345678901234567890",
        true,
    );
    let event = to_app(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyModifiers::CONTROL,
    ))
    .expect("Ctrl+L must map");
    let (after, side) = transit(app, event);
    assert_eq!(
        (
            after.screen(),
            after.welcome().stage,
            after.pair().known().to_string(),
            side,
        ),
        (
            Screen::Welcome,
            WelcomeStage::PickLanguage,
            kamishibai::languages::catalog().codes()[1].to_string(),
            Side::None,
        ),
        "Ctrl+L on the first Welcome slide must cycle the language without persisting it"
    );
}

#[test]
fn welcome_language_step_footer_is_clean() {
    let app = App::new(LanguagePair::new("en", "en")).opening_welcome(
        KeySource::Env,
        "123456789012345678901234567890",
        true,
    );
    let rendered = flat(&app);
    assert_eq!(
        (
            rendered.contains("[↑ ↓ ← →] language"),
            rendered.contains("Ctrl+L")
        ),
        (true, false),
        "the language step footer must offer the arrow-key language switch and drop the redundant Ctrl+L"
    );
}

#[test]
fn welcome_key_step_offers_load_from_env_only_when_present() {
    let with_env = flat(&App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Empty,
        "",
        true,
    ));
    let without_env = flat(&App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Empty,
        "",
        false,
    ));
    assert_eq!(
        (
            with_env.contains("load from env"),
            without_env.contains("load from env"),
        ),
        (true, false),
        "the key step must offer load-from-env only when GEMINI_API_KEY is present"
    );
}

#[test]
fn key_step_does_not_advertise_where_to_get_a_key() {
    let rendered = flat(&App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Empty,
        "",
        false,
    ));
    assert_eq!(
        (
            rendered.contains("get a key"),
            rendered.contains("aistudio")
        ),
        (false, false),
        "the key step must not advertise where to get a key — the user sorts that out themselves"
    );
}

#[test]
fn loaded_and_env_key_steps_drop_helper_prose() {
    let loaded = flat(&App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Env,
        "123456789012345678901234567890",
        true,
    ));
    let env_empty = flat(&App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Empty,
        "",
        true,
    ));
    assert_eq!(
        (
            loaded.contains("Enter to start"),
            env_empty.contains("found in env"),
        ),
        (false, false),
        "the key step drops helper prose once a key exists or env can supply one"
    );
}

#[test]
fn empty_welcome_submit_asks_for_a_key_without_leaving_setup() {
    let app = App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Empty,
        "",
        false,
    );
    let (after, side) = transit(app, AppEvent::Submit);
    assert_eq!(
        (
            after.screen(),
            after.welcome().stage,
            after.welcome().notice.clone(),
            side,
        ),
        (
            Screen::Welcome,
            WelcomeStage::EnterKey,
            Some(String::from("enter a key first")),
            Side::None,
        ),
        "submitting an empty key step must stay on Welcome and tell the user how to provide a key"
    );
}

#[test]
fn welcome_key_is_checked_before_it_is_saved() {
    let app = App::new(LanguagePair::new("en", "ru")).opening_welcome_at(
        WelcomeStage::EnterKey,
        KeySource::Env,
        "123456789012345678901234567890",
        true,
    );
    let (_after, side) = transit(app, AppEvent::Submit);
    assert_eq!(
        side,
        Side::ValidateKey(String::from("123456789012345678901234567890")),
        "submitting a key must request a live validity check before anything is persisted"
    );
}

#[test]
fn rejected_key_notice_renders_on_the_key_step() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .opening_welcome_at(
            WelcomeStage::EnterKey,
            KeySource::Pasted,
            "123456789012345678901234567890",
            false,
        )
        .welcome_notice("key invalid");
    let rendered = flat(&app);
    assert!(
        rendered.contains("key invalid"),
        "a rejected key must surface its notice inline on the key step"
    );
}

#[test]
fn language_badge_is_consistent_across_every_fullscreen_screen() {
    let your_words = base().confirmed_learning("en");
    let what_i_understood = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new(
            "whilst",
            "neutral conjunction",
            true,
        )]);
    let your_cards = base()
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(vec![
            CardDraft::new("whilst", "understanding", LanguagePair::new("en", "ru"))
                .with_meta(meta_for("whilst"), None)
                .with_artifacts(ready()),
        ]);
    let done = base()
        .with_screen(Screen::Done)
        .confirmed_learning("en")
        .done_published("deck.apkg", "report.pdf", "out/");
    for app in [your_words, what_i_understood, your_cards, done] {
        let rendered = flat(&app);
        assert!(
            rendered.contains("→ EN"),
            "every fullscreen screen must render the compact language chip"
        );
    }
}

#[test]
fn your_words_shows_detecting_marker_before_target_is_confirmed() {
    let app = base();
    let rendered = flat(&app);
    assert!(
        rendered.contains("…"),
        "while target is pending the language chip must show a `…` placeholder"
    );
}

#[test]
fn picking_my_language_on_what_i_understood_persists_the_new_code() {
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker(PickerSection::Known));
    assert_eq!(
        opened.modal(),
        Some(kamishibai::tui::ModalKind::PickLanguages),
        "OpenLanguagePicker on What I understood must open the pair modal"
    );
    let (after, side) = transit(opened, AppEvent::SetLanguages(known_choice("es")));
    assert_eq!(
        (after.modal(), after.pair().known().to_string(), side,),
        (
            None,
            String::from("ES"),
            Side::AdoptLanguages(known_choice("es")),
        ),
        "confirming a language pick must close the modal, swap support, and request persistence"
    );
}

#[test]
fn letter_l_on_done_is_not_a_hidden_language_shortcut() {
    let app = base()
        .with_screen(Screen::Done)
        .confirmed_learning("en")
        .done_published("deck.apkg", "report.pdf", "out/");
    let (after, side) = transit(app, AppEvent::KeyChar('l'));
    assert_eq!(
        (after.pair().known().to_string(), side),
        (String::from("ru"), Side::None),
        "letter L on Done must not be a hidden language shortcut"
    );
}

#[test]
fn picker_modal_opens_with_the_active_language_preselected_and_arrows_cycle_through_the_catalog() {
    let app = base().confirmed_learning("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker(PickerSection::Known));
    let initial = PickerSection::Known.chip_for("ru");
    assert_eq!(
        opened.picker_cursor().index(PickerSection::Known),
        initial,
        "the picker must open with the cursor on the active `my` language"
    );
    let (after_right, _) = transit(opened, AppEvent::LanguagePickerNext);
    let (after_left_left, _) = transit(after_right, AppEvent::LanguagePickerPrev);
    assert_eq!(
        after_left_left.picker_cursor().index(PickerSection::Known),
        initial,
        "Prev after Next must return the cursor to the original chip"
    );
}

#[test]
fn picker_does_not_open_on_your_cards_or_done_because_the_pair_is_frozen() {
    for screen in [Screen::YourCards, Screen::Done] {
        let app = base().with_screen(screen).confirmed_learning("en");
        let (after, side) = transit(app, AppEvent::OpenLanguagePicker(PickerSection::Known));
        assert_eq!(
            (after.modal(), side),
            (None, Side::None),
            "OpenLanguagePicker on {screen:?} must not open the picker — the batch pair is frozen"
        );
    }
}

#[test]
fn clicking_the_learning_half_opens_the_modal_on_the_learning_half() {
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker(PickerSection::Learning));
    assert_eq!(
        opened.picker_cursor().section(),
        PickerSection::Learning,
        "opening the modal from the learning half must focus that half"
    );
}

#[test]
fn up_and_down_move_inside_one_column_while_left_and_right_swap_columns() {
    let opened = transit(
        base().confirmed_learning("en"),
        AppEvent::OpenLanguagePicker(PickerSection::Known),
    )
    .0;
    let (moved, _) = transit(opened, AppEvent::LanguagePickerNext);
    let (turned, _) = transit(
        moved,
        AppEvent::LanguagePickerFocus(PickerSection::Learning),
    );
    assert_eq!(
        (
            turned.picker_cursor().section(),
            turned.picker_cursor().index(PickerSection::Known)
        ),
        (
            PickerSection::Learning,
            PickerSection::Known.chip_for("ru") + 1
        ),
        "swapping columns must keep the pick the other column already made"
    );
}

#[test]
fn the_arrow_keys_map_to_columns_and_up_down_to_rows() {
    let opened = transit(
        base().confirmed_learning("en"),
        AppEvent::OpenLanguagePicker(PickerSection::Known),
    )
    .0;
    let right = transit(opened.clone(), press(crossterm::event::KeyCode::Right)).0;
    let down = transit(opened, press(crossterm::event::KeyCode::Down)).0;
    assert_eq!(
        (
            right.picker_cursor().section(),
            right.picker_cursor().index(PickerSection::Known),
            down.picker_cursor().section(),
            down.picker_cursor().index(PickerSection::Known),
        ),
        (
            PickerSection::Learning,
            PickerSection::Known.chip_for("ru"),
            PickerSection::Known,
            PickerSection::Known.chip_for("ru") + 1,
        ),
        "the vertical lists must take rows from up/down and columns from left/right"
    );
}

#[test]
fn pressing_left_on_the_left_column_stays_put() {
    let opened = transit(
        base().confirmed_learning("en"),
        AppEvent::OpenLanguagePicker(PickerSection::Known),
    )
    .0;
    assert_eq!(
        transit(opened, press(crossterm::event::KeyCode::Left))
            .0
            .picker_cursor()
            .section(),
        PickerSection::Known,
        "left on the leftmost column must not throw focus across the modal"
    );
}

#[test]
fn the_unpinned_learning_half_opens_on_the_auto_chip() {
    let opened = transit(
        base().confirmed_learning("en"),
        AppEvent::OpenLanguagePicker(PickerSection::Learning),
    )
    .0;
    assert_eq!(
        PickerSection::Learning.code_at(opened.picker_cursor().index(PickerSection::Learning)),
        None,
        "a batch left to detection must open the learning half on auto"
    );
}

#[test]
fn pinning_the_learning_language_rereads_the_reviewed_words() {
    let app = reviewed().confirmed_learning("en");
    let (after, side) = transit(app, AppEvent::SetLanguages(pinned_choice("ru", "de")));
    assert_eq!(
        (after.pair().learning().to_string(), side),
        (
            String::from("DE"),
            Side::AdoptLanguagesAndRunUnderstanding(pinned_choice("ru", "de")),
        ),
        "pinning a different learning language must reread the batch under it"
    );
}

#[test]
fn a_pin_naming_the_language_already_on_screen_costs_no_provider_call() {
    let app = reviewed().confirmed_learning("en");
    let (_after, side) = transit(app, AppEvent::SetLanguages(pinned_choice("ru", "en")));
    assert_eq!(
        side,
        Side::AdoptLanguages(pinned_choice("ru", "en")),
        "pinning the language already understood must not reread the batch"
    );
}

#[test]
fn confirming_the_pair_unchanged_asks_for_nothing() {
    let app = reviewed().confirmed_learning("en");
    let (_after, side) = transit(app, AppEvent::SetLanguages(known_choice("ru")));
    assert_eq!(
        side,
        Side::None,
        "confirming the modal without changing anything must not touch preferences or Gemini"
    );
}

#[test]
fn dropping_a_pin_hands_the_language_back_to_detection() {
    let pinned = transit(
        reviewed().confirmed_learning("en"),
        AppEvent::SetLanguages(pinned_choice("ru", "de")),
    )
    .0;
    let (after, side) = transit(pinned, AppEvent::SetLanguages(known_choice("ru")));
    assert_eq!(
        (after.learning_pin(), side),
        (
            None,
            Side::AdoptLanguagesAndRunUnderstanding(known_choice("ru")),
        ),
        "the auto chip must drop the pin and let the pass decide again"
    );
}

#[test]
fn picking_my_language_through_transit_writes_to_the_preference_store() {
    let home = tempdir().expect("temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("ru"))
        .expect("seed my language");
    let app = base()
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en");
    let (opened, _) = transit(app, AppEvent::OpenLanguagePicker(PickerSection::Known));
    let (after, side) = transit(opened, AppEvent::SetLanguages(known_choice("es")));
    if let Side::AdoptLanguages(choice) = side {
        store
            .write(&Preferences::new(choice.known()))
            .expect("persist new code");
    }
    let restored = store.read().expect("reload preferences").my_language;
    assert_eq!(
        (after.pair().known().to_string(), restored),
        (String::from("ES"), String::from("ES")),
        "picking a language through the modal must update both the session pair and the persisted preference"
    );
}

#[test]
fn typing_letter_l_inside_your_words_does_not_rotate_the_language() {
    let mut state = base();
    for symbol in "apple".chars() {
        let event = kamishibai::tui::to_app(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(symbol),
            crossterm::event::KeyModifiers::NONE,
        ))
        .expect("char");
        state = transit(state, event).0;
    }
    assert_eq!(
        state.pair().known().to_string(),
        "ru",
        "letter L inside Your words blob must be treated as user input, not as a global shortcut"
    );
}

/// The open picker is redrawn on every keypress and hit-tested on every tick of
/// the pointer refresh, so both paths have to be cheap. Neither was: each one
/// rebuilt the whole language catalog — twenty-two profiles and every string
/// inside them — thousands of times per frame, once per rendered span and once
/// per candidate row, and the modal fell seconds behind the arrow keys.
#[test]
fn the_open_picker_keeps_up_with_the_arrow_keys() {
    let opened = transit(
        reviewed(),
        AppEvent::OpenLanguagePicker(PickerSection::Known),
    )
    .0;
    let terminal = ratatui::layout::Rect::new(0, 0, 120, 24);
    let started = std::time::Instant::now();
    for _ in 0..20 {
        flat(&opened);
        for column in 0..8 {
            let _ = kamishibai::tui::mouse_pointer_at(&opened, terminal, column * 12, 12);
        }
    }
    let spent = started.elapsed();
    assert!(
        spent < std::time::Duration::from_secs(3),
        "twenty picker frames with a pointer sweep took {spent:?}, which is the lag the modal showed on every arrow key"
    );
}

/// The Welcome language step lays every language out as a grid, so a click has
/// to land on the language actually drawn under the pointer.
#[test]
fn clicking_a_language_on_the_welcome_grid_picks_that_language() {
    let app = App::new(LanguagePair::new("en", "en")).opening_welcome(
        KeySource::Empty,
        String::new(),
        false,
    );
    let terminal = ratatui::layout::Rect::new(0, 0, 120, 40);
    let landed = kamishibai::tui::welcome_language_at(&app, terminal, 0, 0);
    let ru = PickerSection::Known.chip_for("ru");
    let target = (0..terminal.width)
        .flat_map(|x| (0..terminal.height).map(move |y| (x, y)))
        .find(|(x, y)| kamishibai::tui::welcome_language_at(&app, terminal, *x, *y) == Some(ru))
        .expect("the grid must draw Russian somewhere");
    let picked = transit(app, AppEvent::WelcomeLanguageAt(ru)).0;
    assert_eq!(
        (landed, picked.pair().known().to_string()),
        (None, String::from("ru")),
        "clicking the cell at {target:?} must pick the language drawn there and nowhere else"
    );
}

/// `↑` and `↓` move one whole line of the grid, which is what makes a grid
/// worth having: `←` and `→` alone would walk the catalog one language at a
/// time.
#[test]
fn a_vertical_arrow_on_the_welcome_grid_moves_one_whole_line() {
    let app = App::new(LanguagePair::new("en", "en")).opening_welcome(
        KeySource::Empty,
        String::new(),
        false,
    );
    let terminal = ratatui::layout::Rect::new(0, 0, 120, 40);
    let down = kamishibai::tui::welcome_language_step(&app, terminal, 1)
        .expect("the language step must answer a vertical arrow");
    let moved = transit(app, down).0;
    assert_eq!(
        moved.pair().known().to_string(),
        "ja",
        "one press of down must skip the whole grid line below English, which is three languages wide here"
    );
}

/// Every language must stay clickable on a terminal too small for their names,
/// which is the shape the step falls back to rather than hiding a language.
#[test]
fn every_language_stays_clickable_on_a_narrow_welcome_step() {
    let app = App::new(LanguagePair::new("en", "en")).opening_welcome(
        KeySource::Empty,
        String::new(),
        false,
    );
    let terminal = ratatui::layout::Rect::new(0, 0, 80, 16);
    let reachable = (0..terminal.width)
        .flat_map(|x| (0..terminal.height).map(move |y| (x, y)))
        .filter_map(|(x, y)| kamishibai::tui::welcome_language_at(&app, terminal, x, y))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        reachable,
        (0..PickerSection::Known.chips()).collect::<std::collections::BTreeSet<_>>(),
        "a language the narrow step draws stayed out of the mouse's reach"
    );
}
