//! Bounded write, repair, edit, and review transaction over an injected provider.

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Card, Language, Parts, judge, tidy};
use crate::prompt::PromptTemplate;

/// The model boundary for an explanation request, including edits and repairs.
pub trait Writer {
    /// Return one response, propagating transport and provider failures.
    fn ask(&mut self, prompt: String) -> Result<String>;
}

/// Facts and the learner's request for one explanation transaction.
#[derive(Clone, Debug, Serialize)]
pub struct Input {
    #[serde(flatten)]
    card: Card,
    sentence_known: String,
    old: String,
    request: String,
}

impl Input {
    /// Bind the approved card to its metadata draft and optional learner request.
    #[must_use]
    pub fn new(card: Card, sentence_known: String, old: String, request: String) -> Self {
        Self {
            card,
            sentence_known,
            old,
            request,
        }
    }
}

/// Fully interpolated immutable instructions for one explanation transaction.
#[derive(Clone, Debug)]
pub struct Task {
    input: Input,
    writer: String,
    editor: String,
    stamp: String,
}

impl Task {
    /// Render the catalog's reader conventions with checked template substitution.
    pub fn from_input(input: Input, language: &Language) -> Result<Self> {
        let known = &language.names.reader;
        let learning = &language.names.learning;
        let [open, close] = &language.context.quotes;
        let headers = &language.context.headers;
        let base = include_str!("../../assets/explanation_senses.txt")
            .lines()
            .collect::<Vec<_>>();
        let (lines, from) = if input.card.senses.is_empty() {
            (base[0], base[1])
        } else {
            (
                "- The card gives the meanings in \"senses\", in their order: the learner has reviewed this list, and the first sense is the one the card teaches. Write exactly one line for each sense, in the same order, none dropped, none merged with another and none added.",
                "- \"from\" is the number of the sense this line stands for, as it is given in \"senses\": \"1\" for the first line, \"2\" for the second and so on.",
            )
        };
        let writer = PromptTemplate::new(include_str!("../../assets/explanation_task.txt"))
            .render(&[
                ("{known}", known.clone()),
                ("{learning}", learning.clone()),
                ("{open}", open.clone()),
                ("{close}", close.clone()),
                ("{literally}", language.context.literally.clone()),
                ("{meaning}", headers[0].trim_end_matches('.').to_string()),
                ("{where}", headers[1].trim_end_matches('.').to_string()),
                ("{limits}", headers[2].trim_end_matches('.').to_string()),
                ("{nuance}", headers[3].trim_end_matches('.').to_string()),
                ("{notes}", language.context.notes.clone()),
                ("{sense_lines}", lines.to_string()),
                ("{from_rule}", from.to_string()),
            ])?;
        let editor = PromptTemplate::new(include_str!("../../assets/explanation_edit.txt"))
            .render(&[
                ("{known}", known.clone()),
                ("{learning}", learning.clone()),
                ("{open}", open.clone()),
                ("{close}", close.clone()),
                ("{limits}", headers[2].clone()),
                ("{notes}", language.context.notes.clone()),
            ])?;
        let stamp = Sha256::digest(format!("{writer}\n{editor}"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            input,
            writer,
            editor,
            stamp,
        })
    }

    /// Identify the exact rendered assignment without including card facts or secrets.
    #[must_use]
    pub fn stamp(&self) -> &str {
        &self.stamp
    }

    fn prompt(
        &self,
        instructions: &str,
        parts: Option<&Parts>,
        fails: &[String],
    ) -> Result<String> {
        let mut prompt = format!(
            "{instructions}\n\nCard input:\n{}",
            serde_json::to_string_pretty(&self.input)?
        );
        if let Some(parts) = parts {
            prompt.push_str(&format!(
                "\n\nWriter's JSON:\n{}",
                serde_json::to_string(parts)?
            ));
        }
        if !fails.is_empty() {
            prompt.push_str(&format!("\n\nYour answer breaks these rules:\n- {}\nAnswer again with the whole JSON. Change only what is needed to fix them and keep every other rule. A fix must leave a whole natural phrase, never a cut-off one.", fails.join("\n- ")));
        }
        Ok(prompt)
    }
}

fn ask(writer: &mut impl Writer, prompt: String) -> Result<Option<Parts>> {
    let raw = writer.ask(prompt)?;
    let raw = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .and_then(|text| text.trim().strip_suffix("```"))
        .unwrap_or(raw.trim());
    Ok(serde_json::from_str::<Parts>(raw)
        .ok()
        .map(|parts| tidy(&parts)))
}

/// Write and edit with at most two repairs, retaining the draft without requests above five meanings.
pub fn write(writer: &mut impl Writer, task: &Task, language: &Language) -> Result<Option<Parts>> {
    let count = if task.input.card.senses.is_empty() {
        task.input
            .card
            .translation
            .split(';')
            .filter(|group| !group.trim().is_empty())
            .count()
    } else {
        task.input.card.senses.len()
    };
    if count > 5 {
        return Ok(None);
    }
    let Some(mut current) = ask(writer, task.prompt(&task.writer, None, &[])?)? else {
        return Ok(None);
    };
    let (fails, _) = judge(&task.input.card, &current, language).into_parts();
    if !fails.is_empty()
        && let Some(fixed) = ask(writer, task.prompt(&task.writer, Some(&current), &fails)?)?
    {
        current = fixed;
    }
    if let Some(edited) = ask(writer, task.prompt(&task.editor, Some(&current), &[])?)? {
        current = edited;
        let (fails, _) = judge(&task.input.card, &current, language).into_parts();
        if !fails.is_empty()
            && let Some(fixed) = ask(writer, task.prompt(&task.writer, Some(&current), &fails)?)?
        {
            current = fixed;
        }
    }
    let (fails, _) = judge(&task.input.card, &current, language).into_parts();
    Ok(fails.is_empty().then_some(current))
}
