//! Render demo for the head-row meta preview on `Your cards`.
//!
//! `cargo run --example meta_preview_demo` walks through several scenarios at
//! a few terminal widths and prints the rendered buffer for each. Intended for
//! visual verification of the wrap behavior and styling.

use kamishibai::session::{
    Artifact, ArtifactSlot, CardArtifacts, CardDraft, CardMeta, LanguagePair,
};
use kamishibai::tui::{App, Screen, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn ready_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene).succeeded(),
        ArtifactSlot::fresh(Artifact::Picture).succeeded(),
        ArtifactSlot::fresh(Artifact::Sound).succeeded(),
    )
}

fn meta_only_artifacts() -> CardArtifacts {
    CardArtifacts::from_parts(
        ArtifactSlot::fresh(Artifact::Meta).succeeded(),
        ArtifactSlot::fresh(Artifact::Scene),
        ArtifactSlot::fresh(Artifact::Picture),
        ArtifactSlot::fresh(Artifact::Sound),
    )
}

fn draft(
    term: &str,
    target_sentence: &str,
    artifacts: CardArtifacts,
    pair: LanguagePair,
) -> CardDraft {
    let meta = CardMeta::new(
        format!("/{term}/"),
        format!("/{term} sentence/"),
        format!("meaning of {term}"),
        5,
        format!("source for {term}"),
        term,
        format!("hint for {term}"),
        format!("notes for {term}"),
        target_sentence,
    );
    CardDraft::new(term, format!("understanding for {term}"), pair)
        .with_meta(meta, None)
        .with_artifacts(artifacts)
}

fn untouched(term: &str, pair: LanguagePair) -> CardDraft {
    CardDraft::new(term, format!("understanding for {term}"), pair)
}

fn render(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            let symbol = buffer[(column, row)].symbol();
            if symbol.is_empty() {
                continue;
            }
            out.push_str(symbol);
        }
        out.push('\n');
    }
    out
}

fn banner(title: &str, width: u16) {
    let bar = "─".repeat(width as usize);
    println!("\n{bar}");
    println!("{title}");
    println!("{bar}");
}

fn print_scene(title: &str, app: &App, width: u16, height: u16) {
    banner(title, width);
    print!("{}", render(app, width, height));
}

fn seeded(drafts: Vec<CardDraft>, pair: LanguagePair) -> App {
    App::new(pair)
        .with_screen(Screen::YourCards)
        .confirmed_learning("en")
        .cards_started(drafts)
}

fn main() {
    let pair_ru_en = LanguagePair::new("en", "ru");
    let pair_ja_ru = LanguagePair::new("ru", "ja");

    let mixed = seeded(
        vec![
            draft(
                "whilst",
                "She kept reading whilst the kettle boiled.",
                ready_artifacts(),
                pair_ru_en.clone(),
            ),
            draft(
                "at the end",
                "We were exhausted at the end.",
                meta_only_artifacts(),
                pair_ru_en.clone(),
            ),
            untouched("ancient", pair_ru_en.clone()),
            draft(
                "in the long run",
                "In the long run all of us are dead, but some of us still have laundry to fold tonight.",
                meta_only_artifacts(),
                pair_ru_en.clone(),
            ),
        ],
        pair_ru_en.clone(),
    );
    print_scene(
        "scenario 1 — mixed: short, medium, untouched, long (width 80)",
        &mixed,
        80,
        22,
    );
    print_scene("scenario 2 — same set at narrower width 60", &mixed, 60, 26);
    print_scene("scenario 3 — same set at wider width 120", &mixed, 120, 18);

    let japanese = seeded(
        vec![
            draft(
                "猫",
                "Кошка спит на подоконнике у окна весь долгий вечер.",
                ready_artifacts(),
                pair_ja_ru.clone(),
            ),
            draft(
                "雨が降る",
                "Дождь идёт.",
                meta_only_artifacts(),
                pair_ja_ru.clone(),
            ),
            draft(
                "図書館",
                "Я хожу в библиотеку каждое воскресенье после обеда, чтобы взять новые книги и просто посидеть в тишине.",
                meta_only_artifacts(),
                pair_ja_ru.clone(),
            ),
        ],
        pair_ja_ru.clone(),
    );
    print_scene(
        "scenario 4 — japanese terms, russian targets (width 80)",
        &japanese,
        80,
        18,
    );

    let chinese_target = seeded(
        vec![
            draft(
                "harbor",
                "港口在傍晚时分非常安静，远处只有海鸥的叫声打破寂静。",
                ready_artifacts(),
                pair_ru_en.clone(),
            ),
            draft(
                "morning",
                "光从窗户照进来。",
                meta_only_artifacts(),
                pair_ru_en.clone(),
            ),
        ],
        pair_ru_en.clone(),
    );
    print_scene(
        "scenario 5 — non-spaced target sentence (chinese), width 80",
        &chinese_target,
        80,
        14,
    );

    let one_huge = seeded(
        vec![draft(
            "supercalifragilisticexpialidocious",
            "It is supposedly the longest English word in popular use and even saying it should make you sound precocious or atrocious.",
            ready_artifacts(),
            pair_ru_en.clone(),
        )],
        pair_ru_en.clone(),
    );
    print_scene(
        "scenario 6 — very long term + very long sentence, width 60",
        &one_huge,
        60,
        14,
    );
    print_scene(
        "scenario 7 — same single huge card, width 100",
        &one_huge,
        100,
        12,
    );
}
