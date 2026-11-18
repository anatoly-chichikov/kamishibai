//! Bounded write, repair, edit, and review transaction over an injected provider.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

use super::{Card, Example, Language, Parts, Sense, judge, tidy};
use crate::prompt::PromptTemplate;

/// The model boundary for an explanation request, including edits and repairs.
pub trait Writer {
    /// Return one response, propagating transport and provider failures.
    fn ask(&mut self, prompt: String) -> Result<String>;
}

/// Select creation or revision independently of the retained metadata draft.
#[derive(Clone, Copy, Debug)]
pub enum Operation {
    /// Explain a newly generated card, including every repair and editor pass.
    Create,
    /// Revisit an existing card, even when its correction note is empty.
    Revise,
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
    prompts: Prompts,
    stamp: String,
    draft_words: Option<String>,
}

#[derive(Clone, Debug)]
struct Prompts {
    writer: String,
    editor: String,
}

impl Task {
    /// Prepare a fresh explanation while retaining its metadata draft as input.
    pub fn from_input(input: Input, language: &Language) -> Result<Self> {
        Self::from_operation(input, language, Operation::Create)
    }

    /// Render a declared operation without inferring it from draft or request text.
    pub fn from_operation(input: Input, language: &Language, operation: Operation) -> Result<Self> {
        let draft_words =
            matches!(operation, Operation::Create).then(|| format!(" {} ", words(&input.old)));
        let (operation, old) = match operation {
            Operation::Create => (
                "create",
                "an unverified metadata draft. Start from the card sentence and the reviewed senses when supplied. Check any useful fact independently. Compose a new example in a different situation. Do not borrow the wording, participants or translation of an illustration in this draft",
            ),
            Operation::Revise => (
                "revise",
                "the explanation that is being replaced. Check its facts against the sentence and the learner's request before reusing them",
            ),
        };
        let reviewed = !input.card.senses.is_empty();
        let values = |editor| {
            let mut values = substitutions(language, reviewed, editor);
            values.push(("{operation}", operation.to_string()));
            values.push(("{old_role}", old.to_string()));
            values
        };
        let examples = pair_examples(language, reviewed)?;
        let render = |template, editor| -> Result<String> {
            let mut instructions = PromptTemplate::new(template).render(&values(editor))?;
            instructions.push_str(&examples);
            Ok(instructions)
        };
        let writer = render(include_str!("../../assets/explanation_task.txt"), false)?;
        let editor = render(include_str!("../../assets/explanation_edit.txt"), true)?;
        let stamp = Sha256::digest(format!("{writer}\n{editor}"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            input,
            prompts: Prompts { writer, editor },
            stamp,
            draft_words,
        })
    }

    /// Identify the exact rendered assignment without including card facts or secrets.
    #[must_use]
    pub fn stamp(&self) -> &str {
        &self.stamp
    }

    fn copied(&self, example: &Example) -> bool {
        let Some(draft) = &self.draft_words else {
            return false;
        };
        let learning = words(&example.text);
        let reader = words(&example.translation);
        !learning.is_empty()
            && !reader.is_empty()
            && (draft.contains(&format!(" {learning} {reader} "))
                || draft.contains(&format!(" {reader} {learning} ")))
    }

    fn prompt(
        &self,
        instructions: &str,
        review: Option<(&Parts, &Language)>,
        fails: &[String],
    ) -> Result<String> {
        let mut prompt = format!(
            "{instructions}\n\nCard input:\n{}",
            serde_json::to_string_pretty(&self.input)?
        );
        if let Some((parts, language)) = review {
            prompt.push_str(&format!("\n\nRead the card as it will appear to the learner. Say each displayed line aloud before editing the JSON. If a list puts unlike things after the same dash, keep just one natural completion. If a sentence about how to say a phrase has no natural subject or verb in the reader language, show the phrase directly. Preserve who acts and whether the action is complete in both translations.\n{}",
                super::compose(parts, language)));
            if self.input.card.senses.len() > 1 {
                let rows: Vec<_> = self.input.card.senses.iter().filter_map(|sense| {
                    let row = parts.meanings.iter().find(|row| row.from == sense.number)?;
                    Some(serde_json::json!({"number":sense.number, "approved":sense.understanding,
                        "head":row.head, "note":row.note}))
                }).collect();
                prompt.push_str(&format!("\n\nCheck every proposed note against its own approved sense. Remove any item from a different sense, including a literal image suggested by the term. Keep the row and its number.\nReviewed row check:\n{}", serde_json::to_string(&rows)?));
            }
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

fn words(text: &str) -> String {
    text.unicode_words()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Calibration {
    card: Card,
    sentence_known: String,
    parts: Parts,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_notes: Option<String>,
}

/// Project the exact pair's approved example sentences for the metadata author.
pub(crate) fn sentence_translations(reader: &str, learning: &str) -> Result<Option<String>> {
    let Some(text) = crate::languages::explanation_pair(reader, learning)? else {
        return Ok(None);
    };
    let samples: Vec<Calibration> = serde_json::from_str(&text)?;
    let sentences = samples
        .into_iter()
        .map(|sample| {
            serde_json::json!({
                "target_sentence": sample.card.sentence,
                "source_sentence": sample.sentence_known,
            })
        })
        .collect::<Vec<_>>();
    Ok(Some(serde_json::to_string(&sentences)?))
}

fn pair_examples(language: &Language, reviewed: bool) -> Result<String> {
    let reader = &language.names.reader;
    let learning = &language.names.learning;
    let Some(mut text) = crate::languages::explanation_pair(reader, learning)? else {
        return Ok(String::new());
    };
    if reviewed {
        let mut samples: Vec<Calibration> = serde_json::from_str(&text)?;
        for sample in &mut samples {
            sample.card.senses = sample
                .parts
                .meanings
                .iter_mut()
                .enumerate()
                .map(|(index, row)| {
                    row.from = (index + 1).to_string();
                    Sense::new(row.from.clone(), row.head.clone(), String::new())
                })
                .collect();
        }
        text = serde_json::to_string_pretty(&samples)?;
    }
    Ok(format!(
        "\n\nComplete examples for this exact pair, reader {reader} and learning {learning}:\nRead each head together with its note. Each example translation describes the people in its own sentence. Use the examples for style and scope only. Review notes belong to the sample, never to your answer.\n{text}"
    ))
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

fn phrases(parts: &Parts, language: &Language) -> std::collections::BTreeSet<String> {
    let pattern =
        regex::Regex::new("⟦[^⟦⟧]*⟧").expect("invariant: learning marker pattern must compile");
    parts
        .strings()
        .into_iter()
        .flat_map(|text| {
            pattern
                .find_iter(&super::layout::mark(text, language))
                .map(|hit| hit.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn preserves(current: &Parts, edited: &Parts, language: &Language) -> bool {
    phrases(edited, language).is_subset(&phrases(current, language))
        && current
            .meanings
            .iter()
            .map(|line| &line.from)
            .eq(edited.meanings.iter().map(|line| &line.from))
        && current.sections().into_iter().zip(edited.sections()).all(
            |((_, _, before), (_, _, after))| after.text.is_empty() || before.text == after.text,
        )
}

fn revision(original: &Parts, edited: &Parts) -> Parts {
    let mut source = original.clone();
    for (before, after) in [
        (&mut source.r#where.example, &edited.r#where.example),
        (&mut source.limits.example, &edited.limits.example),
        (&mut source.nuance.example, &edited.nuance.example),
    ] {
        if before.text != after.text {
            before.text.clear();
            before.translation.clear();
        }
    }
    source
}

/// Write and edit with at most two repairs, rejecting edits that change learning text.
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
    let Some(mut current) = ask(writer, task.prompt(&task.prompts.writer, None, &[])?)? else {
        return Ok(None);
    };
    let (mut fails, _) = judge(&task.input.card, &current, language).into_parts();
    for (section, _, example) in current.sections() {
        if task.copied(example) {
            fails.push(format!("{section} example: the entire bilingual example was copied from the unverified metadata draft. Write a fresh illustration in a different situation and translate the whole sentence independently, or omit this optional example"));
        }
    }
    if !fails.is_empty()
        && let Some(fixed) = ask(
            writer,
            task.prompt(&task.prompts.writer, Some((&current, language)), &fails)?,
        )?
    {
        current = fixed;
    }
    if let Some(edited) = ask(
        writer,
        task.prompt(&task.prompts.editor, Some((&current, language)), &[])?,
    )? {
        let accepted = judge(&task.input.card, &current, language)
            .into_parts()
            .0
            .is_empty();
        let intact = !accepted || preserves(&current, &edited, language);
        let original = current;
        current = edited;
        let (mut fails, _) = judge(&task.input.card, &current, language).into_parts();
        if !intact {
            fails.push("learning language: the editor changed the learning text; the supplied JSON is the original writer response, preserve its marked phrases and example texts exactly while editing only the reader prose".into());
            fails.push("editing: any example the editor rewrote or removed is now empty in the supplied JSON; keep it empty, do not restore the original or add a replacement; keep every from and the row order, qualify only an ambiguous first head with its sentence object, and correct the notes without merging senses".into());
            let allowed = phrases(&original, language);
            let added = phrases(&current, language)
                .difference(&allowed)
                .cloned()
                .collect::<Vec<_>>();
            if !added.is_empty() {
                fails.push(format!(
                    "learning language: these phrases were not in the original: {}; original phrases allowed to survive: {}; preserve spelling and capitalization exactly, rephrase the reader prose around them or remove the optional section",
                    added.join(", "),
                    allowed.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        if !fails.is_empty() {
            let source = if intact {
                current.clone()
            } else {
                revision(&original, &current)
            };
            if let Some(fixed) = ask(
                writer,
                task.prompt(&task.prompts.editor, Some((&source, language)), &fails)?,
            )? {
                if accepted && !preserves(&original, &fixed, language) {
                    return Ok(None);
                }
                current = fixed;
            } else if !intact {
                return Ok(None);
            }
        }
    }
    for example in [
        &mut current.r#where.example,
        &mut current.limits.example,
        &mut current.nuance.example,
    ] {
        if task.copied(example) {
            example.text.clear();
            example.translation.clear();
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

    struct Replies {
        answers: std::collections::VecDeque<String>,
    }

    struct Capture(String);

    struct Feedback {
        answers: std::collections::VecDeque<String>,
        diagnosis: String,
    }

    struct Revision {
        answers: std::collections::VecDeque<String>,
        editor: String,
        reviewed: bool,
    }

    impl super::Writer for Revision {
        fn ask(&mut self, prompt: String) -> anyhow::Result<String> {
            if let Some(answer) = self.answers.pop_front() {
                return Ok(answer);
            }
            self.reviewed = prompt.starts_with(&self.editor);
            let (_, source) = prompt.split_once("Writer's JSON:\n").unwrap();
            let (source, _) = source
                .split_once("\n\nYour answer breaks these rules:")
                .unwrap();
            Ok(source.into())
        }
    }

    impl super::Writer for Feedback {
        fn ask(&mut self, prompt: String) -> anyhow::Result<String> {
            if let Some((_, diagnosis)) = prompt.split_once("Your answer breaks these rules:") {
                self.diagnosis = diagnosis.to_string();
            }
            self.answers
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("unexpected extra feedback request"))
        }
    }

    impl super::Writer for Capture {
        fn ask(&mut self, prompt: String) -> anyhow::Result<String> {
            self.0 = prompt;
            Ok(String::new())
        }
    }

    #[test]
    fn pair_examples_cannot_escape_their_exact_pair_across_930_requests() {
        let samples = examples();
        let mut faults = Vec::new();
        for known in samples.keys() {
            for learning in samples.keys().filter(|code| *code != known) {
                let language = language(known, learning);
                let mut writer = Capture(String::new());
                super::write(&mut writer, &task(&language), &language).unwrap();
                let expected = known == "ru" && learning == "en";
                let translations =
                    super::sentence_translations(&language.names.reader, &language.names.learning)
                        .unwrap();
                if translations.is_some() != expected {
                    faults.push(format!(
                        "{known}->{learning}: wrong pair in metadata translations"
                    ));
                }
                if !prompt_scripted(&writer.0, known) {
                    faults.push(format!("{known}->{learning}: foreign script in request"));
                }
                for text in [
                    "The bank closes at five.",
                    "Don't spill the beans about the party.",
                    "I borrowed this book from a friend.",
                    "I had the roof repaired last week.",
                    "He is a bright kid with plenty of ideas.",
                    "She taught French for ten years.",
                    "He has a deep understanding of local history.",
                    "Those two players are dark horses.",
                    "The museum admits visitors with tickets.",
                    "Both men are her kin.",
                    "The sailors came ashore at dawn.",
                ] {
                    if writer.0.contains(text) != expected {
                        faults.push(format!("{known}->{learning}: {text}"));
                    }
                }
            }
        }
        assert!(
            faults.is_empty(),
            "pair examples were absent or crossed a language boundary: {faults:?}"
        );
    }

    impl super::Writer for Replies {
        fn ask(&mut self, _prompt: String) -> anyhow::Result<String> {
            self.answers
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("unexpected extra quality request"))
        }
    }

    #[test]
    fn quality_editing_cannot_corrupt_learning_phrases_or_examples() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/quality.json"
        ))
        .unwrap();
        let corrupted = cases[4..]
            .iter()
            .filter(|case| {
                let job = &case["job"];
                let language = Language::from_codes(
                    job["known"].as_str().unwrap(),
                    job["learning"].as_str().unwrap(),
                )
                .unwrap();
                let task = Task::from_input(
                    Input::new(
                        serde_json::from_value(job["card"].clone()).unwrap(),
                        job["sentence_known"].as_str().unwrap().into(),
                        String::new(),
                        String::new(),
                    ),
                    &language,
                )
                .unwrap();
                let answers = case["attempts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(std::iter::once(&case["attempts"][0]))
                    .map(serde_json::Value::to_string)
                    .collect();
                let mut writer = Replies { answers };
                let expected =
                    super::tidy(&serde_json::from_value(case["attempts"][0].clone()).unwrap());
                super::write(&mut writer, &task, &language).unwrap() != Some(expected)
            })
            .map(|case| case["job"]["id"].clone())
            .collect::<Vec<_>>();
        assert!(
            corrupted.is_empty(),
            "editor corrupted previously valid learning text: {corrupted:?}"
        );
    }

    #[test]
    fn quality_editing_cannot_discard_safe_reader_changes_or_section_removal() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/quality.json"
        ))
        .unwrap();
        let original = cases[4]["attempts"][0].clone();
        let mut rewritten = original.clone();
        rewritten["where"]["text"] = serde_json::json!(
            original["where"]["text"]
                .as_str()
                .unwrap()
                .replace("Za stolom", "Za ručkom")
        );
        let mut removed = original.clone();
        removed["nuance"] =
            serde_json::json!({"text":"", "why":"", "example":{"text":"", "translation":""}});
        let language = Language::from_codes("sr", "fa").unwrap();
        let job = &cases[4]["job"];
        let task = Task::from_input(
            Input::new(
                serde_json::from_value(job["card"].clone()).unwrap(),
                job["sentence_known"].as_str().unwrap().into(),
                String::new(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let lost = [rewritten, removed]
            .into_iter()
            .filter(|edited| {
                let mut writer = Replies {
                    answers: [original.to_string(), edited.to_string()].into(),
                };
                super::write(&mut writer, &task, &language).unwrap()
                    != Some(super::tidy(
                        &serde_json::from_value(edited.clone()).unwrap(),
                    ))
            })
            .count();
        assert_eq!(
            lost, 0,
            "safe reader editing or unsupported-section removal was discarded"
        );
    }

    #[test]
    fn quality_learning_corruption_cannot_survive_a_failed_editor_repair() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/quality.json"
        ))
        .unwrap();
        let accepted = cases[4..]
            .iter()
            .flat_map(|case| {
                let job = &case["job"];
                let language = Language::from_codes(
                    job["known"].as_str().unwrap(),
                    job["learning"].as_str().unwrap(),
                )
                .unwrap();
                let task = Task::from_input(
                    Input::new(
                        serde_json::from_value(job["card"].clone()).unwrap(),
                        job["sentence_known"].as_str().unwrap().into(),
                        String::new(),
                        String::new(),
                    ),
                    &language,
                )
                .unwrap();
                [case["attempts"][1].to_string(), "unparsed".into()]
                    .into_iter()
                    .map(move |repair| {
                        let mut writer = Replies {
                            answers: [
                                case["attempts"][0].to_string(),
                                case["attempts"][1].to_string(),
                                repair,
                            ]
                            .into(),
                        };
                        super::write(&mut writer, &task, &language)
                            .unwrap()
                            .is_some()
                    })
            })
            .any(|accepted| accepted);
        assert!(
            !accepted,
            "an unrepaired editor change installed corrupt learning text"
        );
    }

    #[test]
    fn quality_editing_cannot_reject_removal_or_reordering_of_unchanged_phrases() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/editor_deletions.json"
        ))
        .unwrap();
        let rejected = cases
            .iter()
            .filter(|case| {
                let job = &case["job"];
                let language = Language::from_codes(
                    job["known"].as_str().unwrap(),
                    job["learning"].as_str().unwrap(),
                )
                .unwrap();
                let task = Task::from_input(
                    Input::new(
                        serde_json::from_value(job["card"].clone()).unwrap(),
                        job["sentence_known"].as_str().unwrap().into(),
                        String::new(),
                        String::new(),
                    ),
                    &language,
                )
                .unwrap();
                let mut writer = Replies {
                    answers: case["attempts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(serde_json::Value::to_string)
                        .collect(),
                };
                super::write(&mut writer, &task, &language)
                    .unwrap()
                    .is_none()
            })
            .count();
        assert_eq!(
            rejected, 0,
            "safe deletion or reordering of unchanged learning phrases was rejected"
        );
    }

    #[test]
    fn quality_quote_normalization_cannot_discard_an_unchanged_learning_phrase() {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/marker_edits.json"
        ))
        .unwrap();
        let language = Language::from_codes("ru", "en").unwrap();
        let task = Task::from_input(
            Input::new(
                serde_json::from_value(case["job"]["card"].clone()).unwrap(),
                case["job"]["sentence_known"].as_str().unwrap().into(),
                String::new(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let mut writer = Replies {
            answers: case["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .map(serde_json::Value::to_string)
                .collect(),
        };
        let expected = super::tidy(&serde_json::from_value(case["attempts"][1].clone()).unwrap());
        assert_eq!(
            super::write(&mut writer, &task, &language).ok().flatten(),
            Some(expected),
            "changing quotation syntax discarded an unchanged learning phrase"
        );
    }

    #[test]
    fn quality_quote_normalization_cannot_hide_a_changed_learning_word() {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/marker_edits.json"
        ))
        .unwrap();
        let current = serde_json::from_value(case["attempts"][0].clone()).unwrap();
        let mut edited: super::Parts = serde_json::from_value(case["attempts"][1].clone()).unwrap();
        edited.nuance.text = edited.nuance.text.replace("sweet", "sweat");
        assert!(
            !super::preserves(&current, &edited, &language("ru", "en")),
            "a real word change escaped through quotation normalization"
        );
    }

    #[test]
    fn quality_repair_feedback_cannot_hide_the_changed_learning_phrase() {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/editor_case.json"
        ))
        .unwrap();
        let language = language("ru", "en");
        let task = Task::from_input(
            Input::new(
                serde_json::from_value(case["job"]["card"].clone()).unwrap(),
                case["job"]["sentence_known"].as_str().unwrap().into(),
                String::new(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let mut writer = Feedback {
            answers: case["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .map(serde_json::Value::to_string)
                .collect(),
            diagnosis: String::new(),
        };
        super::write(&mut writer, &task, &language).unwrap();
        assert!(
            ["⟦suite⟧", "⟦Suite⟧"]
                .iter()
                .all(|phrase| writer.diagnosis.contains(phrase)),
            "repair feedback hid the changed phrase or the original spelling"
        );
    }

    #[test]
    fn quality_reader_repair_cannot_change_accepted_learning_text() {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/editor_repair.json"
        ))
        .unwrap();
        let language = language("ru", "en");
        let task = Task::from_input(
            Input::new(
                serde_json::from_value(case["job"]["card"].clone()).unwrap(),
                case["job"]["sentence_known"].as_str().unwrap().into(),
                String::new(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let mut writer = Replies {
            answers: case["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .map(serde_json::Value::to_string)
                .collect(),
        };
        assert!(
            super::write(&mut writer, &task, &language)
                .unwrap()
                .is_none(),
            "repairing reader prose bypassed protection of accepted learning text"
        );
    }

    #[test]
    fn quality_editor_repair_cannot_resurrect_a_replaced_example() {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/editor_sense.json"
        ))
        .unwrap();
        let language = language("ru", "en");
        let task = Task::from_input(
            Input::new(
                serde_json::from_value(case["job"]["card"].clone()).unwrap(),
                case["job"]["sentence_known"].as_str().unwrap().into(),
                String::new(),
                String::new(),
            ),
            &language,
        )
        .unwrap();
        let mut writer = Revision {
            answers: case["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .take(2)
                .map(serde_json::Value::to_string)
                .collect(),
            editor: task.prompts.editor.clone(),
            reviewed: false,
        };
        let mut expected: super::Parts =
            serde_json::from_value(case["attempts"][0].clone()).unwrap();
        expected.r#where.example.text.clear();
        expected.r#where.example.translation.clear();
        let result = super::write(&mut writer, &task, &language).unwrap();
        assert_eq!(
            (result, writer.reviewed),
            (Some(expected), true),
            "a repeated writer restored an example the editor tried to replace"
        );
    }

    #[test]
    fn quality_writer_cannot_omit_observed_semantic_failure_constraints() {
        let task = task(&language("ru", "en"));
        let missing = [
            "Do not merge another sense into the first note",
            "Do not turn a typical situation into a necessary condition",
            "A more specific alternative does not make the original word wrong",
            "A transparent word or compound has no literal line",
            "An example must demonstrate the card's sense, never a contrasting sense",
            "Choose a different fact for each section",
            "Check every letter of every learning phrase",
        ]
        .into_iter()
        .filter(|rule| !task.prompts.writer.contains(rule))
        .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "writer lacks safeguards for observed semantic failures: {missing:?}"
        );
    }

    fn calibrations(task: &Task, role: &str) -> Vec<super::Calibration> {
        let prompt = task.prompt(role, None, &[]).unwrap();
        let (instructions, _) = prompt.split_once("\n\nCard input:").unwrap();
        let start = instructions.find("\n[").unwrap();
        serde_json::from_str(&instructions[start..]).unwrap()
    }

    #[test]
    fn reviewed_cards_cannot_show_translation_fragments_as_approved_sense_numbers() {
        let language = language("ru", "en");
        let mut input = task(&language).input;
        input.card.senses = vec![super::Sense::new("1".into(), "кошка".into(), String::new())];
        input.old = "Черновое объяснение".into();
        let task = Task::from_input(input, &language).unwrap();
        let consistent =
            [&task.prompts.writer, &task.prompts.editor]
                .into_iter()
                .all(|role| {
                    let samples = calibrations(&task, role);
                    !samples.is_empty()
                        && samples.iter().all(|sample| {
                            let senses = &sample.card.senses;
                            let rows = &sample.parts.meanings;
                            senses.len() == rows.len()
                                && rows.iter().zip(senses).enumerate().all(
                                    |(index, (row, sense))| {
                                        row.from == (index + 1).to_string()
                                            && sense.number == row.from
                                            && !sense.understanding.is_empty()
                                    },
                                )
                        })
                });
        assert!(
            consistent,
            "the writer or editor demonstrated unreviewed rows for a reviewed card"
        );
    }

    #[test]
    fn unreviewed_cards_cannot_gain_numbered_anchors_from_pair_examples() {
        let task = task(&language("ru", "en"));
        let consistent = calibrations(&task, &task.prompts.writer)
            .into_iter()
            .all(|sample| {
                sample.card.senses.is_empty()
                    && sample.parts.meanings.iter().all(|row| {
                        row.from.is_empty() || sample.card.translation.contains(&row.from)
                    })
            });
        assert!(
            consistent,
            "an unreviewed sample lost its translation anchors"
        );
    }

    #[test]
    fn fresh_cards_cannot_treat_metadata_prose_as_an_existing_explanation() {
        let language = language("es", "fa");
        let mut input = task(&language).input;
        input.old = "unverified draft".into();
        let creation = Task::from_input(input.clone(), &language).unwrap();
        let revision = Task::from_operation(input, &language, super::Operation::Revise).unwrap();
        assert!(
            [&creation.prompts.writer, &creation.prompts.editor]
                .iter()
                .all(|role| role.contains("unverified metadata draft")
                    && !role.contains("the explanation that is being replaced"))
                && revision
                    .prompts
                    .writer
                    .contains("the explanation that is being replaced"),
            "creation and revision assigned the same authority to metadata prose"
        );
    }

    fn metadata_example() -> (Input, super::Parts) {
        let case: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/explanation/metadata_example.json"
        ))
        .unwrap();
        let source = &case["input"];
        let input = Input::new(
            serde_json::from_value(source.clone()).unwrap(),
            source["sentence_known"].as_str().unwrap().into(),
            source["old"].as_str().unwrap().into(),
            String::new(),
        );
        (
            input,
            serde_json::from_value(case["parts"].clone()).unwrap(),
        )
    }

    #[test]
    fn a_fresh_card_cannot_publish_a_bilingual_example_copied_from_its_metadata_draft() {
        let language = language("ru", "en");
        let (input, mut parts) = metadata_example();
        let task = Task::from_input(input, &language).unwrap();
        let mut writer = Replies {
            answers: vec![serde_json::to_string(&parts).unwrap(); 4].into(),
        };
        parts.nuance.example.text.clear();
        parts.nuance.example.translation.clear();
        assert_eq!(
            super::write(&mut writer, &task, &language).unwrap(),
            Some(parts),
            "the complete copied bilingual example survived the fresh-card transaction"
        );
    }

    #[test]
    fn a_corrected_reader_translation_cannot_be_discarded_with_the_copied_example() {
        let language = language("ru", "en");
        let (input, mut parts) = metadata_example();
        let task = Task::from_input(input, &language).unwrap();
        let first = serde_json::to_string(&parts).unwrap();
        parts.nuance.example.translation = "Они всего лишь мелкие сошки.".into();
        let fixed = serde_json::to_string(&parts).unwrap();
        let mut writer = Replies {
            answers: [first, fixed.clone(), fixed].into(),
        };
        assert_eq!(
            super::write(&mut writer, &task, &language).unwrap(),
            Some(parts),
            "the writer lost an independently translated example"
        );
    }

    #[test]
    fn a_revision_cannot_lose_its_existing_bilingual_example_to_the_freshness_check() {
        let language = language("ru", "en");
        let (input, parts) = metadata_example();
        let task = Task::from_operation(input, &language, super::Operation::Revise).unwrap();
        let answer = serde_json::to_string(&parts).unwrap();
        let mut writer = Replies {
            answers: [answer.clone(), answer].into(),
        };
        assert_eq!(
            super::write(&mut writer, &task, &language).unwrap(),
            Some(parts),
            "the editor discarded a pre-existing example from a requested revision"
        );
    }

    #[test]
    fn an_editor_cannot_review_a_later_note_against_the_first_or_literal_sense() {
        let (input, parts) = metadata_example();
        let language = language("ru", "en");
        let task = Task::from_input(input, &language).unwrap();
        let prompt = task
            .prompt(&task.prompts.editor, Some((&parts, &language)), &[])
            .unwrap();
        let rows: Option<Vec<serde_json::Value>> = prompt
            .split_once("Reviewed row check:\n")
            .and_then(|(_, tail)| {
                serde_json::Deserializer::from_str(tail)
                    .into_iter::<Vec<serde_json::Value>>()
                    .next()?
                    .ok()
            });
        assert_eq!(
            rows.and_then(|rows| rows.get(1).cloned()),
            Some(
                serde_json::json!({"number": "2", "approved": "малыши", "head": "малыши", "note": "дети или мальки рыб"})
            ),
            "the editor was never asked to reconcile the fish gloss with the specific reviewed row"
        );
    }

    #[test]
    fn a_mental_ability_note_cannot_be_just_a_list_of_people() {
        let language = language("ru", "en");
        let (input, _) = metadata_example();
        let task = Task::from_input(input, &language).unwrap();
        let sample = calibrations(&task, &task.prompts.writer)
            .into_iter()
            .find(|sample| sample.card.term == "bright")
            .unwrap();
        assert_eq!(
            sample.parts.meanings[0].note, "быстро схватывает суть",
            "the calibration substitutes people's roles for the ability the learner needs to recognize"
        );
    }

    #[test]
    fn a_note_example_cannot_join_places_and_a_definition_after_the_same_head() {
        let language = language("ru", "en");
        let (input, _) = metadata_example();
        let task = Task::from_input(input, &language).unwrap();
        let bank = calibrations(&task, &task.prompts.writer)
            .into_iter()
            .find(|sample| sample.card.term == "bank")
            .unwrap();
        assert!(
            bank.parts.meanings[0].note == "для вкладов и переводов"
                && bank
                    .review_notes
                    .is_some_and(|notes| notes.contains("или учреждение")),
            "the pair example omitted the correction for a note mixing locations with a definition"
        );
    }

    #[test]
    fn a_copyeditor_cannot_miss_the_rendered_head_and_note_the_reader_sees() {
        let (input, parts) = metadata_example();
        let language = language("ru", "en");
        let task = Task::from_input(input, &language).unwrap();
        let prompt = task
            .prompt(&task.prompts.editor, Some((&parts, &language)), &[])
            .unwrap();
        assert!(
            prompt.contains(&super::super::compose(&parts, &language)),
            "the copyeditor only saw disconnected JSON fields instead of the actual reader text"
        );
    }

    #[test]
    fn quality_editor_cannot_preserve_misspellings_as_untouchable_learning_text() {
        let task = task(&language("es", "fa"));
        assert!(
            task.prompts
                .editor
                .contains("Proofread the learning phrases and examples for standard spelling")
                && task
                    .prompts
                    .writer
                    .contains("Do not invent a respelling to illustrate pronunciation"),
            "the editor was told to preserve a writer's spelling error without checking it"
        );
    }

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
                let both = format!("{}\n{}", task.prompts.writer, task.prompts.editor);
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
        (task.prompts.writer, task.prompts.editor)
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
                !task
                    .prompts
                    .writer
                    .contains("longer than one short line of text")
                    || task.prompts.writer.contains("longer than fourteen words")
                    || !task
                        .prompts
                        .editor
                        .contains("longer than one short line of text")
            })
            .collect::<Vec<_>>();
        assert!(
            wrong.is_empty(),
            "unspaced readers received a word-count instruction: {wrong:?}"
        );
    }
}
