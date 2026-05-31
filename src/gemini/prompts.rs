use anyhow::Result;
use serde_json::json;

use crate::languages::LanguageCatalog;
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
    render(
        INTAKE_PROMPT,
        &[
            ("{supported_languages}", language_choices(catalog)?),
            ("{support_language}", language_label(catalog, my)?),
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
    let senses = candidate
        .senses()
        .iter()
        .map(|sense| json!({"understanding": sense.understanding(), "tag": sense.tag()}))
        .collect::<Vec<_>>();
    render(
        SENSE_PROMPT,
        &[
            ("{target_language}", language_label(catalog, pair.target())?),
            (
                "{support_language}",
                language_label(catalog, pair.support())?,
            ),
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
    render(
        CARD_META_PROMPT,
        &[
            ("{target_language}", language_label(catalog, pair.target())?),
            (
                "{source_language}",
                language_label(catalog, pair.support())?,
            ),
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
            ("{target_language}", language_label(catalog, pair.target())?),
            (
                "{source_language}",
                language_label(catalog, pair.support())?,
            ),
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
    let mut output = String::from(template.trim());
    for (placeholder, value) in values {
        output = output.replace(placeholder, value);
    }
    Ok(output)
}
