//! Integration flow for `Your words -> What I understood -> drop item -> make cards`.
//!
//! Uses the real input mapper, renderer, and transition function. The LLM
//! understanding pass is replaced with an inline `Understanding` fake.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kamishibai::session::LearningGuess;
use kamishibai::session::{
    LanguagePair, LearningTarget, MAX_PLAN_CARDS, RawInputBatch, Sense, Understanding, Understood,
    WordCandidate,
};
use kamishibai::tui::{App, AppEvent, ModalKind, ReviewFocus, Screen, Side, draw, to_app, transit};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn modified(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn flat(app: &App) -> String {
    let backend = TestBackend::new(140, 24);
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

fn modifiers(app: &App, needle: &str) -> Vec<Modifier> {
    let backend = TestBackend::new(140, 24);
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
            return (0..needle.chars().count())
                .map(|offset| buffer[(column + offset as u16, row)].modifier)
                .collect();
        }
    }
    Vec::new()
}

fn style_of(app: &App, needle: &str) -> (Color, Color, Modifier) {
    let backend = TestBackend::new(140, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        if let Some(start) = rendered.find(needle) {
            let column = u16::try_from(rendered[..start].chars().count())
                .expect("rendered column must fit the terminal");
            let cell = &buffer[(column, row)];
            return (cell.fg, cell.bg, cell.modifier);
        }
    }
    panic!("the rendered screen never showed '{needle}'");
}

fn has_highlight_after_text(app: &App, needle: &str) -> bool {
    let backend = TestBackend::new(140, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let mut rendered = String::new();
        let mut last_text = None;
        for column in 0..buffer.area.width {
            let symbol = buffer[(column, row)].symbol();
            rendered.push_str(symbol);
            if !symbol.trim().is_empty() {
                last_text = Some(column);
            }
        }
        if rendered.contains(needle) {
            let Some(last_text) = last_text else {
                return false;
            };
            return ((last_text + 1)..buffer.area.width)
                .any(|column| buffer[(column, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f));
        }
    }
    false
}

fn highlight_is_contiguous(app: &App, needle: &str) -> bool {
    let backend = TestBackend::new(140, 24);
    let mut terminal = Terminal::new(backend).expect("backend");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        let rendered = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if rendered.contains(needle) {
            let highlighted = (0..buffer.area.width)
                .filter(|column| buffer[(*column, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f))
                .collect::<Vec<_>>();
            let Some((first, last)) = highlighted.first().zip(highlighted.last()) else {
                return false;
            };
            return (*first..=*last)
                .all(|column| buffer[(column, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f));
        }
    }
    false
}

fn many_candidates(count: usize) -> Vec<WordCandidate> {
    (1..=count)
        .map(|index| {
            WordCandidate::new(
                format!("term-{index:02}"),
                format!("understanding for term-{index:02}"),
                true,
            )
        })
        .collect()
}

fn bank_candidate() -> WordCandidate {
    WordCandidate::with_senses(
        "bank",
        vec![
            Sense::tagged("Сущ. «банк», финансовое учреждение.", "фин."),
            Sense::plain("Сущ. «берег» реки или водоёма."),
            Sense::tagged("Гл. «наклонять(ся)» при повороте самолёта.", "авиац."),
        ],
        0,
        true,
    )
}

fn single_candidate() -> WordCandidate {
    WordCandidate::new(
        "bittersweet",
        "Прил. про смешанное чувство — радостное и грустное.",
        true,
    )
}

struct FakeUnderstanding;

impl Understanding for FakeUnderstanding {
    fn understand(
        &self,
        _raw: &RawInputBatch,
        _my: &str,
        _target: &LearningTarget,
    ) -> Result<Understood> {
        Ok(Understood::new(
            LearningGuess::new("en", true),
            vec![
                WordCandidate::new(
                    "sincerely",
                    "Наречие «искренне» — формальная закрывающая фраза в письмах.",
                    true,
                ),
                WordCandidate::new(
                    "expel",
                    "Глагол «исключить» в смысле учебного заведения, не «выпустить газ».",
                    true,
                ),
                WordCandidate::new("at the end", "Фраза о времени или месте — «в конце».", true),
                WordCandidate::new(
                    "celebratory",
                    "Прилагательное «праздничный»; в исходнике опечатка, исправлено.",
                    true,
                ),
                WordCandidate::new(
                    "debuted",
                    "Прошедшая форма глагола «дебютировать», окончание -ed.",
                    true,
                ),
            ],
        ))
    }
}

fn run_understanding(app: App) -> App {
    let result = FakeUnderstanding
        .understand(
            &RawInputBatch::new(app.blob()),
            app.pair().known(),
            &LearningTarget::Detect,
        )
        .expect("fake understanding must succeed");
    app.with_screen(Screen::WhatIUnderstood)
        .confirmed_learning(result.guess().code())
        .understood(result.candidates().to_vec())
}

#[test]
fn long_what_i_understood_list_scrolls_to_the_selected_candidate() {
    let mut app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(many_candidates(35));
    for _ in 0..29 {
        app = transit(app, AppEvent::NavNext)
            .0
            .body_scroll_to_selection(6, 132);
    }
    let rendered = flat(&app);
    assert!(
        app.body_scroll() > 0 && rendered.contains("term-30") && !rendered.contains("term-01"),
        "long review lists must keep the selected candidate inside the visible scroll window: {rendered}"
    );
}

#[test]
fn what_i_understood_renders_understanding_rows_with_localized_prompts_and_card_count() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(
            FakeUnderstanding
                .understand(&RawInputBatch::new("whilst"), "ru", &LearningTarget::Detect)
                .expect("fake must succeed")
                .candidates()
                .to_vec(),
        );
    let rendered = flat(&app);
    assert!(
        rendered.contains("RU → EN")
            && rendered.contains("step 2/3")
            && rendered.contains("what i understood")
            && rendered.contains("quick check before i build the cards")
            && rendered.contains("sincerely")
            && rendered.contains("искренне")
            && rendered.contains("expel")
            && rendered.contains("at the end")
            && rendered.contains("[↑↓]")
            && rendered.contains("[Enter] toggle")
            && rendered.contains("[Ctrl+G]")
            && rendered.contains("generate")
            && rendered.contains("[Esc] back")
            && !rendered.contains("[R] change"),
        "sense check must render the new mono header, gloss list, and key hints: {rendered}"
    );
}

#[test]
fn multi_sense_word_renders_collapsed_with_active_index() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("bank")
            && rendered.contains("банк")
            && rendered.contains("1/3")
            && !rendered.contains("берег"),
        "collapsed multi-sense rows must show active first sense and a selected/total indicator: {rendered}"
    );
}

#[test]
fn multi_meaning_word_lists_only_selected_senses_in_a_collapsed_block() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate().selecting_senses(vec![0, 1])]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("bank")
            && rendered.contains("2/3")
            && rendered.contains("multiple meanings:")
            && rendered.contains("банк")
            && rendered.contains("берег")
            && !rendered.contains("наклонять"),
        "a collapsed word with several selected meanings must head a block and list only those meanings, keeping the X/Y counter: {rendered}"
    );
}

#[test]
fn collapsed_meaning_lines_are_dim_not_selected_highlight() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate().selecting_senses(vec![0, 1])]);
    assert!(
        modifiers(&app, "берег")
            .iter()
            .all(|modifier| !modifier.contains(Modifier::BOLD)),
        "the listed meaning lines must read as dim, read-only context, never the bold selected-row style"
    );
}

#[test]
fn enter_on_multi_sense_word_expands_the_sense_list() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let inside = transit(opened.clone(), AppEvent::NavNext).0;
    let rendered = flat(&inside);
    assert!(
        opened.review_focus() == ReviewFocus::Head(0)
            && inside.sense_list_open(0)
            && rendered.contains("[Space] select")
            && rendered.contains("[Ctrl+G] generate")
            && rendered.contains("✓ [фин.] Сущ. «банк»")
            && rendered.contains("[авиац.] Гл.")
            && rendered.contains("+ add more"),
        "Enter on a multi-sense row must expand a tagged sublist with an add-more row: {rendered}"
    );
}

#[test]
fn expanded_sense_focus_moves_inside_without_dimming_the_parent() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let head = transit(app, AppEvent::KeyEnter).0;
    let opened = transit(head, AppEvent::NavNext).0;
    let second = transit(opened.clone(), AppEvent::NavNext).0;
    let third = transit(second.clone(), AppEvent::NavNext).0;
    let add_more = transit(third, AppEvent::NavNext).0;
    let foreground = Color::Rgb(0xe6, 0xe3, 0xda);
    let muted = Color::Rgb(0x8b, 0x8a, 0x83);
    let background = Color::Rgb(0x0e, 0x0e, 0x10);
    let highlight = Color::Rgb(0x1c, 0x1c, 0x1f);
    assert_eq!(
        (
            style_of(&opened, "bank"),
            style_of(&opened, "1/3"),
            style_of(&opened, "[фин.]"),
            style_of(&opened, "✓"),
            style_of(&second, "✓"),
            style_of(&second, "Сущ. «берег»"),
            style_of(&second, "[авиац.]"),
            style_of(&add_more, "+ add more"),
        ),
        (
            (foreground, background, Modifier::empty()),
            (muted, background, Modifier::empty()),
            (muted, background, Modifier::empty()),
            (foreground, highlight, Modifier::BOLD),
            (foreground, background, Modifier::empty()),
            (foreground, highlight, Modifier::BOLD),
            (muted, background, Modifier::empty()),
            (foreground, highlight, Modifier::BOLD),
        ),
        "expanded focus must move from an ordinary parent into a bright focused choice while selected context stays readable"
    );
}

#[test]
fn right_arrow_on_a_collapsed_multi_sense_row_keeps_the_list_closed() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let pressed = transit(app, to_app(press(KeyCode::Right)).expect("map")).0;
    assert!(
        !pressed.sense_list_open(0),
        "a side arrow opened the sense list even though only Enter may open it"
    );
}

#[test]
fn left_arrow_inside_an_open_sense_list_keeps_it_open_with_selection_intact() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let toggled = transit(third, AppEvent::KeyChar(' ')).0;
    let pressed = transit(toggled, to_app(press(KeyCode::Left)).expect("map")).0;
    assert!(
        pressed.sense_list_open(0) && pressed.candidates()[0].selected_senses() == [0, 2],
        "a side arrow closed the sense list even though only Enter and Esc may close it"
    );
}

#[test]
fn moving_inside_expanded_senses_moves_cursor_without_changing_selection() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let rendered = flat(&third);
    assert!(
        third.review_focus() == ReviewFocus::Sense { row: 0, index: 2 }
            && third.candidates()[0].selected() == 0
            && rendered.contains("✓ [фин.] Сущ. «банк»")
            && rendered.contains("[авиац.] Гл. «наклонять(ся)»"),
        "moving inside the expanded list must move focus without changing the committed selection: {rendered}"
    );
    assert!(
        has_highlight_after_text(&third, "[авиац.]"),
        "focused sense row highlight must continue through the end of the line"
    );
}

#[test]
fn space_toggles_the_focused_sense_and_enter_commits_the_selection() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let toggled = transit(third, AppEvent::KeyChar(' ')).0;
    let confirmed = transit(toggled, AppEvent::KeyEnter).0;
    let rendered = flat(&confirmed);
    assert!(
        !confirmed.sense_list_open(0)
            && confirmed.candidates()[0].selected_senses() == [0, 2]
            && rendered.contains("2/3")
            && !rendered.contains("✓"),
        "Space must mark another sense and Enter must collapse with both meanings selected: {rendered}"
    );
}

#[test]
fn footer_card_count_updates_before_sense_picker_closes() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let selected = transit(second, AppEvent::KeyChar(' ')).0;
    let selected_footer = flat(&selected);
    let deselected = transit(selected.clone(), AppEvent::KeyChar(' ')).0;
    let deselected_footer = flat(&deselected);
    assert_eq!(
        (
            selected.sense_list_open(0),
            selected.candidates()[0].selected_count(),
            selected_footer.contains("2 cards"),
            selected_footer.contains("2/3"),
            deselected.sense_list_open(0),
            deselected.candidates()[0].selected_count(),
            deselected_footer.contains("1 card"),
            deselected_footer.contains("1/3"),
        ),
        (true, 2, true, true, true, 1, true, true),
        "the footer must track each committed sense toggle while the list stays open"
    );
}

#[test]
fn ctrl_g_from_expanded_senses_commits_selection_and_starts_generation() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let toggled = transit(third, AppEvent::KeyChar(' ')).0;
    let (next, side) = transit(toggled, AppEvent::Generate);
    assert_eq!(
        (
            next.screen(),
            next.candidates()[0].selected_senses().to_vec(),
            side,
        ),
        (Screen::YourCards, vec![0, 2], Side::StartGeneration,),
        "Ctrl+G from an open sense list must use the committed choices and start generation"
    );
}

#[test]
fn escape_collapses_the_focused_sense_list_and_keeps_committed_selection() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let toggled = transit(third, AppEvent::KeyChar(' ')).0;
    let collapsed = transit(toggled, AppEvent::Cancel).0;
    let rendered = flat(&collapsed);
    assert!(
        !collapsed.sense_list_open(0)
            && collapsed.candidates()[0].selected_senses() == [0, 2]
            && rendered.contains("2/3")
            && rendered.contains("банк")
            && rendered.contains("наклонять"),
        "Esc from an open sense list must collapse it while every committed toggle survives: {rendered}"
    );
}

#[test]
fn dash_hint_orders_the_requested_sense_first() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::with_senses(
            "set",
            vec![
                Sense::plain("Гл. «установить» значение или устройство."),
                Sense::plain("Сущ. «комплект», набор связанных предметов."),
            ],
            0,
            true,
        )]);
    let rendered = flat(&app);
    assert!(
        rendered.contains("set")
            && rendered.contains("установить")
            && rendered.contains("1/2")
            && !rendered.contains("комплект"),
        "a dash hint must place the requested sense first on the collapsed row: {rendered}"
    );
}

#[test]
fn enter_on_single_sense_word_opens_picker_with_add_more() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![single_candidate()]);
    let (next, side) = transit(app, AppEvent::KeyEnter);
    let rendered = flat(&next);
    assert!(
        next.modal().is_none()
            && next.sense_list_open(0)
            && side == Side::None
            && rendered.contains("+ add more"),
        "Enter on a single-sense row must open the picker with add more instead of a modal: {rendered}"
    );
}

#[test]
fn space_on_add_more_opens_missing_meanings_modal() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let third = transit(second, AppEvent::NavNext).0;
    let add_more = transit(third, AppEvent::NavNext).0;
    let (modal, side) = transit(add_more, AppEvent::KeyChar(' '));
    let rendered = flat(&modal);
    assert!(
        modal.modal() == Some(ModalKind::ChangeSomething)
            && modal.sense_list_open(0)
            && side == Side::None
            && rendered.contains("what meanings did we miss?"),
        "Space on add more must open the missing-meanings modal without closing the picker: {rendered}"
    );
}

#[test]
fn appended_narrow_sense_is_selected_and_the_list_stays_open() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()])
        .senses_appended_to_selected(
            vec![Sense::tagged(
                "Сущ. «банк» как сумма ставок в раздаче.",
                "покер",
            )],
            None,
        );
    let rendered = flat(&app);
    assert!(
        app.sense_list_open(0)
            && rendered.contains("1/4")
            && rendered.contains("✓ [покер] Сущ. «банк»"),
        "add more must append a tagged sense, select it, and keep the list open: {rendered}"
    );
}

#[test]
fn duplicate_change_request_shows_a_short_message_without_adding_senses() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()])
        .senses_appended_to_selected(Vec::new(), Some(String::from("это уже есть в списке")));
    let rendered = flat(&app);
    assert!(
        rendered.contains("это уже есть в списке")
            && rendered.contains("1/3")
            && !rendered.contains("1/4"),
        "duplicate add-more results must show a notice and avoid adding a sense: {rendered}"
    );
}

#[test]
fn empty_add_more_result_shows_minimal_notice_without_adding_senses() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()])
        .sense_list_toggled()
        .senses_appended_to_selected(Vec::new(), None);
    let rendered = flat(&app);
    assert!(
        app.sense_list_open(0)
            && rendered.contains("nothing to add")
            && rendered.contains("1/3")
            && !rendered.contains("1/4"),
        "empty add-more results must keep the picker open and show a quiet notice: {rendered}"
    );
}

#[test]
fn off_language_rows_ignore_enter_and_change_but_can_be_dropped() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new(
            "сообщение",
            "Слово на русском, не на target-языке.",
            false,
        )]);
    let after_enter = transit(app.clone(), AppEvent::KeyEnter).0;
    let after_r = transit(app.clone(), AppEvent::KeyChar('R')).0;
    let after_drop = transit(app, AppEvent::KeyChar('D')).0;
    assert!(
        after_enter.modal().is_none()
            && after_r.modal().is_none()
            && after_drop.candidates().is_empty(),
        "off-language rows must ignore Enter/R and still allow D to drop them"
    );
}

#[test]
fn dropping_the_last_candidate_returns_to_your_words_with_input_cleared() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .seeded_blob("bittersweet")
        .confirmed_learning("en")
        .understood(vec![single_candidate()]);
    let after = transit(app, AppEvent::KeyChar('d')).0;
    assert_eq!(
        (after.screen(), after.blob()),
        (Screen::YourWords, ""),
        "dropping the final candidate must return to the enter-words step with the input wiped"
    );
}

#[test]
fn support_language_rerun_preserves_selected_sense_by_index() {
    let before = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate().selecting(2)]);
    let after = before.understood_preserving_senses(vec![WordCandidate::with_senses(
        "bank",
        vec![
            Sense::tagged("N. a financial institution.", "fin."),
            Sense::plain("N. the side of a river or lake."),
            Sense::tagged("V. to tilt while an aircraft turns.", "aviation"),
        ],
        0,
        true,
    )]);
    let rendered = flat(&after);
    assert!(
        rendered.contains("V. to tilt")
            && rendered.contains("1/3")
            && after.candidates()[0].selected() == 2,
        "support-language reruns must preserve the selected sense by index: {rendered}"
    );
}

#[test]
fn what_i_understood_styles_selected_row_distinctly_from_others() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(
            FakeUnderstanding
                .understand(
                    &RawInputBatch::new("sincerely"),
                    "ru",
                    &LearningTarget::Detect,
                )
                .expect("fake must succeed")
                .candidates()
                .to_vec(),
        );
    assert!(
        modifiers(&app, "sincerely")
            .iter()
            .any(|modifier| modifier.contains(Modifier::BOLD)),
        "the selected term on the gloss list must render in bold"
    );
}

#[test]
fn excluded_candidate_renders_with_strikethrough_and_dim_gloss() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new(
            "сообщение",
            "Слово на русском, не на target-языке — карточка не создаётся.",
            false,
        )]);
    let rendered = flat(&app);
    let term_modifiers = modifiers(&app, "сообщение");
    assert!(
        rendered.contains("не на target-языке")
            && term_modifiers
                .iter()
                .any(|modifier| modifier.contains(Modifier::CROSSED_OUT)),
        "excluded items must show their reason and render the term with a strikethrough: {rendered}"
    );
}

#[test]
fn selected_off_language_row_keeps_one_contiguous_highlight() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new(
            "cat",
            "Это слово не относится к русскому языку.",
            false,
        )]);
    let dim = Color::Rgb(0x8b, 0x8a, 0x83);
    let highlight = Color::Rgb(0x1c, 0x1c, 0x1f);
    assert_eq!(
        (
            highlight_is_contiguous(&app, "Это слово не относится"),
            style_of(&app, "cat"),
            style_of(&app, "Это слово не относится"),
        ),
        (
            true,
            (dim, highlight, Modifier::CROSSED_OUT),
            (dim, highlight, Modifier::empty()),
        ),
        "the selected off-language row must not punch ordinary-background gaps through its highlight"
    );
}

#[test]
fn drop_selected_removes_candidate_and_make_cards_advances_to_your_cards() {
    let start = App::new(LanguagePair::new("en", "ru"))
        .seeded_blob("whilst\nat the end\nin the end\nwreck");
    let (after_submit, side) = transit(start, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        side,
        Side::RunUnderstanding,
        "Generate on blob must request the understanding pass"
    );
    let reviewing = run_understanding(after_submit);
    let after_nav = transit(reviewing, to_app(press(KeyCode::Down)).expect("map")).0;
    let after_drop = transit(after_nav, to_app(press(KeyCode::Char('d'))).expect("map")).0;
    let (after_pick, pick_side) = transit(
        after_drop.clone(),
        to_app(press(KeyCode::Enter)).expect("map"),
    );
    let (after_make, make_side) = transit(
        after_drop,
        to_app(modified(KeyCode::Char('g'), KeyModifiers::CONTROL)).expect("map"),
    );
    let remaining: Vec<String> = after_make
        .candidates()
        .iter()
        .map(|candidate| String::from(candidate.term()))
        .collect();
    assert_eq!(
        (
            after_pick.sense_list_open(after_pick.selected()),
            pick_side,
            after_make.screen(),
            make_side,
            remaining,
        ),
        (
            true,
            Side::None,
            Screen::YourCards,
            Side::StartGeneration,
            vec![
                String::from("sincerely"),
                String::from("at the end"),
                String::from("celebratory"),
                String::from("debuted"),
            ],
        ),
        "flow must drop the highlighted row, then Enter must toggle meanings and Ctrl+G must advance to Your Cards with StartGeneration"
    );
}

#[test]
fn empty_candidate_list_keeps_user_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en");
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        (next.screen(), side),
        (Screen::WhatIUnderstood, Side::None),
        "submitting with no candidates must keep the user on What I understood"
    );
}

#[test]
fn skipped_candidate_list_keeps_user_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new(
            "окно",
            "Слово на русском, не на EN-target — карточка не создаётся.",
            false,
        )]);
    let (next, side) = transit(app, kamishibai::tui::AppEvent::Generate);
    assert_eq!(
        (next.screen(), side),
        (Screen::WhatIUnderstood, Side::None),
        "only skipped candidates must not advance into card generation"
    );
}

#[test]
fn the_review_names_the_languages_the_pass_found_equally_plausible() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new("gift", "a present", true)])
        .with_alternates(vec![String::from("DE"), String::from("NL")]);
    assert!(
        flat(&app).contains("also plausible: DE  ·  NL"),
        "an ambiguous batch must name the other languages it could have been read as"
    );
}

#[test]
fn an_unambiguous_review_says_nothing_about_other_languages() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new("gift", "a present", true)]);
    assert!(
        !flat(&app).contains("also plausible"),
        "an unambiguous batch must not invent a language caveat"
    );
}

#[test]
fn a_pinned_batch_drops_the_alternates_the_pass_reported() {
    let pinned = transit(
        App::new(LanguagePair::new("en", "ru"))
            .with_screen(Screen::WhatIUnderstood)
            .confirmed_learning("en")
            .understood(vec![WordCandidate::new("gift", "a present", true)]),
        AppEvent::SetLanguages(kamishibai::tui::LanguageChoice::new(
            "RU",
            kamishibai::tui::learning_target(Some("de")),
        )),
    )
    .0
    .with_alternates(vec![String::from("NL")]);
    assert!(
        !flat(&pinned).contains("also plausible"),
        "a batch whose language the user pinned must stop second-guessing that decision"
    );
}

#[test]
fn clicking_a_plausible_language_rereads_the_batch_as_that_language() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![WordCandidate::new("gift", "a present", true)])
        .with_alternates(vec![String::from("DE")]);
    let area = ratatui::layout::Rect::new(0, 0, 140, 24);
    let column = u16::try_from("    also plausible: ".len()).expect("column must fit");
    let event = kamishibai::tui::review_event_at(&app, area, column, area.y + 4);
    assert_eq!(
        event,
        Some(AppEvent::SetLanguages(
            kamishibai::tui::LanguageChoice::new(
                "ru",
                kamishibai::tui::learning_target(Some("de"))
            )
        )),
        "clicking a plausible language must reread the batch as it, keeping the known half"
    );
}

#[test]
fn an_oversized_plan_never_starts_generation() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(many_candidates(MAX_PLAN_CARDS + 1));
    let (after, side) = transit(app, AppEvent::Generate);
    assert_eq!(
        (
            side,
            after.screen(),
            after
                .review_notice()
                .is_some_and(|notice| notice.contains("card limit"))
        ),
        (Side::None, Screen::WhatIUnderstood, true),
        "an oversized plan started generating instead of asking for fewer senses"
    );
}

#[test]
fn a_plan_exactly_at_the_card_ceiling_still_starts_generation() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(many_candidates(MAX_PLAN_CARDS));
    assert_eq!(
        transit(app, AppEvent::Generate).1,
        Side::StartGeneration,
        "a plan sitting exactly on the card ceiling was refused"
    );
}

#[test]
fn two_candidates_keep_their_sense_lists_open_at_once() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate(), bank_candidate()]);
    let first_open = transit(app, AppEvent::KeyEnter).0;
    let second_head = (0..5).fold(first_open, |walked, _| transit(walked, AppEvent::NavNext).0);
    let both_open = transit(second_head, AppEvent::KeyEnter).0;
    assert!(
        both_open.sense_list_open(0) && both_open.sense_list_open(1),
        "opening the second sense list collapsed the first instead of keeping both inline"
    );
}

#[test]
fn down_from_the_last_sense_row_moves_to_the_next_candidate_head_leaving_the_list_open() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate(), bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let walked = (0..5).fold(opened, |walked, _| transit(walked, AppEvent::NavNext).0);
    assert!(
        walked.review_focus() == ReviewFocus::Head(1) && walked.sense_list_open(0),
        "walking out of the open list bottom closed it instead of passing through to the next word"
    );
}

#[test]
fn up_from_a_candidate_head_reenters_the_previous_open_list_at_its_add_more_row() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate(), bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let second_head = (0..5).fold(opened, |walked, _| transit(walked, AppEvent::NavNext).0);
    let reentered = transit(second_head, AppEvent::NavPrev).0;
    assert_eq!(
        reentered.review_focus(),
        ReviewFocus::Sense { row: 0, index: 3 },
        "walking up past a head skipped the previous open list instead of entering its add-more row"
    );
}

#[test]
fn space_commits_a_sense_toggle_into_the_candidate_immediately() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let second = transit(first, AppEvent::NavNext).0;
    let toggled = transit(second, AppEvent::KeyChar(' ')).0;
    assert_eq!(
        toggled.candidates()[0].selected_senses(),
        [0, 1],
        "a Space toggle stayed tentative instead of committing into the candidate"
    );
}

#[test]
fn space_cannot_deselect_the_last_selected_sense() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let toggled = transit(first, AppEvent::KeyChar(' ')).0;
    assert_eq!(
        toggled.candidates()[0].selected_senses(),
        [0],
        "Space removed the only selected sense and left the word with no meaning"
    );
}

#[test]
fn c_collapses_every_expanded_block_on_what_i_understood() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate(), bank_candidate()]);
    let first_open = transit(app, AppEvent::KeyEnter).0;
    let second_head = (0..5).fold(first_open, |walked, _| transit(walked, AppEvent::NavNext).0);
    let both_open = transit(second_head, AppEvent::KeyEnter).0;
    let collapsed = transit(both_open, AppEvent::KeyChar('c')).0;
    assert!(
        !collapsed.any_sense_list_open() && collapsed.review_focus() == ReviewFocus::Head(1),
        "collapse all left a sense list open or moved the walk off its row"
    );
}

#[test]
fn c_collapses_open_lists_and_closes_guidance_together() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let guided = transit(opened, AppEvent::KeyChar('S')).0;
    let pressed = transit(guided, AppEvent::KeyChar('c')).0;
    assert!(
        !pressed.any_sense_list_open() && pressed.sentence_settings_editor().is_none(),
        "collapse all left the guidance editor or an open sense list behind"
    );
}

#[test]
fn c_opens_every_reviewable_sense_list_when_none_is_open() {
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![
            bank_candidate(),
            WordCandidate::new("сообщение", "Слово на русском, не на target-языке.", false),
        ]);
    let expanded = transit(app, AppEvent::KeyChar('c')).0;
    assert!(
        expanded.sense_list_open(0) && !expanded.sense_list_open(1),
        "the collapse toggle failed to open every reviewable list while skipping off-language rows"
    );
}

#[test]
fn focused_add_more_row_highlight_starts_where_sense_row_highlights_start() {
    fn first_highlighted_column(app: &App, needle: &str) -> Option<u16> {
        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).expect("backend");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let buffer = terminal.backend().buffer();
        for row in 0..buffer.area.height {
            let rendered = (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>();
            if rendered.contains(needle) {
                return (0..buffer.area.width)
                    .find(|column| buffer[(*column, row)].bg == Color::Rgb(0x1c, 0x1c, 0x1f));
            }
        }
        None
    }
    let app = App::new(LanguagePair::new("en", "ru"))
        .with_screen(Screen::WhatIUnderstood)
        .confirmed_learning("en")
        .understood(vec![bank_candidate()]);
    let opened = transit(app, AppEvent::KeyEnter).0;
    let first = transit(opened, AppEvent::NavNext).0;
    let on_sense = transit(first, AppEvent::NavNext).0;
    let add_more = (0..2).fold(on_sense.clone(), |walked, _| {
        transit(walked, AppEvent::NavNext).0
    });
    assert!(
        first_highlighted_column(&add_more, "+ add more").is_some()
            && first_highlighted_column(&add_more, "+ add more")
                == first_highlighted_column(&on_sense, "«берег»"),
        "the focused add-more row painted a shorter highlight than the sense rows"
    );
}
