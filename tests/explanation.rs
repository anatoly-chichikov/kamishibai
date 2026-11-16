//! Offline contract for the public explanation layout and judge.

use std::collections::BTreeSet;

use kamishibai::explanation::{Card, Language, Parts, compose, judge, quote, tidy};
use serde::Deserialize;

/// Supply one accepted English explanation to offline workflow providers.
pub(crate) fn cat_response() -> serde_json::Value {
    serde_json::json!({"meanings":[{"head":"a cat","note":"a furry pet on a windowsill","from":"1"}],"literal":"","where":{"text":"","example":{"text":"","translation":""}},"limits":{"text":"","example":{"text":"","translation":""}},"nuance":{"text":"","why":"","example":{"text":"","translation":""}}})
}

/// Build one complete approved card for renderer and note-identity evidence.
pub(crate) fn approved_entry() -> kamishibai::vocabulary::VocabularyEntry {
    let cards: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/explanation/cards.json")).unwrap();
    let card = &cards[0];
    serde_json::from_value(serde_json::json!({"term":card["term"],"meaning":card["translation"],"pronunciation":card["pronunciation"],"transcription":"sample","importance":7,"source":{"sentence":card["sentence_known"],"lang":"RU","highlight":"крайняя черта","hint":"A boundary on the sand","context":card["markdown"]},"target":{"sentence":card["sentence"],"lang":"EN"}})).unwrap()
}

#[test]
fn the_new_explanation_note_keeps_a_complete_field_and_identity_manifest() {
    use kamishibai::anki::{CardModel, NoteFormat, VocabularyNote};
    let note = VocabularyNote::new(CardModel::new().model()).note(
        &approved_entry(),
        "[sound:probe.wav]",
        "<img src=\"probe.jpg\">",
    );
    insta::assert_snapshot!(
        "explanation_note_manifest",
        serde_json::to_string_pretty(
            &serde_json::json!({"fields":note.fields,"guid":note.guid,"sort_field":note.sort_field})
        )
        .unwrap()
    );
}

#[derive(Deserialize)]
struct Case {
    id: String,
    card: Card,
    parts: Parts,
    markdown: String,
    fails: BTreeSet<String>,
}

#[test]
fn the_reference_corpus_cannot_change_its_text_or_failed_rules() {
    let language = Language::from_codes("ru", "en").unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/explanation/judge.json")).unwrap();
    let failures = cases
        .into_iter()
        .filter_map(|case| {
            let (fails, markdown) = judge(&case.card, &case.parts, &language).into_parts();
            let rules = fails
                .iter()
                .map(|fail| fail.split(':').next().unwrap().to_string())
                .collect::<BTreeSet<_>>();
            (rules != case.fails || markdown != case.markdown).then_some((
                case.id,
                rules,
                case.fails,
                markdown == case.markdown,
            ))
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "the reference explanations changed: {failures:#?}"
    );
}

#[test]
fn tidying_and_quoting_cannot_change_a_second_time() {
    let language = Language::from_codes("ru", "en").unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/explanation/judge.json")).unwrap();
    assert!(
        cases
            .into_iter()
            .all(|case| tidy(&tidy(&case.parts)) == tidy(&case.parts)
                && quote(&quote(&case.markdown, &language), &language)
                    == quote(&case.markdown, &language)
                && (case
                    .fails
                    .is_empty()
                    .then(|| kamishibai::markdown::parse_markdown(&compose(&case.parts, &language)))
                    .is_none_or(|blocks| blocks.iter().all(|block| match block {
                        kamishibai::markdown::Block::Bullet { chunks, .. } =>
                            chunks.iter().all(|chunk| !chunk.bold && !chunk.italic),
                        kamishibai::markdown::Block::Paragraph(chunks) =>
                            chunks.iter().enumerate().all(|(index, chunk)| !chunk.italic
                                && (!chunk.bold
                                    || (index == 0
                                        && [
                                            "Значение.",
                                            "Где встречается.",
                                            "Где неуместно.",
                                            "Нюанс."
                                        ]
                                        .contains(&chunk.text.as_str())))),
                    })))),
        "normalization changed an already normalized explanation or accepted prose gained emphasis"
    );
}

#[test]
fn markers_cannot_escape_the_readers_quotation_marks() {
    let language = Language::from_codes("de", "en").unwrap();
    assert_eq!(
        quote("Man sagt ⟦charged with theft⟧.", &language),
        "Man sagt „charged with theft“.",
        "learning words escaped the reader's quotation marks"
    );
}

#[test]
fn new_html_cannot_lose_meaning_contrast_or_split_an_example() {
    use kamishibai::markdown::{parse_card_context, to_html};
    let context = "**Meaning.**\n- duck — a bird near the pond\n- hoax — a newspaper story\n\n**Subtlety.**\nA short fact.\n\n“A duck swims.”\n\n— Une cane nage.";
    assert_eq!(
        to_html(&parse_card_context(context)),
        "<p style=\"margin: 0.55em 0; line-height: 1.5;\"><strong>Meaning.</strong></p><ul style=\"margin: 0.45em 0; padding-left: 1.2em; line-height: 1.5;\"><li style=\"color: var(--kamishibai-term, inherit);\">duck — a bird near the pond</li><li style=\"color: var(--kamishibai-hint, inherit);\">hoax — a newspaper story</li></ul><p style=\"margin: 0.55em 0; line-height: 1.5;\"><strong>Subtlety.</strong> A short fact.</p><p style=\"margin: 0.55em 0; line-height: 1.5;\">“A duck swims.”<br>— Une cane nage.</p>",
        "the new note lost its inline colors, spacing, or joined example"
    );
}

#[test]
fn legacy_html_cannot_change_even_one_byte() {
    use kamishibai::markdown::{parse_card_context, to_html};
    assert_eq!(
        to_html(&parse_card_context(
            "**Meaning.**\n- **duck**\n- hoax\n\nAn old explanation."
        )),
        "<p style=\"margin: 0.4em 0;\"><strong>Meaning.</strong></p><ul style=\"margin: 0.4em 0; padding-left: 1.2em;\"><li><strong>duck</strong></li><li>hoax</li></ul><p style=\"margin: 0.4em 0;\">An old explanation.</p>",
        "legacy note HTML changed, which would change its identity"
    );
}

#[test]
fn old_and_new_note_fields_and_terminal_lines_keep_their_rendering_snapshots() {
    use kamishibai::markdown::{parse_card_context, to_html, to_ratatui};
    let cards: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/explanation/cards.json")).unwrap();
    let contexts = [
        ("new", cards[0]["markdown"].as_str().unwrap()),
        (
            "old",
            "**Значение.**\n- **граница**\n- черта на песке\n\n**Где встречается.**\nНа переговорах.",
        ),
    ];
    let snapshots = contexts.into_iter().map(|(kind, context)| {
        let blocks = parse_card_context(context);
        let lines = to_ratatui(&blocks).into_iter().map(|line| line.spans.into_iter().map(|span| {
            let color = match span.style.fg { Some(ratatui::style::Color::Rgb(red, green, blue)) => Some([red,green,blue]), _ => None };
            serde_json::json!({"text":span.content,"bold":span.style.add_modifier.contains(ratatui::style::Modifier::BOLD),"color":color})
        }).collect::<Vec<_>>()).collect::<Vec<_>>();
        (kind, serde_json::json!({"html":to_html(&blocks),"terminal":lines}))
    }).collect::<std::collections::BTreeMap<_,_>>();
    insta::assert_snapshot!(
        "explanation_renderings",
        serde_json::to_string_pretty(&snapshots).unwrap()
    );
}

#[test]
fn empty_sections_cannot_leave_a_heading() {
    let language = Language::from_codes("ru", "en").unwrap();
    let parts: Parts = serde_json::from_value(serde_json::json!({"meanings":[{"head":"утка","note":"птица у пруда","from":"утка"}],"literal":"","where":{"text":"","example":{"text":"","translation":""}},"limits":{"text":"","example":{"text":"","translation":""}},"nuance":{"text":"","why":"","example":{"text":"","translation":""}}})).unwrap();
    assert_eq!(
        compose(&parts, &language),
        "**Значение.**\n- утка — птица у пруда",
        "empty sections left headings on the card"
    );
}

#[test]
fn rules_without_rejections_in_the_corpus_cannot_silently_accept_them() {
    let language = Language::from_codes("ru", "en").unwrap();
    let card = Card::new(
        String::from("duck"),
        String::from("утка"),
        String::from("The duck swims."),
        Vec::new(),
    );
    let base = serde_json::json!({"meanings":[{"head":"утка","note":"птица у пруда","from":"утка"}],"literal":"","where":{"text":"","example":{"text":"","translation":""}},"limits":{"text":"","example":{"text":"","translation":""}},"nuance":{"text":"","why":"","example":{"text":"","translation":""}}});
    let changes = [
        (
            "structure",
            "/where/text",
            "В парке.\n\n**Чужое.**\n\nПтица стоит у воды.",
        ),
        ("highlight", "/meanings/0/head", "утка\nсрыв"),
        ("markup", "/meanings/0/note", "**птица** у пруда"),
        ("quotes", "/where/text", "Скажи ⟧duck рядом у пруда."),
        ("empty", "/limits/text", "Ничего особенного."),
        ("semicolon", "/meanings/0/note", "птица; у пруда"),
        ("quotation marks", "/where/text", "Скажи \"утка\" у пруда."),
        ("formal address", "/where/text", "Вы видите птицу у пруда."),
    ];
    let baseline: Parts = serde_json::from_value(base.clone()).unwrap();
    let mut failures = changes
        .into_iter()
        .filter_map(|(rule, pointer, value)| {
            let mut changed = base.clone();
            *changed.pointer_mut(pointer).unwrap() = value.into();
            let parts: Parts = serde_json::from_value(changed).unwrap();
            let (fails, _) = judge(&card, &parts, &language).into_parts();
            (!fails
                .iter()
                .any(|failure| failure.starts_with(&format!("{rule}:"))))
            .then_some(rule)
        })
        .collect::<Vec<_>>();
    if !judge(&card, &baseline, &language).into_parts().0.is_empty() {
        failures.push("baseline");
    }
    let mut repeated = base;
    repeated["meanings"][0]["note"] = "птица ловит мелкую рыбу на пруду".into();
    repeated["where"]["text"] = "Птица ловит мелкую рыбу на пруду.".into();
    let parts: Parts = serde_json::from_value(repeated).unwrap();
    if !judge(&card, &parts, &language)
        .into_parts()
        .0
        .iter()
        .any(|failure| failure.starts_with("repeat:"))
    {
        failures.push("repeat");
    }
    assert!(
        failures.is_empty(),
        "rules without corpus rejections accepted invalid strings: {failures:?}"
    );
}

#[test]
fn every_catalog_language_cannot_lose_its_quotation_or_literal_convention() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../assets/prompt_examples.json")).unwrap();
    assert!(
        kamishibai::languages::catalog()
            .codes()
            .into_iter()
            .all(|code| {
                let context = &catalog[code.to_ascii_lowercase()]["card"]["context"];
                let quotes = context["quotes"].as_array().unwrap();
                let language = Language::from_codes(code, "EN").unwrap();
                quote("⟦duck⟧", &language)
                    == format!(
                        "{}duck{}",
                        quotes[0].as_str().unwrap(),
                        quotes[1].as_str().unwrap()
                    )
                    && !context["literally"].as_str().unwrap().is_empty()
            }),
        "a supported language lost its complete explanation conventions"
    );
}

#[test]
fn reviewed_meanings_cannot_be_dropped_added_or_reordered() {
    let language = Language::from_codes("en", "fr").unwrap();
    let card = Card::new(
        String::from("chat"),
        String::from("a cat"),
        String::from("Le chat dort."),
        vec![kamishibai::explanation::Sense::new(
            String::from("1"),
            String::from("a cat"),
            String::new(),
        )],
    );
    let mut valid = cat_response();
    let variants = [
        valid.clone(),
        {
            valid["meanings"][0]["from"] = "2".into();
            valid.clone()
        },
        {
            valid["meanings"] = serde_json::json!([]);
            valid.clone()
        },
        {
            valid["meanings"] = serde_json::json!([
                cat_response()["meanings"][0].clone(),
                cat_response()["meanings"][0].clone()
            ]);
            valid
        },
    ];
    assert_eq!(
        variants
            .into_iter()
            .map(|value| {
                let parts: Parts = serde_json::from_value(value).unwrap();
                judge(&card, &parts, &language)
                    .into_parts()
                    .0
                    .iter()
                    .any(|fail| fail.starts_with("list:"))
            })
            .collect::<Vec<_>>(),
        [false, true, true, true],
        "the reviewed meaning count or order was not enforced"
    );
}

#[test]
fn learning_markers_cannot_hide_a_mistake_from_the_reason_rule() {
    let language = Language::from_codes("ru", "en").unwrap();
    let card = Card::new(
        String::from("charge"),
        String::from("обвинять"),
        String::from("Police charge him."),
        Vec::new(),
    );
    let parts: Parts = serde_json::from_value(serde_json::json!({"meanings":[{"head":"обвинять","note":"в краже денег, в нападении","from":"обвинять"}],"literal":"","where":{"text":"","example":{"text":"","translation":""}},"limits":{"text":"","example":{"text":"","translation":""}},"nuance":{"text":"Говорят ⟦charged with theft⟧, а не ⟦of theft⟧.","why":"⟦With⟧ называет причину обвинения.","example":{"text":"","translation":""}}})).unwrap();
    assert!(
        judge(&card, &parts, &language).into_parts().0.is_empty(),
        "learning markers hid a valid mistake and its reason from the judge"
    );
}

#[test]
fn explanation_provenance_cannot_be_lost_when_metadata_is_cached() {
    use kamishibai::session::{CardMeta, CardMetaCache, LanguagePair};
    let root = tempfile::tempdir().unwrap();
    let cache = CardMetaCache::new(root.path());
    let pair = LanguagePair::new("FR", "EN");
    let parts: Parts = serde_json::from_value(cat_response()).unwrap();
    let meta = CardMeta::new(
        "ipa",
        "ipa",
        "a cat",
        5,
        "The cat sleeps",
        "cat",
        "clue",
        "new explanation",
        "Le chat dort",
    )
    .with_explanation(Some(kamishibai::explanation::Record::new(
        Some(parts),
        String::from("task stamp"),
        String::from("metadata draft"),
    )));
    cache.store("chat", "a cat", &pair, &meta).unwrap();
    let restored = cache.load("chat", "a cat", &pair).unwrap().unwrap();
    assert_eq!(
        (&restored, restored.metadata_context()),
        (&meta, "metadata draft"),
        "cache persistence lost the accepted parts, task stamp, or correction draft"
    );
}

#[test]
fn a_host_without_reviewed_senses_cannot_send_an_empty_approved_inventory() {
    let input = kamishibai::explanation::Input::new(
        Card::new(
            String::from("chat"),
            String::from("a cat"),
            String::from("Le chat dort"),
            Vec::new(),
        ),
        String::from("The cat sleeps"),
        String::from("old explanation"),
        String::new(),
    );
    assert!(
        serde_json::to_value(input).unwrap().get("senses").is_none(),
        "a host without reviewed senses sent an empty approved list instead of using translation meanings"
    );
}

#[test]
fn non_latin_readers_cannot_be_rejected_by_russian_word_matching() {
    let cases = [
        ("EL", "πάπια", "πουλί που ζει στη λίμνη"),
        ("AR", "بطة", "طائر يعيش قرب البركة"),
        ("ZH", "鸭子", "池塘边游泳的水鸟"),
    ];
    let failures = cases
        .into_iter()
        .filter_map(|(known, head, note)| {
            let language = Language::from_codes(known, "EN").unwrap();
            let card = Card::new(
                String::from("duck"),
                head.to_string(),
                String::from("A duck swims"),
                Vec::new(),
            );
            let mut value = cat_response();
            value["meanings"][0] = serde_json::json!({"head":head,"note":note,"from":head});
            let parts: Parts = serde_json::from_value(value).unwrap();
            let (fails, _) = judge(&card, &parts, &language).into_parts();
            (!fails.is_empty()).then_some((known, fails))
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "a reader's script was treated as an empty or repeated definition: {failures:?}"
    );
}

struct Writer {
    answers: std::collections::VecDeque<String>,
    prompts: Vec<String>,
}

impl kamishibai::explanation::Writer for Writer {
    fn ask(&mut self, prompt: String) -> anyhow::Result<String> {
        self.prompts.push(prompt);
        Ok(self
            .answers
            .pop_front()
            .expect("unexpected fifth explanation request"))
    }
}

#[test]
fn repair_requests_cannot_omit_the_judges_actionable_findings() {
    use kamishibai::explanation::{Input, Task, write};
    let language = Language::from_codes("ru", "en").unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/explanation/judge.json")).unwrap();
    let case = cases
        .into_iter()
        .find(|case| !case.fails.is_empty())
        .unwrap();
    let (findings, _) = judge(&case.card, &case.parts, &language).into_parts();
    let task = Task::from_input(
        Input::new(
            case.card,
            String::from("sentence"),
            String::from("metadata facts"),
            String::from("learner request"),
        ),
        &language,
    )
    .unwrap();
    let raw = serde_json::to_string(&case.parts).unwrap();
    let mut writer = Writer {
        answers: vec![raw; 4].into(),
        prompts: Vec::new(),
    };
    let replacement = write(&mut writer, &task, &language).unwrap();
    assert!(
        replacement.is_none()
            && findings
                .iter()
                .all(|finding| writer.prompts[1].contains(finding)
                    && writer.prompts[3].contains(finding))
            && writer
                .prompts
                .iter()
                .all(|prompt| prompt.contains("metadata facts")
                    && prompt.contains("learner request")),
        "a repair lost its findings, metadata facts, or learner request"
    );
}

#[test]
fn writing_cannot_exceed_four_calls_or_install_an_unaccepted_answer() {
    use kamishibai::explanation::{Input, Task, write};
    let language = Language::from_codes("ru", "en").unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/explanation/judge.json")).unwrap();
    let clean = cases.iter().find(|case| case.fails.is_empty()).unwrap();
    let broken = cases.iter().find(|case| !case.fails.is_empty()).unwrap();
    let good = serde_json::to_string(&clean.parts).unwrap();
    let bad = serde_json::to_string(&broken.parts).unwrap();
    let input = Input::new(
        clean.card.clone(),
        String::from("sentence"),
        String::from("old draft"),
        String::new(),
    );
    let task = Task::from_input(input, &language).unwrap();
    let routes = [
        (vec![good.clone(), good.clone()], true, 2),
        (vec![bad.clone(), good.clone(), good.clone()], true, 3),
        (vec![good.clone(), bad.clone(), good.clone()], true, 3),
        (
            vec![bad.clone(), bad.clone(), bad.clone(), bad.clone()],
            false,
            4,
        ),
        (vec![String::from("unparsed")], false, 1),
        (
            vec![bad.clone(), String::from("unparsed"), good.clone()],
            true,
            3,
        ),
        (vec![good.clone(), String::from("unparsed")], true, 2),
        (vec![good, bad, String::from("unparsed")], false, 3),
    ];
    let results = routes
        .into_iter()
        .map(|(answers, expected, count)| {
            let mut writer = Writer {
                answers: answers.into(),
                prompts: Vec::new(),
            };
            let written = write(&mut writer, &task, &language).unwrap();
            (written.is_some() == expected, writer.prompts.len() == count)
        })
        .collect::<Vec<_>>();
    assert!(
        results.iter().all(|(accepted, count)| *accepted && *count),
        "an explanation path exceeded its call budget or installed rejected text: {results:?}"
    );
}
