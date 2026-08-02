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

/// Render the card-meta generation prompt.
pub(super) fn render_card_meta_prompt(
    term: &str,
    understanding: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let source = pair.known_profile(catalog)?;
    let examples = catalog.prompts(source.code)?;
    render(
        CARD_META_PROMPT,
        &[
            (
                "{target_language}",
                language_label(catalog, pair.learning())?,
            ),
            ("{source_language}", language_label(catalog, source.code)?),
            ("{hint_length}", String::from(examples.hint_length())),
            ("{hint_examples}", examples.hint()?),
            ("{context_examples}", examples.context()?),
            ("{term}", String::from(term)),
            ("{understanding}", String::from(understanding)),
        ],
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
                "{target_language}",
                language_label(catalog, pair.learning())?,
            ),
            ("{source_language}", language_label(catalog, source.code)?),
            ("{hint_length}", String::from(examples.hint_length())),
            ("{hint_examples}", examples.hint()?),
            ("{context_examples}", examples.context()?),
            ("{term}", String::from(draft.term())),
            ("{understanding}", String::from(draft.understanding())),
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

fn render(template: &str, values: &[(&str, String)]) -> Result<String> {
    PromptTemplate::new(template).render(values)
}

#[cfg(test)]
mod tests {
    use super::{
        render_bulk_prompt, render_card_meta_prompt, render_card_prompt, render_intake_prompt,
    };
    use crate::application::LearningTarget;
    use crate::languages::catalog;
    use crate::session::{
        AxisSet, CardDraft, CardMeta, LanguagePair, Register, SentenceAxis, SentenceKind,
        SentenceLabelSelection, SentenceLabels, SentenceLevel, WordCandidate,
    };

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
        let prompt = render_card_meta_prompt(
            "canard",
            "eine Zeitungsente",
            &LanguagePair::new("fr", "de"),
            &catalog(),
        )
        .expect("german card meta prompt must render");
        assert!(
            prompt.contains("**Bedeutung.**")
                && prompt.contains("selten")
                && !prompt.contains("Не шумный"),
            "german card meta retained examples from another support language"
        );
    }

    #[test]
    fn card_meta_prompt_attributes_cefr_only_after_writing_a_natural_sentence() {
        let prompt = render_card_meta_prompt(
            "canard",
            "a duck",
            &LanguagePair::new("fr", "en"),
            &catalog(),
        )
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
        let prompt = render_card_meta_prompt(
            "{understanding}",
            "chosen sense",
            &LanguagePair::new("fr", "en"),
            &catalog(),
        )
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
        let japanese = render_card_meta_prompt(
            "承認",
            "同意して認めること",
            &LanguagePair::new("en", "ja"),
            &catalog(),
        )
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
                        && render_card_meta_prompt("term", "one precise sense", &pair, &catalog)
                            .is_ok()
                        && render_card_prompt(&draft, "make it shorter", &pair, &catalog).is_ok()
                })
        });
        assert!(
            complete,
            "a supported language pair cannot render all typed prompt examples"
        );
    }
}
