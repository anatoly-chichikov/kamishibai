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
    serde_json::from_value(serde_json::json!({"term":card["term"],"meaning":card["translation"],"pronunciation":card["pronunciation"],"transcription":"sample","importance":7,"source":{"sentence":card["sentence_known"],"lang":"RU","highlight":"крайняя черта","hint":"A boundary on the sand","context":card["markdown"],"context_format":"explanation_v1"},"target":{"sentence":card["sentence"],"lang":"EN"}})).unwrap()
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
    use kamishibai::vocabulary::ContextFormat;
    let context = "**Meaning.**\n- duck — a bird near the pond\n- hoax — a newspaper story\n\n**Subtlety.**\nA short fact.\n\n“A duck swims.”\n\n— Une cane nage.";
    assert_eq!(
        ContextFormat::ExplanationV1.html(context),
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
fn review_f1_unweighted_imported_contexts_cannot_change_legacy_note_identity() {
    use kamishibai::anki::{CardModel, NoteFormat, VocabularyNote};
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/explanation/legacy/cases.json")).unwrap();
    let format = VocabularyNote::new(CardModel::new().model());
    let changed = cases
        .into_iter()
        .filter_map(|case| {
            let entry = serde_json::from_value(case["entry"].clone()).unwrap();
            let note = format.note(&entry, "[sound:a.wav]", "");
            (note.guid != case["guid"] || note.fields[9] != case["html"])
                .then_some(case["name"].clone())
        })
        .collect::<Vec<_>>();
    assert!(
        changed.is_empty(),
        "valid 1.18.0 imports gained new note identities: {changed:?}"
    );
}

#[test]
fn review_f1_explicit_formats_cannot_be_lost_in_import_cache_or_result_json() {
    use kamishibai::session::{CardMetaCache, LanguagePair, from_entry, to_entry};
    use kamishibai::vocabulary::ContextFormat;
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/explanation/legacy/cases.json")).unwrap();
    let restored = [ContextFormat::Legacy, ContextFormat::ExplanationV1]
        .into_iter()
        .map(|format| {
            let root = tempfile::tempdir().unwrap();
            let cache = CardMetaCache::new(root.path());
            let mut entry: kamishibai::vocabulary::VocabularyEntry =
                serde_json::from_value(cases[2]["entry"].clone()).unwrap();
            entry.source.context_format = format;
            let pair = LanguagePair::new("FR", "EN");
            let draft = from_entry(&entry, pair.clone());
            cache
                .store("chat", "a cat", &pair, draft.meta().unwrap())
                .unwrap();
            let meta = cache.load("chat", "a cat", &pair).unwrap().unwrap();
            let output = to_entry(&draft.with_meta(meta, None)).unwrap();
            let json = serde_json::to_value(&output).unwrap();
            let decoded: kamishibai::vocabulary::VocabularyEntry =
                serde_json::from_value(json.clone()).unwrap();
            decoded == entry
                && json["source"].get("context_format").is_some()
                    == (format == ContextFormat::ExplanationV1)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        restored,
        [true, true],
        "an import or cache round trip lost the format or added a legacy discriminator"
    );
}

#[test]
fn review_f1_fallback_provenance_cannot_enable_the_new_format() {
    use kamishibai::session::CardMeta;
    use kamishibai::vocabulary::ContextFormat;
    let context = "**Meaning.**\n- a cat — a furry pet on a windowsill\n\n**Usage.**\nAt home.";
    let formats = [None, Some(serde_json::from_value(cat_response()).unwrap())]
        .into_iter()
        .map(|parts| {
            let meta = CardMeta::new(
                "ipa",
                "ipa",
                "a cat",
                7,
                "The cat sleeps",
                "cat",
                "a furry pet",
                context,
                "Le chat dort",
            )
            .with_explanation(Some(kamishibai::explanation::Record::new(
                parts,
                "stamp".into(),
                context.into(),
            )));
            let mut json = serde_json::to_value(meta).unwrap();
            json.as_object_mut().unwrap().remove("context_format");
            serde_json::from_value::<CardMeta>(json)
                .unwrap()
                .context_format()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        formats,
        [ContextFormat::Legacy, ContextFormat::ExplanationV1],
        "fallback provenance enabled the new renderer or accepted 1.19.0 provenance lost it"
    );
}

/// Run explicitly with ANKI_PYTHON pointing to Python with Anki on its module path.
#[test]
#[ignore = "requires the native Anki Python engine; run with ANKI_PYTHON and --ignored"]
fn review_f1_native_anki_reimports_cannot_duplicate_legacy_notes() {
    use kamishibai::anki::{CardModel, VocabularyDeck, VocabularyNote};
    let directory = tempfile::tempdir().unwrap();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/explanation/legacy/cases.json")).unwrap();
    for case in cases {
        let entry = serde_json::from_value(case["entry"].clone()).unwrap();
        let mut deck = VocabularyDeck::new(
            1525230255,
            "Compatibility Review",
            VocabularyNote::new(CardModel::new().model()),
            Vec::<(std::path::PathBuf, String)>::new(),
        );
        deck.add(&entry, "[sound:a.wav]", "");
        deck.save(
            directory
                .path()
                .join(format!("{}.apkg", case["name"].as_str().unwrap())),
        )
        .unwrap();
    }
    let mut child = std::process::Command::new(
        std::env::var_os("ANKI_PYTHON")
            .expect("ANKI_PYTHON must name Python with the native Anki package"),
    )
    .args([
        "-c",
        r#"
from contextlib import closing
import json
from pathlib import Path
import sys
from anki.collection import Collection
from anki.import_export_pb2 import ImportAnkiPackageRequest
root, fixtures = map(Path, sys.argv[1:])
results = []
for case in json.loads((fixtures / 'cases.json').read_text()):
    name = case['name']
    with closing(Collection(str(root / (name + '.anki2')))) as collection:
        old = fixtures / (name + '-1.18.0.apkg')
        new = root / (name + '.apkg')
        counts = []
        for package in [old, old, new, new]:
            collection.import_anki_package(ImportAnkiPackageRequest(package_path=str(package)))
            counts.append((collection.note_count(), collection.card_count()))
        results.append(counts)
        print(json.dumps({'case': name, 'counts': counts}), flush=True)
assert results == [[(1, 1)] * 4] * 3, 'cross-version imports duplicated a legacy note'
"#,
    ])
    .arg(directory.path())
    .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/explanation/legacy"))
    .spawn()
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let success = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status.success();
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        success,
        "the native Anki import regression failed or exceeded its deadline"
    );
}

#[test]
fn old_and_new_note_fields_and_terminal_lines_keep_their_rendering_snapshots() {
    use kamishibai::vocabulary::ContextFormat;
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
        let format = if kind == "new" { ContextFormat::ExplanationV1 } else { ContextFormat::Legacy };
        let lines = format.terminal(context).into_iter().map(|line| line.spans.into_iter().map(|span| {
            let color = match span.style.fg { Some(ratatui::style::Color::Rgb(red, green, blue)) => Some([red,green,blue]), _ => None };
            serde_json::json!({"text":span.content,"bold":span.style.add_modifier.contains(ratatui::style::Modifier::BOLD),"color":color})
        }).collect::<Vec<_>>()).collect::<Vec<_>>();
        (kind, serde_json::json!({"html":format.html(context),"terminal":lines}))
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

fn review_fixture(head: &str, note: &str) -> serde_json::Value {
    let mut value = cat_response();
    value["meanings"][0]["head"] = head.into();
    value["meanings"][0]["note"] = note.into();
    value
}

fn reviewed_card(term: &str, head: &str) -> Card {
    Card::new(
        term.into(),
        head.into(),
        "A different sentence.".into(),
        vec![kamishibai::explanation::Sense::new(
            "1".into(),
            head.into(),
            String::new(),
        )],
    )
}

fn reviewed_failures(
    value: serde_json::Value,
    known: &str,
    learning: &str,
    card: &Card,
) -> Vec<String> {
    let parts: Parts = serde_json::from_value(value).unwrap();
    judge(
        card,
        &tidy(&parts),
        &Language::from_codes(known, learning).unwrap(),
    )
    .into_parts()
    .0
}

#[test]
fn review_f2_greek_mistakes_cannot_reject_their_relevant_reason() {
    let failures = [
        "Говорят ⟦στην Ελλάδα⟧, а не ⟦σε Ελλάδα⟧.",
        "В Греции не говорят ⟦σε Ελλάδα⟧, говорят ⟦στην Ελλάδα⟧.",
    ]
    .into_iter()
    .flat_map(|text| {
        let mut value = review_fixture("Греция", "страна у Эгейского моря");
        value["nuance"]["text"] = text.into();
        value["nuance"]["why"] = "⟦Στην⟧ называет страну как место.".into();
        reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
    })
    .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "a Greek correction lost its explanation: {failures:?}"
    );
}

#[test]
fn review_f2_greek_mistakes_cannot_pass_without_a_reason() {
    let mut value = review_fixture("Греция", "страна у Эгейского моря");
    value["nuance"]["text"] = "Говорят ⟦στην Ελλάδα⟧, а не ⟦σε Ελλάδα⟧.".into();
    assert!(
        reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
            .iter()
            .any(|fail| fail.starts_with("why:")),
        "a Greek mistake escaped the reason obligation"
    );
}

#[test]
fn review_f2_reader_words_cannot_make_an_unrelated_reason_relevant() {
    let mut value = review_fixture("Греция", "страна у Эгейского моря");
    value["nuance"]["text"] = "В Греции не говорят ⟦σε Ελλάδα⟧, говорят ⟦στην Ελλάδα⟧.".into();
    value["nuance"]["why"] = "В Греции ⟦από⟧ называет начало пути.".into();
    assert!(
        reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
            .iter()
            .any(|fail| fail.starts_with("why:")),
        "shared reader words accepted an unrelated Greek reason"
    );
}

#[test]
fn review_f2_shared_scripts_cannot_hide_marked_mistakes_or_borrow_reader_words() {
    let outcomes = [
        "⟦В⟧ называет страну как место.",
        "⟦З⟧ называет страну как место.",
        "",
    ]
    .into_iter()
    .map(|reason| {
        let mut value = review_fixture("Украина", "страна у Чёрного моря");
        value["nuance"]["text"] = "Говорят ⟦в Україні⟧, а не ⟦на Україні⟧.".into();
        value["nuance"]["why"] = reason.into();
        reviewed_failures(value, "RU", "UK", &reviewed_card("Україна", "Украина"))
            .iter()
            .any(|fail| fail.starts_with("why:"))
    })
    .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        [false, true, true],
        "shared Cyrillic scripts hid the mistake or confused reader prose with its reason"
    );
}

#[test]
fn review_f2r_correct_reasons_cannot_depend_on_the_quotation_form() {
    let failures = [
        "Говорят ⟦στην Ελλάδα⟧, а не ⟦σε Ελλάδα⟧.",
        "Говорят «στην Ελλάδα», а не «σε Ελλάδα».",
    ]
    .into_iter()
    .flat_map(|nuance| {
        [
            "⟦Στην⟧ называет страну как место.",
            "«Στην» называет страну как место.",
        ]
        .into_iter()
        .flat_map(move |reason| {
            let mut value = review_fixture("Греция", "страна у Эгейского моря");
            value["nuance"]["text"] = nuance.into();
            value["nuance"]["why"] = reason.into();
            reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
        })
    })
    .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "reader quotes changed a correct reason into a refusal: {failures:?}"
    );
}

#[test]
fn review_f2r_missing_reasons_cannot_hide_behind_reader_quotes() {
    let missed = [
        "Говорят ⟦στην Ελλάδα⟧, а не ⟦σε Ελλάδα⟧.",
        "Говорят «στην Ελλάδα», а не «σε Ελλάδα».",
    ]
    .into_iter()
    .filter(|nuance| {
        let mut value = review_fixture("Греция", "страна у Эгейского моря");
        value["nuance"]["text"] = (*nuance).into();
        !reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
            .iter()
            .any(|fail| fail.starts_with("why:"))
    })
    .collect::<Vec<_>>();
    assert!(
        missed.is_empty(),
        "quotation syntax hid the reason obligation: {missed:?}"
    );
}

#[test]
fn review_f2r_unrelated_reasons_cannot_pass_in_either_quotation_form() {
    let outcomes = [
        "Говорят ⟦στην Ελλάδα⟧, а не ⟦σε Ελλάδα⟧.",
        "Говорят «στην Ελλάδα», а не «σε Ελλάδα».",
    ]
    .into_iter()
    .flat_map(|nuance| {
        ["⟦από⟧ называет начало пути.", "«από» называет начало пути."]
            .into_iter()
            .map(move |reason| {
                let mut value = review_fixture("Греция", "страна у Эгейского моря");
                value["nuance"]["text"] = nuance.into();
                value["nuance"]["why"] = reason.into();
                reviewed_failures(value, "RU", "EL", &reviewed_card("Ελλάδα", "Греция"))
                    .iter()
                    .any(|fail| fail.starts_with("why:"))
            })
    })
    .collect::<Vec<_>>();
    assert_eq!(
        outcomes, [true; 4],
        "a quoted unrelated reason passed the obligation"
    );
}

#[test]
fn review_f2r_shared_scripts_cannot_borrow_reader_words_outside_quotes() {
    let outcomes = [
        "Говорят ⟦в Україні⟧, а не ⟦на Україні⟧.",
        "Говорят «в Україні», а не «на Україні».",
    ]
    .into_iter()
    .flat_map(|nuance| {
        [
            "⟦В⟧ называет страну как место.",
            "«В» называет страну как место.",
            "⟦З⟧ называет страну как место.",
            "«З» называет страну как место.",
            "",
        ]
        .into_iter()
        .map(move |reason| {
            let mut value = review_fixture("Украина", "страна у Чёрного моря");
            value["nuance"]["text"] = nuance.into();
            value["nuance"]["why"] = reason.into();
            reviewed_failures(value, "RU", "UK", &reviewed_card("Україна", "Украина"))
                .iter()
                .any(|fail| fail.starts_with("why:"))
        })
    })
    .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        [
            false, false, true, true, true, false, false, true, true, true
        ],
        "shared scripts confused quoted learning words with reader prose"
    );
}

#[test]
fn review_f2r_reader_glosses_cannot_be_mistaken_for_learning_phrases() {
    let language = Language::from_codes("RU", "EN").unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/explanation/judge.json")).unwrap();
    let failures = cases
        .into_iter()
        .filter(|case| ["3.t2.A14.parts", "4.r1.K57.parts"].contains(&case.id.as_str()))
        .flat_map(|case| judge(&case.card, &case.parts, &language).into_parts().0)
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "reader-language quotations hid a learning word or introduced a false mistake: {failures:?}"
    );
}

#[test]
fn review_f2r_writing_cannot_accept_a_quoted_mistake_without_a_relevant_reason() {
    use kamishibai::explanation::{Input, Task, write};
    let language = Language::from_codes("RU", "EL").unwrap();
    let task = Task::from_input(
        Input::new(
            reviewed_card("Ελλάδα", "Греция"),
            "Предложение карточки.".into(),
            "original context".into(),
            String::new(),
        ),
        &language,
    )
    .unwrap();
    let outcomes = [("⟦", "⟧"), ("«", "»")]
        .into_iter()
        .flat_map(|(open, close)| {
            ["Στην", "", "από"]
                .into_iter()
                .map(move |word| (open, close, word))
        })
        .map(|(open, close, word)| {
            let mut value = review_fixture("Греция", "страна у Эгейского моря");
            value["nuance"]["text"] =
                format!("Говорят {open}στην Ελλάδα{close}, а не {open}σε Ελλάδα{close}.").into();
            value["nuance"]["why"] = if word.is_empty() {
                String::new()
            } else {
                format!("{open}{word}{close} называет страну как место.")
            }
            .into();
            let mut writer = Writer {
                answers: vec![value.to_string(); 4].into(),
                prompts: Vec::new(),
            };
            let result = write(&mut writer, &task, &language).unwrap();
            let repair = [1, 3].into_iter().all(|index| {
                writer
                    .prompts
                    .get(index)
                    .is_some_and(|prompt| prompt.contains("why:"))
            });
            (result.is_some(), writer.prompts.len(), repair)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        [
            (true, 2, false),
            (false, 4, true),
            (false, 4, true),
            (true, 2, false),
            (false, 4, true),
            (false, 4, true)
        ],
        "the full write transaction accepted an incomplete explanation or lost its why repair"
    );
}

#[test]
fn review_r1_judging_cannot_drop_unquoted_learning_words_beside_quoted_phrases() {
    let failures = ["EN", "EL"]
        .into_iter()
        .flat_map(|learning| {
            let language = Language::from_codes("RU", learning).unwrap();
            mixed_cases(learning)
                .into_iter()
                .filter_map(move |(card, parts, accepted)| {
                    let failures = judge(&card, &parts, &language).into_parts().0;
                    let correct = if accepted {
                        failures.is_empty()
                    } else {
                        failures.iter().any(|failure| failure.starts_with("why:"))
                    };
                    (!correct).then_some((learning, parts, failures))
                })
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "mixed quotation changed reason relevance: {failures:?}"
    );
}

#[test]
fn review_r1_writing_cannot_spend_repairs_on_a_correct_mixed_reason() {
    use kamishibai::explanation::{Input, Task, write};
    let failures = ["EN", "EL"]
        .into_iter()
        .flat_map(|learning| {
            let language = Language::from_codes("RU", learning).unwrap();
            mixed_cases(learning)
                .into_iter()
                .filter_map(move |(card, parts, accepted)| {
                    let task = Task::from_input(
                        Input::new(
                            card,
                            "Предложение карточки.".into(),
                            "metadata draft".into(),
                            String::new(),
                        ),
                        &language,
                    )
                    .unwrap();
                    let mut writer = Writer {
                        answers: vec![serde_json::to_string(&parts).unwrap(); 4].into(),
                        prompts: Vec::new(),
                    };
                    let result = write(&mut writer, &task, &language).unwrap();
                    let expected = if accepted {
                        (Some(parts.clone()), 2)
                    } else {
                        (None, 4)
                    };
                    ((result, writer.prompts.len()) != expected).then_some((
                        learning,
                        parts,
                        writer.prompts.len(),
                    ))
                })
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "mixed quotation lost a valid explanation or accepted a missing or unrelated reason: {failures:?}"
    );
}

#[test]
fn review_r1_shared_scripts_cannot_borrow_unmarked_reader_words() {
    let mut value = review_fixture("Украина", "страна у Чёрного моря");
    value["nuance"]["text"] = "Говорят ⟦в Україні⟧, а не ⟦на Україні⟧.".into();
    value["nuance"]["why"] = "⟦Говорят⟧ описывает обычный выбор слова.".into();
    assert!(
        reviewed_failures(value, "RU", "UK", &reviewed_card("Україна", "Украина"))
            .iter()
            .any(|failure| failure.starts_with("why:")),
        "the reason borrowed a reader word outside marked learning phrases"
    );
}

fn mixed_cases(learning: &str) -> Vec<(Card, Parts, bool)> {
    let (term, head, note, right, wrong, reasons) = match learning {
        "EN" => (
            "shore",
            "берег",
            "край земли у воды",
            "on the shore",
            "in the shore",
            [
                "On показывает место на краю воды.",
                "In означало бы место внутри берега.",
                "",
                "At указывает на точку назначения.",
            ],
        ),
        "EL" => (
            "Ελλάδα",
            "Греция",
            "страна у Эгейского моря",
            "στην Ελλάδα",
            "σε Ελλάδα",
            [
                "Στην называет страну как место.",
                "Σε само по себе не называет страну как место.",
                "",
                "Από называет начало пути.",
            ],
        ),
        _ => panic!("unsupported mixed-quotation fixture language"),
    };
    [
        (right.to_string(), wrong.to_string()),
        (format!("«{right}»"), format!("«{wrong}»")),
        (format!("«{right}»"), wrong.to_string()),
        (right.to_string(), format!("«{wrong}»")),
        (format!("⟦{right}⟧"), wrong.to_string()),
        (right.to_string(), format!("⟦{wrong}⟧")),
    ]
    .into_iter()
    .flat_map(|(right, wrong)| {
        reasons.into_iter().enumerate().map(move |(index, reason)| {
            let mut value = review_fixture(head, note);
            value["nuance"]["text"] = format!("Говорят {right}, а не {wrong}.").into();
            value["nuance"]["why"] = reason.into();
            (
                reviewed_card(term, head),
                serde_json::from_value(value).unwrap(),
                index < 2,
            )
        })
    })
    .collect()
}

#[test]
fn review_f3_single_letter_heads_cannot_be_treated_as_empty() {
    let failures = [
        ("JA", "猫", "窓辺で日向ぼっこをする身近なペット"),
        ("JA", "水", "川や海を流れる透明な液体"),
        ("ZH", "火", "燃烧木材时出现的明亮火焰"),
        ("RU", "я", "говорящий человек называет себя"),
    ]
    .into_iter()
    .flat_map(|(known, head, note)| {
        reviewed_failures(
            review_fixture(head, note),
            known,
            "EN",
            &reviewed_card("word", head),
        )
    })
    .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "a natural single-letter head was rejected: {failures:?}"
    );
}

#[test]
fn review_f4_reversed_or_nested_quotes_cannot_pass_the_judge() {
    let missed = [
        ("EN", "At home people say ⟧chat⟦."),
        ("EN", "At home people say ⟦⟦chat⟧⟧."),
        ("EN", "At home people say ”chat“."),
        ("EN", "At home people say ““chat””."),
        ("EN", "At home people say “⟦chat⟧”."),
        ("FR", "Au bord du lac on dit »cat«."),
        ("FR", "Au bord du lac on dit ««cat»»."),
        (
            "FR",
            "Au bord du lac on dit «\u{a0}«\u{a0}cat\u{a0}»\u{a0}».",
        ),
    ]
    .into_iter()
    .filter(|(known, text)| {
        let mut value = cat_response();
        value["where"]["text"] = (*text).into();
        let learning = if *known == "FR" { "EN" } else { "FR" };
        !reviewed_failures(value, known, learning, &reviewed_card("chat", "a cat"))
            .iter()
            .any(|fail| fail.starts_with("quotes:"))
    })
    .collect::<Vec<_>>();
    assert!(
        missed.is_empty(),
        "unordered or nested quotes were accepted: {missed:?}"
    );
}

/// The six reviewed meanings used by the independent CLI explanation review.
pub(crate) fn six_meanings() -> (Card, Parts) {
    let senses = (1..=6)
        .map(|number| {
            kamishibai::explanation::Sense::new(
                number.to_string(),
                format!("sense {number}"),
                String::new(),
            )
        })
        .collect();
    let card = Card::new(
        "set".into(),
        "a group".into(),
        "This is a set.".into(),
        senses,
    );
    let mut value = cat_response();
    value["meanings"] = serde_json::json!([
        {"head":"a group","note":"books beside a window","from":"1"},
        {"head":"place","note":"put on a table","from":"2"},
        {"head":"harden","note":"cooling concrete","from":"3"},
        {"head":"sunset","note":"at the horizon","from":"4"},
        {"head":"prepare","note":"a dining table","from":"5"},
        {"head":"fixed","note":"the agreed price","from":"6"}
    ]);
    (card, serde_json::from_value(value).unwrap())
}

#[test]
fn review_f5_six_reviewed_senses_cannot_expand_the_five_line_limit() {
    let (card, parts) = six_meanings();
    let failures = judge(&card, &parts, &Language::from_codes("EN", "FR").unwrap())
        .into_parts()
        .0;
    assert!(
        failures
            .iter()
            .any(|failure| failure.starts_with("length:")),
        "six reviewed senses silently expanded the explanation limit: {failures:?}"
    );
}

#[test]
fn review_f5_six_senses_cannot_spend_any_explanation_calls() {
    use kamishibai::explanation::{Input, Task, write};
    let (card, parts) = six_meanings();
    let language = Language::from_codes("EN", "FR").unwrap();
    let task = Task::from_input(
        Input::new(
            card,
            "a sentence".into(),
            "metadata draft".into(),
            String::new(),
        ),
        &language,
    )
    .unwrap();
    let mut writer = Writer {
        answers: vec![serde_json::to_string(&parts).unwrap(); 4].into(),
        prompts: Vec::new(),
    };
    let written = write(&mut writer, &task, &language).unwrap();
    assert_eq!(
        (written, writer.prompts.len()),
        (None, 0),
        "six reviewed senses spent requests on an unsatisfiable explanation"
    );
}

#[test]
fn review_f5_unreviewed_groups_cannot_spend_requests_beyond_five() {
    use kamishibai::explanation::{Input, Task, write};
    let language = Language::from_codes("RU", "EN").unwrap();
    let calls = [
        "one; two; three; four; five",
        "one; two; three; four; five; six",
        ";one;;two;three;four;five;",
    ]
    .into_iter()
    .map(|translation| {
        let task = Task::from_input(
            Input::new(
                Card::new(
                    "set".into(),
                    translation.into(),
                    "This is a set.".into(),
                    Vec::new(),
                ),
                "sentence".into(),
                "complete metadata draft".into(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let mut writer = Writer {
            answers: vec!["unparseable".into()].into(),
            prompts: Vec::new(),
        };
        let result = write(&mut writer, &task, &language).unwrap();
        (result, writer.prompts.len())
    })
    .collect::<Vec<_>>();
    assert_eq!(
        calls,
        [(None, 1), (None, 0), (None, 1)],
        "the preflight changed the five-group boundary or counted empty groups"
    );
}

#[test]
fn review_f4_identical_hebrew_quote_marks_cannot_be_treated_as_nested() {
    let mut value = review_fixture("חתול", "חיית מחמד פרוותית ליד החלון");
    value["where"]["text"] = "בבית אומרים ⟦chat⟧ וגם ⟦minou⟧.".into();
    let failures = reviewed_failures(value, "HE", "FR", &reviewed_card("chat", "חתול"));
    assert!(
        failures.is_empty(),
        "balanced Hebrew high quotes failed review: {failures:?}"
    );
}

#[test]
fn review_f6_native_sentence_endings_cannot_gain_an_ascii_stop() {
    let changed = ['。', '！', '？', '؟', '।']
        .into_iter()
        .filter(|end| {
            let mut value = review_fixture("猫ちゃん", "窓辺で日向ぼっこをする身近なペット");
            let text = format!("家庭で飼っている動物について話すとき{end}");
            value["where"]["text"] = text.clone().into();
            let parts: Parts = serde_json::from_value(value).unwrap();
            serde_json::to_value(tidy(&parts)).unwrap()["where"]["text"] != text
        })
        .collect::<Vec<_>>();
    assert!(
        changed.is_empty(),
        "native sentence endings gained a second stop: {changed:?}"
    );
}

#[test]
fn review_f6_native_full_stops_cannot_pass_in_unpunctuated_notes() {
    let missed = ['。', '！', '？', '؟', '।']
        .into_iter()
        .filter(|end| {
            let value = review_fixture(
                "猫ちゃん",
                &format!("窓辺で日向ぼっこをする身近なペット{end}"),
            );
            !reviewed_failures(value, "JA", "EN", &reviewed_card("cat", "猫ちゃん"))
                .iter()
                .any(|fail| fail.starts_with("form:"))
        })
        .collect::<Vec<_>>();
    assert!(
        missed.is_empty(),
        "sentence punctuation escaped the note form rule: {missed:?}"
    );
}

#[test]
fn review_f6_native_sentence_endings_cannot_be_rejected_by_form() {
    let failures = ['。', '！', '？', '؟', '।']
        .into_iter()
        .flat_map(|end| {
            let mut value = review_fixture("猫ちゃん", "窓辺で日向ぼっこをする身近なペット");
            value["where"]["text"] = format!("家庭で飼っている動物について話すとき{end}").into();
            let parts: Parts = serde_json::from_value(value).unwrap();
            judge(
                &reviewed_card("cat", "猫ちゃん"),
                &parts,
                &Language::from_codes("JA", "EN").unwrap(),
            )
            .into_parts()
            .0
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "native sentence endings failed review: {failures:?}"
    );
}

#[test]
fn review_f7_hebrew_high_quotes_cannot_reverse_the_closing_mark() {
    assert_eq!(
        quote("⟦duck⟧", &Language::from_codes("HE", "EN").unwrap()),
        "”duck”",
        "Hebrew high quotation marks used a reversed English closer"
    );
}

#[test]
fn review_f8_french_quotes_cannot_break_away_from_the_phrase() {
    assert_eq!(
        quote("⟦duck⟧", &Language::from_codes("FR", "EN").unwrap()),
        "«\u{a0}duck\u{a0}»",
        "French quotation spaces can break away from their phrase"
    );
}

#[test]
fn review_f9_combining_accents_cannot_escape_their_quoted_word() {
    let language = Language::from_codes("RU", "EN").unwrap();
    assert_eq!(
        [quote("café", &language), quote("cafe\u{301}", &language)],
        ["«café»", "«cafe\u{301}»"],
        "a decomposed accent was detached from its quoted letter"
    );
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
