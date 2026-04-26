use anyhow::Result;

use crate::languages::LanguageCatalog;
use crate::session::{CardDraft, LanguagePair, MetaTone, WordCandidate};

const INTAKE_PROMPT: &str = include_str!("../../assets/gemini_intake_prompt.txt");
const BULK_PROMPT: &str = include_str!("../../assets/gemini_bulk_prompt.txt");
const CARD_PROMPT: &str = include_str!("../../assets/gemini_card_prompt.txt");

/// Render the first-pass vocabulary review prompt.
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

/// Render the bulk vocabulary refinement prompt.
pub(super) fn render_bulk_prompt(
    candidates: &[WordCandidate],
    comment: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    let rows = candidates
        .iter()
        .map(|candidate| {
            let meta = candidate
                .meta()
                .segments()
                .iter()
                .map(|segment| {
                    serde_json::json!({
                        "text": segment.text(),
                        "tone": meta_tone(segment.tone()),
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "term": candidate.term(),
                "kind": candidate.kind().label(),
                "preview": candidate.preview(),
                "note": candidate.note(),
                "meta": meta,
                "include": candidate.included(),
            })
        })
        .collect::<Vec<_>>();
    render(
        BULK_PROMPT,
        &[
            ("{target_language}", language_label(catalog, pair.target())?),
            (
                "{support_language}",
                language_label(catalog, pair.support())?,
            ),
            ("{current_rows}", serde_json::to_string_pretty(&rows)?),
            ("{user_correction}", String::from(comment)),
        ],
    )
}

/// Render the per-card refinement prompt.
pub(super) fn render_card_prompt(
    draft: &CardDraft,
    comment: &str,
    pair: &LanguagePair,
    catalog: &LanguageCatalog,
) -> Result<String> {
    render(
        CARD_PROMPT,
        &[
            ("{target_language}", language_label(catalog, pair.target())?),
            (
                "{support_language}",
                language_label(catalog, pair.support())?,
            ),
            ("{term}", String::from(draft.term())),
            ("{front}", String::from(draft.payload().front())),
            ("{back}", String::from(draft.payload().back())),
            ("{hint}", String::from(draft.payload().hint())),
            ("{highlight}", String::from(draft.payload().highlight())),
            ("{user_correction}", String::from(comment)),
        ],
    )
}

fn meta_tone(tone: MetaTone) -> &'static str {
    match tone {
        MetaTone::Dim => "dim",
        MetaTone::Bright => "bright",
    }
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
