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
        let writer =
            PromptTemplate::new(include_str!("../../assets/explanation_task.txt")).render(
                &substitutions(language, !input.card.senses.is_empty(), false),
            )?;
        let editor =
            PromptTemplate::new(include_str!("../../assets/explanation_edit.txt")).render(
                &substitutions(language, !input.card.senses.is_empty(), true),
            )?;
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

fn quoted(value: &str, language: &Language) -> String {
    let [open, close] = &language.context.quotes;
    format!("{open}{value}{close}")
}

fn joined(values: &[String], language: &Language) -> String {
    values
        .iter()
        .map(|value| quoted(value, language))
        .collect::<Vec<_>>()
        .join(", ")
}

fn substitutions(language: &Language, reviewed: bool, editor: bool) -> Vec<(&'static str, String)> {
    let examples = &language.names.examples;
    let block = examples.guidance();
    let known = &language.names.reader;
    let headers = &language.context.headers;
    let [open, close] = &language.context.quotes;
    let mut values = vec![
        ("{known}", known.clone()),
        ("{learning}", language.names.learning.clone()),
        ("{open}", open.clone()),
        ("{close}", close.clone()),
        ("{limits}", headers[2].trim_end_matches('.').to_string()),
        ("{office}", joined(&block.voice.office, language)),
        ("{vague}", joined(&block.voice.vague, language)),
        ("{active}", joined(&block.voice.active, language)),
        ("{narrated}", joined(&block.mistake.narrated, language)),
        ("{contrast}", quoted(&block.mistake.contrast, language)),
        (
            "{verbless_good}",
            quoted(&block.voice.verbless.good, language),
        ),
        (
            "{verbless_bad}",
            quoted(&block.voice.verbless.bad, language),
        ),
        (
            "{sentence_length}",
            if examples.worded() {
                "longer than fourteen words"
            } else {
                "longer than one short line of text"
            }
            .into(),
        ),
        (
            "{letters_rule}",
            if block.letters.is_empty() {
                String::new()
            } else {
                format!(
                    " Write these {known} letters wherever the spelling needs them: {}.",
                    joined(&block.letters, language)
                )
            },
        ),
    ];
    if editor {
        values.push((
            "{writing_examples}",
            examples
                .rewrites()
                .map(|(bad, good)| {
                    format!(
                        "  not {} but {}",
                        quoted(bad, language),
                        quoted(good, language)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    } else {
        let base = include_str!("../../assets/explanation_senses.txt")
            .lines()
            .collect::<Vec<_>>();
        let (lines, from) = if reviewed {
            (
                "- The card gives the meanings in \"senses\", in their order: the learner has reviewed this list, and the first sense is the one the card teaches. Write exactly one line for each sense, in the same order, none dropped, none merged with another and none added.",
                "- \"from\" is the number of the sense this line stands for, as it is given in \"senses\": \"1\" for the first line, \"2\" for the second and so on.",
            )
        } else {
            (base[0], base[1])
        };
        values.extend([
            ("{literally}", language.context.literally.clone()),
            ("{meaning}", headers[0].trim_end_matches('.').to_string()),
            ("{where}", headers[1].trim_end_matches('.').to_string()),
            ("{nuance}", headers[3].trim_end_matches('.').to_string()),
            ("{sense_lines}", lines.into()),
            ("{from_rule}", from.into()),
            ("{openers}", joined(&block.meaning.openers, language)),
            ("{reasons}", joined(&block.mistake.reasons, language)),
            (
                "{note_examples}",
                block
                    .meaning
                    .notes
                    .iter()
                    .map(|note| {
                        format!(
                            "  for {}: not {} but {}",
                            quoted(&note.head, language),
                            quoted(&note.bad, language),
                            quoted(&note.good, language)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        ]);
    }
    values
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Card, Input, Language, Task, substitutions};
    use crate::languages::{LanguagePromptExamples, explanation_scripts, prompt_scripted};

    fn examples() -> BTreeMap<String, LanguagePromptExamples> {
        serde_json::from_str(include_str!("../../assets/prompt_examples.json")).unwrap()
    }

    fn language(known: &str, learning: &str) -> Language {
        let profile = crate::languages::catalog();
        let sample = examples().remove(known).unwrap();
        Language {
            context: sample.explanation(),
            known: explanation_scripts(known),
            learning: explanation_scripts(learning),
            names: super::super::Names {
                reader: profile.item(known).unwrap().prompt,
                learning: profile.item(learning).unwrap().prompt,
                examples: sample,
            },
        }
    }

    fn task(language: &Language) -> Task {
        Task::from_input(
            Input::new(
                Card::new(
                    "chat".into(),
                    "cat".into(),
                    "Le chat dort.".into(),
                    Vec::new(),
                ),
                String::new(),
                String::new(),
                String::new(),
            ),
            language,
        )
        .unwrap()
    }

    fn pairs(readers: &[&str]) -> Vec<String> {
        let samples = examples();
        let mut faults = Vec::new();
        for known in readers {
            let mut baseline = None;
            for learning in samples.keys().filter(|code| code.as_str() != *known) {
                let mut language = language(known, learning);
                if let Some(field) = language
                    .names
                    .examples
                    .guidance()
                    .fault(known, language.names.examples.worded())
                {
                    faults.push(format!("{known}->{learning}: invalid {field}"));
                    continue;
                }
                let task = task(&language);
                let both = format!("{}\n{}", task.writer, task.editor);
                for (field, value) in language.names.examples.guidance().strings() {
                    if !both.contains(value) {
                        faults.push(format!("{known}->{learning}: missing {field}"));
                    }
                }
                if !prompt_scripted(&both, known) {
                    faults.push(format!("{known}->{learning}: foreign script"));
                }
                let injected = [false, true]
                    .into_iter()
                    .flat_map(|editor| substitutions(&language, false, editor))
                    .filter(|(slot, _)| {
                        ![
                            "{known}",
                            "{learning}",
                            "{sense_lines}",
                            "{from_rule}",
                            "{sentence_length}",
                            "{letters_rule}",
                        ]
                        .contains(slot)
                    })
                    .map(|(_, value)| value)
                    .collect::<Vec<_>>()
                    .join("\n");
                for (other, sample) in samples.iter().filter(|(code, _)| code.as_str() != *known) {
                    for (field, phrase) in sample.guidance().phrases() {
                        if !phrase.is_empty() && injected.contains(phrase) {
                            faults.push(format!("{known}->{learning}: {other}.{field} leaked"));
                        }
                    }
                }
                if both.lines().any(|line| line.ends_with([' ', '\t']))
                    || both.contains("\n\n\n")
                    || (language.names.examples.guidance().letters.is_empty()
                        && both.contains("letters wherever"))
                {
                    faults.push(format!(
                        "{known}->{learning}: empty spelling rule or dangling whitespace"
                    ));
                }
                language.names.learning = "TARGET_NAME".into();
                let normalized = task_with_target(&language);
                if baseline.as_ref().is_some_and(|base| base != &normalized) {
                    faults.push(format!(
                        "{known}->{learning}: learning language changed more than its name"
                    ));
                }
                baseline = Some(normalized);
            }
        }
        faults
    }

    fn task_with_target(language: &Language) -> (String, String) {
        let task = task(language);
        (task.writer, task.editor)
    }

    #[test]
    fn seed_readers_cannot_leak_examples_across_sixty_pairs() {
        let faults = pairs(&["ru", "en"]);
        assert!(
            faults.is_empty(),
            "seed reader prompts broke their language policy: {faults:#?}"
        );
    }

    #[test]
    fn every_reader_cannot_leak_examples_across_all_930_pairs() {
        let samples = examples();
        let readers = samples.keys().map(String::as_str).collect::<Vec<_>>();
        let faults = pairs(&readers);
        assert!(
            faults.is_empty(),
            "the 930-pair prompt catalog broke its language policy: {faults:#?}"
        );
    }

    #[test]
    fn templates_and_sense_rules_cannot_embed_language_samples() {
        let language = language("en", "fr");
        let senses = [false, true]
            .into_iter()
            .flat_map(|reviewed| substitutions(&language, reviewed, false))
            .filter(|(slot, _)| ["{sense_lines}", "{from_rule}"].contains(slot))
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        let sources = [
            include_str!("../../assets/explanation_task.txt"),
            include_str!("../../assets/explanation_edit.txt"),
            include_str!("../../assets/explanation_senses.txt"),
        ]
        .into_iter()
        .chain(senses.iter().map(String::as_str));
        let allowed = [
            "term",
            "translation",
            "sentence",
            "sentence_known",
            "old",
            "senses",
            "request",
            "meanings",
            "head",
            "note",
            "from",
            "literal",
            "where",
            "limits",
            "nuance",
            "why",
            "example",
            "text",
            "",
            ";",
            ",",
            " — ",
            "{literally} —",
            "1",
            "2",
        ];
        let literals = regex::Regex::new(r#""([^"\n]*)""#).unwrap();
        let markers = regex::Regex::new("⟦([^⟦⟧]*)⟧").unwrap();
        assert!(
            sources.into_iter().all(|source| source
                .chars()
                .all(|ch| !ch.is_alphabetic() || ch.is_ascii())
                && !source.contains(['«', '»', '“', '”'])
                && literals
                    .captures_iter(source)
                    .all(|capture| allowed.contains(&&capture[1]))
                && markers
                    .captures_iter(source)
                    .all(|capture| ["1", "2", " "].contains(&&capture[1]))),
            "a shared template or sense instruction embedded a concrete language sample"
        );
    }

    #[test]
    fn sentence_length_cannot_count_words_in_unspaced_languages() {
        let samples = examples();
        let wrong = samples
            .keys()
            .filter(|code| !samples[*code].worded())
            .filter(|code| {
                let language = language(code, "en");
                let task = task(&language);
                !task.writer.contains("longer than one short line of text")
                    || task.writer.contains("longer than fourteen words")
                    || !task.editor.contains("longer than one short line of text")
            })
            .collect::<Vec<_>>();
        assert!(
            wrong.is_empty(),
            "unspaced readers received a word-count instruction: {wrong:?}"
        );
    }
}
