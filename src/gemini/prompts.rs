use anyhow::Result;
use serde_json::json;

use crate::application::LearningTarget;
use crate::languages::LanguageCatalog;
use crate::prompt::PromptTemplate;
use crate::session::{
    CardDraft, CardMeta, LanguagePair, SentenceAxis, SentenceLabelSelection, WordCandidate,
};

const INTAKE_PROMPT: &str = include_str!("../../assets/gemini_intake_prompt.txt");
const SENSE_PROMPT: &str = include_str!("../../assets/gemini_sense_prompt.txt");
const CARD_META_PROMPT: &str = include_str!("../../assets/gemini_card_meta_prompt.txt");
const CARD_PROMPT: &str = include_str!("../../assets/gemini_card_prompt.txt");
const PHONETICS_PROMPT: &str = include_str!("../../assets/gemini_phonetics_prompt.txt");
const LEARNER_EXPLANATIONS: &str = include_str!("../../assets/learner_explanations_prompt.txt");

/// Render the human-in-the-loop intake prompt.
pub(super) fn render_intake_prompt(
    raw: &str,
    known: &str,
    target: &LearningTarget,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let support = catalog.item(known)?;
    let examples = catalog.prompts(support.code)?;
    render(
        INTAKE_PROMPT,
        &[
            (
                "{learner_explanations}",
                learner_explanations(examples.writing()?)?,
            ),
            ("{supported_languages}", language_choices(catalog)?),
            ("{support_language}", language_label(catalog, support.code)?),
            ("{target_instruction}", target_instruction(target, catalog)?),
            (
                "{understanding_length}",
                String::from(examples.understanding_length()),
            ),
            ("{sense_conventions}", examples.sense_conventions()?),
            ("{intake_examples}", examples.intake()?),
            ("{raw_input}", String::from(raw)),
        ],
    )
}

/// Render the focused sense request prompt fired from add more.
pub(super) fn render_bulk_prompt(
    candidate: &WordCandidate,
    comment: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let support = pair.known_profile(catalog)?;
    let examples = catalog.prompts(support.code)?;
    let senses = candidate
        .senses()
        .iter()
        .map(|sense| json!({"understanding": sense.understanding(), "tag": sense.tag()}))
        .collect::<Vec<_>>();
    render(
        SENSE_PROMPT,
        &[
            (
                "{learner_explanations}",
                learner_explanations(examples.writing()?)?,
            ),
            (
                "{target_language}",
                language_label(catalog, pair.learning())?,
            ),
            ("{support_language}", language_label(catalog, support.code)?),
            (
                "{understanding_length}",
                String::from(examples.understanding_length()),
            ),
            ("{sense_conventions}", examples.sense_conventions()?),
            ("{sense_message_examples}", examples.sense_messages()?),
            ("{term}", String::from(candidate.term())),
            ("{shown_senses}", serde_json::to_string_pretty(&senses)?),
            ("{user_request}", String::from(comment)),
        ],
    )
}

/// Render the focused IPA review from the validated card and its settled reviewed senses.
pub(super) fn render_phonetics_prompt(
    draft: &CardDraft,
    meta: &CardMeta,
    pair: &LanguagePair,
) -> Result<String> {
    let senses = draft
        .reviewed_senses()
        .iter()
        .map(|sense| json!({"understanding": sense.understanding(), "tag": sense.tag()}))
        .collect::<Vec<_>>();
    let input = json!({
        "target_language": pair.learning(),
        "term": draft.term(),
        "reviewed_senses": senses,
        "selected": 0,
        "target_sentence": meta.target_sentence(),
        "pronunciation": meta.pronunciation(),
        "transcription": meta.transcription(),
    });
    render(
        PHONETICS_PROMPT,
        &[("{input_json}", serde_json::to_string_pretty(&input)?)],
    )
}

/// Render the card-meta generation prompt.
pub(super) fn render_card_meta_prompt(
    draft: &CardDraft,
    request: Option<&SentenceLabelSelection>,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let pair = draft.pair();
    let source = pair.known_profile(catalog)?;
    let examples = catalog.prompts(source.code)?;
    render(
        CARD_META_PROMPT,
        &[
            (
                "{learner_explanations}",
                learner_explanations(examples.writing()?)?,
            ),
            (
                "{target_language}",
                language_label(catalog, pair.learning())?,
            ),
            ("{source_language}", language_label(catalog, source.code)?),
            ("{hint_length}", String::from(examples.hint_length())),
            ("{hint_examples}", examples.hint()?),
            ("{context_examples}", examples.context()?),
            ("{term}", String::from(draft.term())),
            ("{understanding}", String::from(draft.understanding())),
            ("{reviewed_senses}", reviewed_senses(draft)?),
            (
                "{initial_approx_schema}",
                String::from(initial_approx_schema(request)),
            ),
            (
                "{initial_approx_rule}",
                String::from(initial_approx_rule(request)),
            ),
            (
                "{initial_sentence_preferences}",
                initial_sentence_preferences(request),
            ),
        ],
    )
}

fn reviewed_senses(draft: &CardDraft) -> Result<String> {
    let senses = draft
        .reviewed_senses()
        .iter()
        .enumerate()
        .map(|(index, sense)| {
            json!({
                "chosen": index == 0,
                "priority": draft.sense_priority(index),
                "understanding": sense.understanding(),
                "tag": sense.tag(),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&senses)?)
}

fn initial_approx_schema(request: Option<&SentenceLabelSelection>) -> &'static str {
    if has_initial_preferences(request) {
        "[\"<requested axis that could not be fulfilled exactly; omit fulfilled and unrequested axes>\"]"
    } else {
        "[]"
    }
}

fn initial_approx_rule(request: Option<&SentenceLabelSelection>) -> &'static str {
    if has_initial_preferences(request) {
        "`approx` is empty when every requested axis is fulfilled exactly; otherwise it contains only the requested axes whose returned natural sentence labels differ from the preset."
    } else {
        "`approx` is always an empty array."
    }
}

fn has_initial_preferences(request: Option<&SentenceLabelSelection>) -> bool {
    request.is_some_and(|request| !request.pinned().is_empty())
}

fn initial_sentence_preferences(request: Option<&SentenceLabelSelection>) -> String {
    let Some(request) = request.filter(|request| !request.pinned().is_empty()) else {
        return String::new();
    };
    format!(
        "\n\n  Initial sentence preset: {}. This explicit preset overrides the preceding no-target rule for exactly the named axes. Treat each named value as a constraint while preserving the approved term, sense, form, and naturally required register. Satisfy it exactly when possible; otherwise return the closest natural sentence and name only that requested axis in `approx`. Unnamed axes remain descriptive.",
        requested_labels(request)
    )
}

/// Render the per-card refinement prompt fired from the inline sentence editor.
pub(super) fn render_card_prompt(
    draft: &CardDraft,
    comment: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let source = pair.known_profile(catalog)?;
    let examples = catalog.prompts(source.code)?;
    let meta = draft
        .rewrite()
        .and_then(|rewrite| rewrite.previous().cloned())
        .or_else(|| draft.meta().cloned())
        .unwrap_or_else(empty_meta);
    let selection = draft
        .rewrite()
        .map(|rewrite| rewrite.selection().clone())
        .or_else(|| {
            meta.sentence_labels()
                .map(SentenceLabelSelection::from_labels)
        })
        .unwrap_or_default();
    let labels = meta.sentence_labels().map(|labels| {
        json!({
            "register": labels.register().token(),
            "type": labels.kind().token(),
            "level": labels.level().prompt_token(),
            "pinned": labels.pinned(),
            "approx": labels.approx(),
        })
    });
    let meta_json = serde_json::to_string_pretty(&json!({
        "pronunciation": meta.pronunciation(),
        "transcription": meta.transcription(),
        "meaning": meta.meaning(),
        "importance": meta.importance(),
        "source_sentence": meta.source_sentence(),
        "source_highlight": meta.source_highlight(),
        "source_hint": meta.source_hint(),
        "source_context": meta.source_context(),
        "target_sentence": meta.target_sentence(),
        "labels": labels,
    }))?;
    let correction = if comment.trim().is_empty() {
        String::from("rewrite only what the requested preset requires")
    } else {
        String::from(comment)
    };
    render(
        CARD_PROMPT,
        &[
            (
                "{learner_explanations}",
                learner_explanations(examples.writing()?)?,
            ),
            (
                "{target_language}",
                language_label(catalog, pair.learning())?,
            ),
            ("{source_language}", language_label(catalog, source.code)?),
            ("{hint_length}", String::from(examples.hint_length())),
            ("{hint_examples}", examples.hint()?),
            ("{context_examples}", examples.context()?),
            ("{term}", String::from(draft.term())),
            ("{understanding}", String::from(draft.understanding())),
            ("{reviewed_senses}", reviewed_senses(draft)?),
            ("{current_meta}", meta_json),
            ("{requested_labels}", requested_labels(&selection)),
            ("{user_correction}", correction),
        ],
    )
}

fn requested_labels(selection: &SentenceLabelSelection) -> String {
    let values = [
        SentenceAxis::Register,
        SentenceAxis::Type,
        SentenceAxis::Level,
    ]
    .into_iter()
    .filter_map(|axis| {
        let value = match axis {
            SentenceAxis::Level => selection.level().map(|level| level.prompt_token()),
            SentenceAxis::Register | SentenceAxis::Type => selection.token(axis),
        }?;
        let strength = if selection.pinned().contains(axis) {
            "changed"
        } else {
            "preserve"
        };
        Some(format!("{}=\"{value}\" ({strength})", axis.token()))
    })
    .collect::<Vec<_>>();
    if values.is_empty() {
        return String::from("none");
    }
    values.join(" · ")
}

fn empty_meta() -> CardMeta {
    CardMeta::new("", "", "", 5, "", "", "", "", "")
}

fn language_choices(catalog: &LanguageCatalog) -> Result<String> {
    let mut items = Vec::new();
    for code in catalog.codes() {
        items.push(language_label(catalog, code)?);
    }
    Ok(items.join(", "))
}

fn target_instruction(target: &LearningTarget, catalog: &LanguageCatalog) -> Result<String> {
    match target {
        LearningTarget::Detect => Ok(String::from(
            "Choose exactly one dominant target language for the whole batch. One non-trivial item is enough to fix the language; treat the whole batch as that language.",
        )),
        LearningTarget::Explicit(code) => {
            let profile = catalog.item(code.as_ref())?;
            Ok(format!(
                "The required target language is {code} ({}). Use exactly this target for the whole batch. Do not detect or choose another target language.",
                profile.prompt
            ))
        }
    }
}

fn language_label(catalog: &LanguageCatalog, code: &str) -> Result<String> {
    let item = catalog.item(code)?;
    Ok(format!("{} ({})", item.code, item.prompt))
}

fn learner_explanations(writing: String) -> Result<String> {
    render(LEARNER_EXPLANATIONS, &[("{learner_examples}", writing)])
}

fn render(template: &str, values: &[(&str, String)]) -> Result<String> {
    PromptTemplate::new(template).render(values)
}

#[cfg(test)]
mod tests {
    use super::{
        CARD_META_PROMPT, language_label, learner_explanations, render, render_bulk_prompt,
        render_card_meta_prompt, render_card_prompt, render_intake_prompt, render_phonetics_prompt,
        reviewed_senses,
    };
    use crate::application::LearningTarget;
    use crate::languages::catalog;
    use crate::session::{
        AxisSet, CardDraft, CardMeta, LanguagePair, Register, Sense, SentenceAxis, SentenceKind,
        SentenceLabelSelection, SentenceLabels, SentenceLevel, WordCandidate,
    };

    #[test]
    fn learner_prompts_cannot_omit_their_local_writing_examples() {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/prompt_examples.json"))
                .expect("prompt examples must decode");
        let catalog = catalog();
        let candidate = WordCandidate::new("term", "one precise sense", true);
        let complete = catalog.codes().into_iter().all(|known| {
            let Some(examples) = document[known.to_ascii_lowercase()]["card"]["writing"]
                .as_array()
                .filter(|examples| examples.len() == 2)
            else {
                return false;
            };
            let pair = LanguagePair::new("en", known);
            let draft = CardDraft::new("term", "one precise sense", pair.clone());
            [
                render_intake_prompt("term", known, &LearningTarget::Detect, &catalog),
                render_bulk_prompt(&candidate, "add one sense", &pair, &catalog),
                render_card_meta_prompt(&draft, None, &catalog),
                render_card_prompt(&draft, "", &pair, &catalog),
            ]
            .into_iter()
            .all(|prompt| {
                prompt.is_ok_and(|prompt| {
                    examples.iter().all(|example| {
                        ["bad", "good"].into_iter().all(|key| {
                            example[key]
                                .as_str()
                                .is_some_and(|text| prompt.matches(text).count() == 1)
                        })
                    }) && !prompt.contains("{learner_examples}")
                })
            })
        });
        assert!(
            complete,
            "a learner prompt omitted, duplicated or failed to interpolate localized writing examples"
        );
    }

    #[test]
    fn local_writing_examples_cannot_enter_the_phonetics_review() {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/prompt_examples.json"))
                .expect("prompt examples must decode");
        let isolated = catalog().codes().into_iter().all(|known| {
            let Some(examples) = document[known.to_ascii_lowercase()]["card"]["writing"]
                .as_array()
                .filter(|examples| examples.len() == 2)
            else {
                return false;
            };
            let pair = LanguagePair::new("en", known);
            let draft = CardDraft::new("term", "one precise sense", pair.clone());
            let meta = CardMeta::new("tɜːm", "tɜːm", "", 5, "", "", "", "", "term");
            render_phonetics_prompt(&draft, &meta, &pair).is_ok_and(|prompt| {
                examples.iter().all(|example| {
                    ["bad", "good"].into_iter().all(|key| {
                        example[key]
                            .as_str()
                            .is_some_and(|text| !prompt.contains(text))
                    })
                })
            })
        });
        assert!(
            isolated,
            "localized writing examples leaked into the IPA-only review"
        );
    }

    #[test]
    fn nested_style_rendering_cannot_rescan_user_slots() {
        let literal = "{learner_examples}";
        let pair = LanguagePair::new("en", "ru");
        let catalog = catalog();
        let candidate = WordCandidate::new("term", literal, true);
        let draft = CardDraft::new("term", literal, pair.clone());
        let preserved = [
            render_intake_prompt(literal, "ru", &LearningTarget::Detect, &catalog),
            render_bulk_prompt(&candidate, literal, &pair, &catalog),
            render_card_meta_prompt(&draft, None, &catalog),
            render_card_prompt(&draft, literal, &pair, &catalog),
        ]
        .into_iter()
        .all(|prompt| prompt.is_ok_and(|prompt| prompt.contains(literal)));
        assert!(
            preserved,
            "nested style rendering interpreted literal user data as another template slot"
        );
    }

    #[test]
    fn learner_explanations_cannot_skip_plain_language_in_any_supported_language() {
        let catalog = catalog();
        let candidate = WordCandidate::new("term", "one precise sense", true);
        let complete = catalog.codes().into_iter().all(|known| {
            let pair = LanguagePair::new("en", known);
            let draft = CardDraft::new("term", "one precise sense", pair.clone());
            [
                render_intake_prompt("term", known, &LearningTarget::Detect, &catalog),
                render_bulk_prompt(&candidate, "add one sense", &pair, &catalog),
                render_card_meta_prompt(&draft, None, &catalog),
                render_card_prompt(&draft, "", &pair, &catalog),
            ]
            .into_iter()
            .all(|prompt| {
                prompt.is_ok_and(|prompt| {
                    prompt
                        .matches("Assume no knowledge of grammar terminology")
                        .count()
                        == 1
                        && !prompt.contains("State the exact grammatical subclass")
                        && !prompt.contains("Always abbreviate part-of-speech labels")
                })
            })
        });
        assert!(
            complete,
            "a learner-facing prompt omitted plain language or still demanded grammar terminology"
        );
    }

    #[test]
    fn learner_prose_rules_cannot_change_the_phonetics_review() {
        let pair = LanguagePair::new("en", "ru");
        let draft = CardDraft::new("term", "one precise sense", pair.clone());
        let meta = CardMeta::new("tɜːm", "tɜːm", "слово", 5, "", "", "", "", "term");
        let prompt =
            render_phonetics_prompt(&draft, &meta, &pair).expect("phonetics review must render");
        assert!(
            !prompt.contains("Assume no knowledge of grammar terminology"),
            "learner prose instructions leaked into the IPA-only review"
        );
    }

    #[test]
    fn english_intake_cannot_embed_russian_support_examples() {
        let prompt = render_intake_prompt("râler", "en", &LearningTarget::Detect, &catalog())
            .expect("english intake prompt must render");
        assert!(
            prompt.contains("\"fin.\"")
                && !prompt.contains("фин.")
                && !prompt.contains("Сущ.")
                && !prompt.contains("Учим"),
            "english intake retained examples from another support language"
        );
    }

    #[test]
    fn english_sense_correction_cannot_embed_foreign_support_examples() {
        let prompt = render_bulk_prompt(
            &WordCandidate::new("râler", "V. to grumble about everything.", true),
            "add the medical sense",
            &LanguagePair::new("fr", "en"),
            &catalog(),
        )
        .expect("english sense prompt must render");
        assert!(
            prompt.contains("\"fin.\"")
                && !prompt.contains("фин.")
                && !prompt.contains("Сущ.")
                && !prompt.contains("sust."),
            "english sense correction retained examples from another support language"
        );
    }

    #[test]
    fn german_card_meta_uses_german_support_examples() {
        let draft = CardDraft::new("canard", "eine Zeitungsente", LanguagePair::new("fr", "de"));
        let prompt = render_card_meta_prompt(&draft, None, &catalog())
            .expect("german card meta prompt must render");
        assert!(
            prompt.contains("**Bedeutung.**")
                && prompt.contains("selten")
                && !prompt.contains("Не шумный"),
            "german card meta retained examples from another support language"
        );
    }

    #[test]
    fn chosen_reviewed_tag_binds_generation_in_both_card_prompts() {
        let candidate = WordCandidate::with_senses(
            "crever",
            vec![
                Sense::plain("to puncture"),
                Sense::tagged("to die", "slang"),
                Sense::plain("to burst"),
            ],
            1,
            true,
        );
        let pair = LanguagePair::new("fr", "en");
        let draft = CardDraft::from_candidate(&candidate, 1, pair.clone());
        let senses = reviewed_senses(&draft).expect("reviewed senses must render");
        let decoded = serde_json::from_str::<serde_json::Value>(senses.as_str())
            .expect("reviewed senses must stay valid JSON");
        let generated = render_card_meta_prompt(&draft, None, &catalog())
            .expect("multi-sense card meta prompt must render");
        let corrected = render_card_prompt(&draft, "make it shorter", &pair, &catalog())
            .expect("multi-sense correction prompt must render");
        assert_eq!(
            (
                decoded,
                generated.matches(senses.as_str()).count(),
                corrected.matches(senses.as_str()).count(),
                generated.contains(
                    "Its `understanding` and any non-null `tag` are one binding constraint, not display metadata"
                ),
                generated.contains(
                    "The chosen tag must govern the translation, target sentence, situation, and descriptive labels"
                ),
                generated.contains("understanding and the chosen reviewed tag"),
                generated.contains("under the chosen understanding and tag"),
                corrected.contains(
                    "Unless the correction explicitly moves the card to another sense, the chosen tag must continue to govern"
                ),
                corrected.contains(
                    "preserve the chosen tag's register, domain, region, and usage constraints throughout the card"
                ),
                corrected.contains("chosen sense, form, and tagged usage"),
                generated.contains("Do not add historical derivation or relatedness claims"),
                corrected.contains("Do not add historical derivation or relatedness claims"),
            ),
            (
                serde_json::json!([
                    {"chosen": true, "priority": 1, "understanding": "to die", "tag": "slang"},
                    {"chosen": false, "priority": 0, "understanding": "to puncture", "tag": null},
                    {"chosen": false, "priority": 2, "understanding": "to burst", "tag": null},
                ]),
                1,
                1,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
                true,
            ),
            "a reviewed tag stopped constraining one of the card-generation prompts"
        );
    }

    #[test]
    fn card_prompts_omit_unprovided_etymology_from_authored_explanations() {
        let candidate = WordCandidate::with_senses(
            "bank",
            vec![
                Sense::tagged("a financial institution", "finance"),
                Sense::tagged("land beside a river", "landform"),
            ],
            0,
            true,
        );
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::from_candidate(&candidate, 0, pair.clone());
        let generated = render_card_meta_prompt(&draft, None, &catalog())
            .expect("multi-sense card meta prompt must render");
        let corrected = render_card_prompt(&draft, "make it shorter", &pair, &catalog())
            .expect("multi-sense correction prompt must render");
        let safeguards = [
            "No trusted etymological source is supplied",
            "Do not add historical derivation or relatedness claims",
            "Do not infer unrelated origins from uncertainty",
        ];
        assert!(
            safeguards
                .iter()
                .all(|rule| generated.contains(rule) && corrected.contains(rule))
                && ![generated, corrected].iter().any(|prompt| {
                    prompt.contains("Give a brief historical")
                        || prompt.contains("reliably known to be unrelated or homonymous")
                }),
            "a card prompt still authorized unsupported etymology from model confidence"
        );
    }

    #[test]
    fn a_changed_term_expires_the_old_reviewed_inventory_in_the_correction_prompt() {
        let candidate = WordCandidate::with_senses(
            "bound",
            vec![Sense::plain("a limit"), Sense::plain("a jump")],
            0,
            true,
        );
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::from_candidate(&candidate, 0, pair.clone());
        let prompt = render_card_prompt(
            &draft,
            "Change the term to limit for a boundary",
            &pair,
            &catalog(),
        )
        .expect("term-changing correction prompt must render");
        assert!(
            prompt.contains("If returned `term` differs from current `term`")
                && prompt.contains("the old `Reviewed meanings` are obsolete")
                && prompt.contains("one bold bullet containing the returned `understanding` only")
                && prompt.contains("Do not invent or transfer alternatives for the new term")
                && !prompt.contains("In the first section copy all `Reviewed meanings`"),
            "a term-changing correction still instructed the author to carry old sibling meanings"
        );
    }

    #[test]
    fn card_prompts_keep_the_glossary_list_only_and_each_usage_section_compact() {
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let prompts = [
            render_card_meta_prompt(&draft, None, &catalog()).expect("metadata prompt must render"),
            render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
                .expect("correction prompt must render"),
        ];
        assert!(
            prompts.iter().all(|prompt| {
                prompt.contains("Do not add a relationship or differences paragraph")
                    && prompt
                        .contains("one useful point each, using one or two short, direct sentences")
                    && prompt.contains("roughly one short line of prose")
                    && prompt.contains("Do not repeat the meaning list in prose")
                    && !prompt.contains("Ground the relationship")
                    && !prompt.contains("Keep every bullet and the relationship sentence")
            }),
            "a card prompt still demanded redundant glossary prose or open-ended usage paragraphs"
        );
    }

    #[test]
    fn card_descriptions_cannot_invent_rules_from_one_working_example() {
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let prompts = [
            render_card_meta_prompt(&draft, None, &catalog()).expect("metadata prompt must render"),
            render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
                .expect("correction prompt must render"),
        ];
        assert!(
            prompts.iter().all(|prompt| {
                prompt.contains("Do not infer an absolute rule from one example")
                    && prompt.contains("Explain one useful usage nuance")
                    && prompt.contains("A translated example alone is not a nuance")
                    && prompt.contains("include one short target-language example in quotation marks")
                    && prompt.contains("The example must show the exact combination being explained")
                    && prompt.contains("Do not rescue a false claim merely by adding usually or often")
                    && prompt.contains("Determine the exact scope silently; explain it with actual words and situations")
                    && prompt.contains("If there is no special usage restriction")
                    && prompt.contains("If there is no useful additional peculiarity")
                    && prompt.contains("Tone, facial expression, and word order are cues")
                    && prompt.contains("not necessary or sufficient conditions for a speaker's intent")
                    && prompt.contains("no additional special point needs noting beyond the guidance above")
                    && prompt.contains("Do not turn a more usual alternative into a ban on the chosen word")
                    && prompt.contains("Keep lexical meaning separate from real-world consequences")
                    && prompt.contains("does not by itself guarantee an outcome or impose an obligation")
                    && prompt.contains("Examples mentioned in a definition are not exhaustive lists")
                    && prompt.contains("do not intensify them with only, always")
                    && prompt.contains("an attempt, plan, or purpose is not a completed result")
                    && prompt.contains("against every written word of target_sentence")
                    && prompt.contains("Ordinary source-language tokens already visible in source_sentence may recur naturally")
                    && prompt.contains("do not name them as the answer")
            }),
            "a card prompt dropped useful guidance or still demanded unsupported restrictions"
        );
    }

    #[test]
    fn a_narrow_correction_cannot_preserve_a_broken_description_contract() {
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let prompt = render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
            .expect("correction prompt must render");
        assert!(
            prompt.contains("Always check the existing source_context against the four-section contract")
                && prompt.contains("repair only the affected description sections")
                && prompt.contains("wording requires grammar knowledge or is unnecessarily difficult")
                && prompt.contains("This does not authorize unrelated changes to the term, sense, example situation, or preserved labels"),
            "a narrow correction could retain obsolete headers or example-only teaching sections"
        );
    }

    #[test]
    fn card_usage_guidance_cannot_turn_shared_domains_into_exclusive_meanings() {
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let prompts = [
            render_card_meta_prompt(&draft, None, &catalog()).expect("metadata prompt must render"),
            render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
                .expect("correction prompt must render"),
        ];
        assert!(
            prompts.iter().all(|prompt| {
                prompt.contains("Scope usage observations to this chosen use or this example")
                    && prompt.contains("check whether each other reviewed meaning also occurs there")
                    && prompt.contains("do not manufacture separate domains")
                    && !prompt.contains(
                        "name a part of speech, construction, register, domain, or concrete situation that separates the reviewed senses",
                    )
            }),
            "a card prompt still demanded an exclusive domain boundary for meanings that can overlap"
        );
    }

    #[test]
    fn card_prompts_do_not_force_a_false_prohibition_or_near_word_distinction() {
        let pair = LanguagePair::new("en", "fr");
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let prompts = [
            render_card_meta_prompt(&draft, None, &catalog()).expect("metadata prompt must render"),
            render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
                .expect("correction prompt must render"),
        ];
        assert!(
            prompts.iter().all(|prompt| {
                prompt.contains("instead of inventing a forbidden setting or a substitute")
                    && prompt.contains("An ordinary valid counterexample defeats an absolute rule")
                    && prompt.contains("Scope the advice to that situation")
                    && prompt.contains(
                        "Default to a concrete scene cue without naming another target-language word",
                    )
                    && prompt.contains("Optional contrasts are not a required field or format")
                    && prompt.contains("If the other word is also a valid answer in this context")
                    && prompt.contains("Check every part of the hint")
                    && prompt.contains(
                        "Ordinary source-language tokens already visible in source_sentence may recur naturally",
                    )
            }),
            "a card prompt still forced a broad rule, a near-word contrast, or translation avoidance"
        );
    }

    #[test]
    fn authored_explanations_cannot_add_qualifiers_beyond_their_evidence() {
        let pair = LanguagePair::new("en", "fr");
        let candidate = WordCandidate::new("anchor", "a heavy mooring device", true);
        let draft = CardDraft::new("anchor", "a heavy mooring device", pair.clone());
        let cards = [
            render_card_meta_prompt(&draft, None, &catalog()).expect("metadata prompt must render"),
            render_card_prompt(&draft, "use a shorter example", &pair, &catalog())
                .expect("correction prompt must render"),
        ];
        let senses = [
            render_intake_prompt("anchor", "fr", &LearningTarget::Detect, &catalog())
                .expect("intake prompt must render"),
            render_bulk_prompt(&candidate, "another common use", &pair, &catalog())
                .expect("sense prompt must render"),
        ];
        assert!(
            cards.iter().all(|prompt| {
                prompt.contains("Do not add properties or causes absent from the definitions")
                    && prompt.contains(
                        "each example translation preserves exactly what its example says",
                    )
                    && prompt.contains("cannot replace the explanation")
            }) && senses.iter().all(|prompt| {
                prompt.contains("Use the shortest distinguishing definition")
                    && prompt.contains(
                        "Check each qualifier against ordinary valid examples of the same use",
                    )
                    && prompt.contains("Remove imagined details about how the action is performed")
            }),
            "an authoring prompt still turned an imagined example or collocation into an unsupported definition"
        );
    }

    #[test]
    fn sense_prompts_do_not_turn_typical_examples_into_lexical_restrictions() {
        let pair = LanguagePair::new("en", "fr");
        let candidate = WordCandidate::new("anchor", "a heavy mooring device", true);
        let prompts = [
            render_intake_prompt("anchor", "fr", &LearningTarget::Detect, &catalog())
                .expect("intake prompt must render"),
            render_bulk_prompt(&candidate, "another common use", &pair, &catalog())
                .expect("sense prompt must render"),
        ];
        assert!(
            prompts.iter().all(|prompt| {
                prompt.contains("A typical example does not define a restriction")
                    && prompt
                        .contains("Prefer a broader accurate gloss to a narrower unsupported one")
                    && prompt.contains("Do not add a tag merely to fill a category")
            }),
            "a sense prompt still encouraged incidental restrictions or invented usage labels"
        );
    }

    #[test]
    fn card_meta_prompt_attributes_cefr_only_after_writing_a_natural_sentence() {
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        let prompt = render_card_meta_prompt(&draft, None, &catalog())
            .expect("three-axis card meta prompt must render");
        assert!(
            prompt.contains(
                "\"labels\":{\"register\":\"<neutral|casual|formal|literary|archaic>\",\"type\":\"<statement|question|request|exclamation|dialogue>\",\"level\":\"<a1|a2|b1|b2|c1|c2>\",\"approx\":[]}"
            ) && prompt.contains("without aiming for any register, type, or level")
                && prompt.contains("Only then attribute the finished sentence")
                && prompt.contains("descriptive and must never trigger a rewrite")
                && prompt.contains("Do not simplify, modernize, or neutralize")
                && prompt.contains("classify only the already-final sentence")
                && prompt.contains("There is no default initial level")
                && prompt.contains("Sentence length alone is not evidence of level")
                && prompt.contains("Naturally required collocations")
                && prompt.contains("surrounding language")
                && prompt.contains("target term is exempt")
                && prompt.contains("rare target term inside basic surrounding language")
                && prompt.contains("within the target language itself")
                && prompt.contains("not an official proficiency assessment")
                && prompt.contains("English-specific word counts")
                && prompt.contains("- a1:")
                && prompt.contains("- a2:")
                && prompt.contains("- b1:")
                && prompt.contains("- b2:")
                && prompt.contains("- c1:")
                && prompt.contains("- c2:")
                && !prompt.contains("Start at b1")
                && !prompt.contains("aim at `b1`")
                && !prompt.contains("return `\"level\":\"b1\"`")
                && !prompt.contains("initial default")
                && !prompt.contains("only deliberate vocabulary target")
                && !prompt.contains("takes practice")
                && !prompt.contains("challenging")
                && !prompt.contains("simplified")
                && !prompt.contains("default profile")
                && !prompt.contains("extended")
                && !prompt.contains("20 natural words")
                && !prompt.contains("\"grammar\":"),
            "card meta prompt turned post-hoc CEFR attribution into a generation target"
        );
    }

    #[test]
    fn empty_initial_preferences_leave_the_legacy_prompt_byte_for_byte() {
        let pair = LanguagePair::new("fr", "en");
        let catalog = catalog();
        let source = pair
            .known_profile(&catalog)
            .expect("source language must resolve");
        let examples = catalog
            .prompts(source.code)
            .expect("prompt examples must resolve");
        let legacy_template = CARD_META_PROMPT
            .replace("{initial_approx_schema}", "[]")
            .replace(
                "{initial_approx_rule}",
                "`approx` is always an empty array.",
            )
            .replace("{initial_sentence_preferences}", "");
        let legacy = render(
            legacy_template.as_str(),
            &[
                (
                    "{learner_explanations}",
                    learner_explanations(examples.writing().expect("writing examples must render"))
                        .expect("learner style must render"),
                ),
                (
                    "{target_language}",
                    language_label(&catalog, pair.learning())
                        .expect("target language label must render"),
                ),
                (
                    "{source_language}",
                    language_label(&catalog, source.code)
                        .expect("source language label must render"),
                ),
                ("{hint_length}", String::from(examples.hint_length())),
                (
                    "{hint_examples}",
                    examples.hint().expect("hint examples must render"),
                ),
                (
                    "{context_examples}",
                    examples.context().expect("context examples must render"),
                ),
                ("{term}", String::from("canard")),
                ("{understanding}", String::from("a duck")),
                (
                    "{reviewed_senses}",
                    reviewed_senses(&CardDraft::new("canard", "a duck", pair.clone()))
                        .expect("reviewed senses must render"),
                ),
            ],
        )
        .expect("legacy card meta prompt must render");
        let draft = CardDraft::new("canard", "a duck", pair);
        assert_eq!(
            render_card_meta_prompt(&draft, None, &catalog)
                .expect("default card meta prompt must render"),
            legacy,
            "empty initial preferences changed the legacy card meta prompt bytes"
        );
    }

    #[test]
    fn initial_preferences_render_only_the_pinned_level_and_type() {
        let request = SentenceLabelSelection::empty()
            .choosing(SentenceAxis::Level, 2)
            .choosing(SentenceAxis::Type, 1);
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        let prompt = render_card_meta_prompt(&draft, Some(&request), &catalog())
            .expect("requested card meta prompt must render");
        assert!(
            prompt.contains(
                "Initial sentence preset: type=\"question\" (changed) · level=\"b1\" (changed)"
            ) && prompt.contains("otherwise return the closest natural sentence")
                && prompt.contains(
                    "\"approx\":[\"<requested axis that could not be fulfilled exactly; omit fulfilled and unrequested axes>\"]"
                )
                && prompt.contains(
                    "`approx` is empty when every requested axis is fulfilled exactly"
                )
                && !prompt.contains("\"approx\":[]")
                && !prompt.contains("`approx` is always an empty array")
                && !prompt.contains("register=\""),
            "initial card meta prompt kept a contradictory approximation rule or invented a register preference"
        );
    }

    #[test]
    fn japanese_card_correction_uses_japanese_support_headers() {
        let prompt = render_card_prompt(
            &CardDraft::new("canard", "新聞の虚報", LanguagePair::new("fr", "ja")),
            "例文を短くする",
            &LanguagePair::new("fr", "ja"),
            &catalog(),
        )
        .expect("japanese card correction prompt must render");
        assert!(
            prompt.contains("**意味**")
                && prompt.contains("**どこで耳にするか**")
                && !prompt.contains("**Where you'll hear it.**"),
            "japanese card correction retained english support headers"
        );
    }

    #[test]
    fn blank_card_correction_renders_three_axis_strengths_with_the_exact_fallback() {
        let labels = SentenceLabels::new(
            Register::Casual,
            SentenceLevel::B1,
            SentenceKind::Statement,
            AxisSet::default(),
            AxisSet::default(),
        );
        let selection =
            SentenceLabelSelection::from_labels(&labels).choosing(SentenceAxis::Register, 2);
        let meta = CardMeta::new(
            "ka.naʁ",
            "lə ka.naʁ naʒ",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A concrete noun",
            "Le canard nage",
        )
        .with_sentence_labels(labels);
        let pair = LanguagePair::new("fr", "en");
        let draft = CardDraft::new("canard", "a duck", pair.clone())
            .with_meta(meta, None)
            .rewriting(selection, " \n ");
        let prompt = render_card_prompt(&draft, " \n ", &pair, &catalog())
            .expect("blank correction prompt must render");
        assert!(
            prompt.contains(
                "register=\"formal\" (changed) · type=\"statement\" (preserve) · level=\"b1\" (preserve)"
            ) && prompt.contains("preserve must remain exact even when the correction conflicts")
                && prompt.contains("rewrite only what the requested preset requires")
                && prompt.contains("rescale only the surrounding language")
                && prompt.contains("the same situation, actors, action, tense, register, and type")
                && prompt.contains("adjacent CEFR level")
                && prompt.contains("target term is exempt")
                && !prompt.contains("takes practice")
                && !prompt.contains("challenging")
                && !prompt.contains("simplified")
                && !prompt.contains("default profile")
                && !prompt.contains("extended")
                && !prompt.contains("20 natural words")
                && !prompt.contains("\"grammar\":")
                && !prompt.contains("grammar="),
            "blank card correction lost minimal profile movement, three-axis strengths, or exact fallback"
        );
    }

    #[test]
    fn level_only_correction_moves_from_b1_to_b2_without_changing_other_axes() {
        let labels = SentenceLabels::new(
            Register::Casual,
            SentenceLevel::B1,
            SentenceKind::Statement,
            AxisSet::default(),
            AxisSet::default(),
        );
        let selection =
            SentenceLabelSelection::from_labels(&labels).choosing(SentenceAxis::Level, 3);
        let meta = CardMeta::new(
            "ka.naʁ",
            "lə ka.naʁ naʒ",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A concrete noun",
            "Le canard nage",
        )
        .with_sentence_labels(labels);
        let pair = LanguagePair::new("fr", "en");
        let draft = CardDraft::new("canard", "a duck", pair.clone())
            .with_meta(meta, None)
            .rewriting(selection, "");
        let prompt = render_card_prompt(&draft, "", &pair, &catalog())
            .expect("level-only correction prompt must render");
        assert!(
            prompt.contains(
                "register=\"casual\" (preserve) · type=\"statement\" (preserve) · level=\"b2\" (changed)"
            ) && prompt.contains("\"level\": \"b1\"")
                && prompt.contains("Keep `term` and its form and sense")
                && prompt.contains("the same situation, actors, action, tense, register, and type")
                && prompt.contains("smallest edit")
                && prompt.contains("adjacent CEFR level")
                && prompt.contains("Never replace the target term")
                && prompt.contains("Do not change sentence length merely to signal a level")
                && prompt.contains("Naturalness is never negotiable")
                && prompt.contains("closest natural rewrite")
                && prompt.contains("put `level` in `approx`")
                && prompt.contains("Never put a preserved axis in `approx`")
                && !prompt.contains("level=\"balanced\"")
                && !prompt.contains("takes practice")
                && !prompt.contains("challenging")
                && !prompt.contains("simplified")
                && !prompt.contains("default profile")
                && !prompt.contains("extended")
                && !prompt.contains("20 natural words")
                && !prompt.contains("grammar="),
            "level-only correction leaked retired labels or lost minimal CEFR movement"
        );
    }

    #[test]
    fn user_values_cannot_trigger_a_second_template_interpolation() {
        let draft = CardDraft::new(
            "{understanding}",
            "chosen sense",
            LanguagePair::new("fr", "en"),
        );
        let prompt = render_card_meta_prompt(&draft, None, &catalog())
            .expect("card meta prompt with placeholder-shaped input must render");
        assert_eq!(
            prompt.matches("{understanding}").count(),
            1,
            "user input was reinterpreted as a template placeholder"
        );
    }

    #[test]
    fn cjk_prompts_use_lexical_length_rules_without_artificial_spaces() {
        let chinese = render_intake_prompt("批准", "zh", &LearningTarget::Detect, &catalog())
            .expect("chinese intake prompt must render");
        let draft = CardDraft::new("承認", "同意して認めること", LanguagePair::new("en", "ja"));
        let japanese = render_card_meta_prompt(&draft, None, &catalog())
            .expect("japanese card prompt must render");
        let english = render_intake_prompt("râler", "en", &LearningTarget::Detect, &catalog())
            .expect("english intake prompt must render");
        assert!(
            chinese.contains("without artificial spaces")
                && japanese.contains("comparable brevity")
                && english.contains("space-delimited words"),
            "language-local prompts retained one universal word-count rule"
        );
    }

    #[test]
    fn every_supported_language_pair_renders_its_typed_examples() {
        let catalog = catalog();
        let candidate = WordCandidate::new("term", "one precise sense", true);
        let complete = catalog.codes().into_iter().all(|known| {
            render_intake_prompt("term", known, &LearningTarget::Detect, &catalog).is_ok()
                && catalog.codes().into_iter().all(|learning| {
                    let pair = LanguagePair::new(learning, known);
                    let draft = CardDraft::new("term", "one precise sense", pair.clone());
                    render_bulk_prompt(&candidate, "add one sense", &pair, &catalog).is_ok()
                        && render_card_meta_prompt(&draft, None, &catalog).is_ok()
                        && render_card_prompt(&draft, "make it shorter", &pair, &catalog).is_ok()
                })
        });
        assert!(
            complete,
            "a supported language pair cannot render all typed prompt examples"
        );
    }
}
