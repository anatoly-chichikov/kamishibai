//! Accounted explanation requests after metadata and phonetics have settled.

use anyhow::{Result, bail};

use super::super::protocol::{GenerationConfig, Request, ThinkingLevel};
use super::{
    GeminiClient, GeneratedResponse, Transport, diagnosis, prioritized_senses, response_text,
};
use crate::explanation::{
    Card, Input, Language, Operation, Record, Sense, Task, Writer, compose, write,
};
use crate::models::{GenerationStage, ModelOutput};
use crate::session::{CardDraft, CardMeta, CostRecord};

struct ExplanationWriter<'a, T: Transport, F> {
    client: &'a GeminiClient<T>,
    observe: &'a mut F,
}

impl<T: Transport, F: FnMut(CostRecord) -> Result<()>> Writer for ExplanationWriter<'_, T, F> {
    fn ask(&mut self, prompt: String) -> Result<String> {
        self.client.explanation_text(prompt, self.observe)
    }
}

impl<T: Transport> GeminiClient<T> {
    /// Replace only the explanation after all metadata facts have settled.
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
                card,
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
            || raw.trim().is_empty()
        {
            bail!(
                "Explanation provider returned no usable answer: {}",
                diagnosis(&metered.response)
            );
        }
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            return Ok(String::new());
        }
        Ok(raw)
    }
}
