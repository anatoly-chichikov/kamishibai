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
        let examples =
            crate::languages::explanation_pair(&language.names.reader, &language.names.learning)?;
        let stamp = Sha256::digest(format!("{writer}\n{editor}{examples}"))
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
        language: &Language,
    ) -> Result<String> {
        let examples =
            crate::languages::explanation_pair(&language.names.reader, &language.names.learning)?;
        let mut prompt = format!(
            "{instructions}{examples}\n\nCard input:\n{}",
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
    let Some(mut current) = ask(writer, task.prompt(&task.writer, None, &[], language)?)? else {
        return Ok(None);
    };
    let (fails, _) = judge(&task.input.card, &current, language).into_parts();
    if !fails.is_empty()
        && let Some(fixed) = ask(
            writer,
            task.prompt(&task.writer, Some(&current), &fails, language)?,
        )?
    {
        current = fixed;
    }
    if let Some(edited) = ask(
        writer,
        task.prompt(&task.editor, Some(&current), &[], language)?,
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
                task.prompt(&task.editor, Some(&source), &fails, language)?,
            )? {
                if !intact && !preserves(&original, &fixed, language) {
                    return Ok(None);
                }
                current = fixed;
            } else if !intact {
                return Ok(None);
            }
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
            editor: task.editor.clone(),
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
        .filter(|rule| !task.writer.contains(rule))
        .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "writer lacks safeguards for observed semantic failures: {missing:?}"
        );
    }

    #[test]
    fn quality_editor_cannot_preserve_misspellings_as_untouchable_learning_text() {
        let task = task(&language("es", "fa"));
        assert!(
            task.editor
                .contains("Proofread the learning phrases and examples for standard spelling")
                && task
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
