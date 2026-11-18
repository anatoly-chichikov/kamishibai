//! Accounted explanation requests after metadata and phonetics have settled.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use super::super::protocol::{GenerationConfig, Request, ThinkingLevel};
use super::{
    GeminiClient, GeneratedResponse, Transport, diagnosis, prioritized_senses, response_text,
};
use crate::explanation::{
    Card, Input, Language, Operation, Parts, Record, Sense, Task, Writer, compose, judge, write,
};
use crate::models::{GenerationStage, ModelOutput};
use crate::session::{CardDraft, CardMeta, CostRecord};

struct ExplanationWriter<'a, T: Transport, F> {
    client: &'a GeminiClient<T>,
    observe: &'a mut F,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Translations {
    source_sentence: String,
    source_highlight: String,
    examples: [String; 3],
}

impl<T: Transport, F: FnMut(CostRecord) -> Result<()>> Writer for ExplanationWriter<'_, T, F> {
    fn ask(&mut self, prompt: String) -> Result<String> {
        self.client.explanation_text(prompt, self.observe)
    }
}

impl<T: Transport> GeminiClient<T> {
    /// Compose the explanation and independently translate reviewed multiword expressions on creation.
    pub(super) fn explanation_observed<F: FnMut(CostRecord) -> Result<()>>(
        &self,
        draft: &CardDraft,
        meta: CardMeta,
        operation: Operation,
        request: &str,
        observe: &mut F,
    ) -> Result<CardMeta> {
        let language = Language::from_codes(draft.pair().known(), draft.pair().learning())?;
        let senses = prioritized_senses(draft)
            .iter()
            .enumerate()
            .map(|(index, sense)| {
                Sense::new(
                    (index + 1).to_string(),
                    sense.understanding().to_string(),
                    sense.tag().unwrap_or("").to_string(),
                )
            })
            .collect();
        let card = Card::new(
            draft.term().to_string(),
            meta.meaning().to_string(),
            meta.target_sentence().to_string(),
            senses,
        );
        let old = meta.source_context().to_string();
        let task = Task::from_operation(
            Input::new(
                card.clone(),
                meta.source_sentence().to_string(),
                old.clone(),
                request.to_string(),
            ),
            &language,
            operation,
        )?;
        let parts = write(
            &mut ExplanationWriter {
                client: self,
                observe,
            },
            &task,
            &language,
        )?;
        let (meta, parts) = match parts {
            Some(parts)
                if matches!(operation, Operation::Create)
                    && draft.reviewed_senses().len() > 1
                    && draft.term().split_whitespace().nth(1).is_some() =>
            {
                let translations = self.translate(draft, &meta, &parts, observe)?;
                let parts = parts.with_translations(translations.examples)?;
                let (failures, _) = judge(&card, &parts, &language).into_parts();
                anyhow::ensure!(
                    failures.is_empty(),
                    "Translated examples failed explanation review: {}",
                    failures.join(", ")
                );
                (
                    meta.with_source_translation(
                        translations.source_sentence,
                        translations.source_highlight,
                    )?,
                    Some(parts),
                )
            }
            rest => (meta, rest),
        };
        let context = parts
            .as_ref()
            .map_or_else(|| old.clone(), |parts| compose(parts, &language));
        Ok(meta
            .with_source_context(context)
            .with_explanation(Some(Record::new(parts, task.stamp().to_string(), old))))
    }

    fn explanation_text<F: FnMut(CostRecord) -> Result<()>>(
        &self,
        prompt: String,
        observe: &mut F,
    ) -> Result<String> {
        let schema = serde_json::from_str(include_str!("../../assets/explanation.schema.json"))?;
        let config = GenerationConfig::json(schema)?
            .with_thinking_level(ThinkingLevel::Low)
            .with_max_output_tokens(3000);
        Ok(self
            .explanation_request(prompt, config, observe)?
            .unwrap_or_default())
    }

    fn translate<F: FnMut(CostRecord) -> Result<()>>(
        &self,
        draft: &CardDraft,
        meta: &CardMeta,
        parts: &Parts,
        observe: &mut F,
    ) -> Result<Translations> {
        let input = json!({"learning_language": draft.pair().learning(), "reader_language": draft.pair().known(),
            "term": draft.term(), "sentence": meta.target_sentence(), "examples": parts.learning_examples(), "sense_context": parts.sense_context()});
        let mut prompt = String::from(
            "Translate each complete learning sentence independently into natural reader-language speech. The sense_context describes the selected situation. Use it when an example could describe either the intended figurative meaning or a literal one. Keep every participant and their number. Preserve who actually performs the action: if another person does the work, the translation must keep that distinction clear. Preserve whether the sentence asks for, requires or reports a completed result. When there is a deadline, it applies to the same event in both languages. Booking, ordering or merely starting the work cannot stand in for the finished result. A dictionary head may need a different number, case or wording in a real sentence. Return source_sentence for the main sentence and source_highlight as its exact nonempty substring translating the term here. Return one reader translation per example in the original order, with an empty string for an empty input. Output only the requested JSON.",
        );
        let catalog = crate::languages::catalog();
        if let Some(examples) = crate::explanation::sentence_translations(
            &draft.pair().known_profile(&catalog)?.prompt,
            &draft.pair().learning_profile(&catalog)?.prompt,
        )? {
            prompt.push_str(&format!("\n\nSentence translation examples:\n{examples}"));
        }
        prompt.push_str(&format!("\n\nTranslation input:\n{input}"));
        let schema = json!({"type":"object","properties":{
            "source_sentence":{"type":"string"},"source_highlight":{"type":"string"},
            "examples":{"type":"array","items":{"type":"string"},"minItems":3,"maxItems":3}
        },"required":["source_sentence","source_highlight","examples"],"additionalProperties":false});
        let config = GenerationConfig::json(schema)?.with_thinking_level(ThinkingLevel::Medium);
        let raw = match self.explanation_request(
            prompt.clone(),
            config.clone().with_max_output_tokens(8192),
            observe,
        )? {
            Some(raw) => raw,
            None => self
                .explanation_request(prompt, config.with_max_output_tokens(16384), observe)?
                .context("Sentence translation reached its output limit on both attempts")?,
        };
        Ok(serde_json::from_str(super::unfence(raw.trim()))?)
    }

    fn explanation_request<F: FnMut(CostRecord) -> Result<()>>(
        &self,
        prompt: String,
        config: GenerationConfig,
        observe: &mut F,
    ) -> Result<Option<String>> {
        let metered = self.request_stage(
            GenerationStage::Explanation,
            &Request::text(prompt, Some(config), None),
        )?;
        observe(metered.cost.clone())?;
        let raw = response_text(&metered.response);
        if matches!(
            metered.response,
            GeneratedResponse::External(ModelOutput::Refused)
        ) || metered
            .response
            .finish_reason()
            .is_some_and(|reason| !matches!(reason, "STOP" | "UNKNOWN" | "MAX_TOKENS"))
        {
            bail!(
                "Explanation provider returned no usable answer: {}",
                diagnosis(&metered.response)
            );
        }
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            return Ok(None);
        }
        anyhow::ensure!(
            !raw.trim().is_empty(),
            "Explanation provider returned no usable answer: {}",
            diagnosis(&metered.response)
        );
        Ok(Some(raw))
    }
}
