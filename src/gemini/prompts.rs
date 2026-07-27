use anyhow::Result;
use serde_json::json;

use crate::languages::LanguageCatalog;
use crate::prompt::PromptTemplate;
use crate::session::{CardDraft, CardMeta, LanguagePair, WordCandidate};

const INTAKE_PROMPT: &str = include_str!("../../assets/gemini_intake_prompt.txt");
const SENSE_PROMPT: &str = include_str!("../../assets/gemini_sense_prompt.txt");
const CARD_META_PROMPT: &str = include_str!("../../assets/gemini_card_meta_prompt.txt");
const CARD_PROMPT: &str = include_str!("../../assets/gemini_card_prompt.txt");

/// Render the human-in-the-loop intake prompt.
pub(super) fn render_intake_prompt(
    raw: &str,
    my: &str,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let support = catalog.item(my)?;
    let examples = catalog.prompts(support.code)?;
    render(
        INTAKE_PROMPT,
        &[
            ("{supported_languages}", language_choices(catalog)?),
            ("{support_language}", language_label(catalog, support.code)?),
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

/// Render the per-card refinement prompt fired from `Change this card`.
pub(super) fn render_card_prompt(
    draft: &CardDraft,
    comment: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let source = pair.known_profile(catalog)?;
    let examples = catalog.prompts(source.code)?;
    let meta = draft.meta().cloned().unwrap_or_else(empty_meta);
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
    }))?;
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
            ("{user_correction}", String::from(comment)),
        ],
    )
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
    use crate::languages::catalog;
    use crate::session::{CardDraft, LanguagePair, WordCandidate};

    #[test]
    fn english_intake_cannot_embed_russian_support_examples() {
        let prompt = render_intake_prompt("râler", "en", &catalog())
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
        let chinese = render_intake_prompt("批准", "zh", &catalog())
            .expect("chinese intake prompt must render");
        let japanese = render_card_meta_prompt(
            "承認",
            "同意して認めること",
            &LanguagePair::new("en", "ja"),
            &catalog(),
        )
        .expect("japanese card prompt must render");
        let english = render_intake_prompt("râler", "en", &catalog())
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
            render_intake_prompt("term", known, &catalog).is_ok()
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
