//! Checked reader-language examples for explanation writing and repair.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unicode_script::UnicodeScript;
use unicode_segmentation::UnicodeSegmentation;

use super::prompt_examples::explanation_scripts;

/// Reader-local voice, meaning, contrast, and spelling examples.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExplanationExamples {
    pub(crate) voice: VoiceExamples,
    pub(crate) meaning: MeaningExamples,
    pub(crate) mistake: MistakeExamples,
    pub(crate) letters: Vec<String>,
}

/// Everyday wording and a verb-bearing rewrite in the reader's address form.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoiceExamples {
    pub(crate) active: Vec<String>,
    pub(crate) office: Vec<String>,
    pub(crate) vague: Vec<String>,
    pub(crate) verbless: RewriteExample,
}

/// A concrete alternative to an unsuitable phrase.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RewriteExample {
    pub(crate) bad: String,
    pub(crate) good: String,
}

/// Concrete meaning notes and place-led opening phrases.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeaningExamples {
    pub(crate) notes: Vec<NoteExample>,
    pub(crate) openers: Vec<String>,
}

/// One head with its definition-like note and concrete replacement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteExample {
    pub(crate) head: String,
    pub(crate) bad: String,
    pub(crate) good: String,
}

/// Local contrast and reason frames with deliberately unfilled task markers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MistakeExamples {
    pub(crate) contrast: String,
    pub(crate) reasons: Vec<String>,
    pub(crate) narrated: Vec<String>,
}

impl ExplanationExamples {
    /// Enumerate the complete block with field paths for validation diagnostics.
    pub(crate) fn strings(&self) -> Vec<(String, &str)> {
        let mut values = Vec::new();
        for (field, items) in [
            ("voice.active", &self.voice.active),
            ("voice.office", &self.voice.office),
            ("voice.vague", &self.voice.vague),
            ("meaning.openers", &self.meaning.openers),
            ("mistake.reasons", &self.mistake.reasons),
            ("mistake.narrated", &self.mistake.narrated),
            ("letters", &self.letters),
        ] {
            values.extend(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| (format!("{field}[{index}]"), item.as_str())),
            );
        }
        values.push(("voice.verbless.bad".into(), &self.voice.verbless.bad));
        values.push(("voice.verbless.good".into(), &self.voice.verbless.good));
        values.push(("mistake.contrast".into(), &self.mistake.contrast));
        for (index, note) in self.meaning.notes.iter().enumerate() {
            for (field, value) in [
                ("head", &note.head),
                ("bad", &note.bad),
                ("good", &note.good),
            ] {
                values.push((format!("meaning.notes[{index}].{field}"), value));
            }
        }
        values
    }

    /// Enumerate phrases whose reuse across languages would leak another reader's examples.
    pub(crate) fn phrases(&self) -> Vec<(String, &str)> {
        self.strings()
            .into_iter()
            .filter(|(field, _)| {
                field.starts_with("voice.verbless")
                    || field.starts_with("meaning.openers")
                    || (field.starts_with("meaning.notes") && !field.ends_with(".head"))
                    || field.starts_with("mistake.")
            })
            .collect()
    }

    /// Locate the first malformed field without repairing authored language data.
    pub(crate) fn fault(&self, code: &str, worded: bool) -> Option<String> {
        for (field, values, minimum, maximum) in [
            ("voice.active", &self.voice.active, 3, 3),
            ("voice.office", &self.voice.office, 5, 13),
            ("voice.vague", &self.voice.vague, 3, 6),
            ("meaning.openers", &self.meaning.openers, 3, 3),
            ("mistake.reasons", &self.mistake.reasons, 2, 2),
            ("mistake.narrated", &self.mistake.narrated, 1, 2),
        ] {
            if !(minimum..=maximum).contains(&values.len())
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return Some(field.into());
            }
        }
        if self.meaning.notes.len() != 2 {
            return Some("meaning.notes".into());
        }
        for (index, note) in self.meaning.notes.iter().enumerate() {
            if note.bad == note.good {
                return Some(format!("meaning.notes[{index}]"));
            }
        }
        if self.voice.verbless.bad == self.voice.verbless.good {
            return Some("voice.verbless".into());
        }
        for (field, value) in self.strings() {
            if value.trim().is_empty()
                || !scripted(value, code)
                || (!worded && value.chars().any(char::is_whitespace))
            {
                return Some(field);
            }
            let marks = if field.starts_with("voice.verbless") {
                Some((1, 0))
            } else if field == "mistake.contrast" {
                Some((1, 1))
            } else if field == "mistake.reasons[0]" {
                Some((1, 0))
            } else if field == "mistake.reasons[1]" {
                Some((0, 1))
            } else if field.starts_with("meaning.") {
                Some((0, 0))
            } else {
                None
            };
            if let Some((one, two)) = marks
                && (value.matches("⟦1⟧").count() != one
                    || value.matches("⟦2⟧").count() != two
                    || value
                        .replace("⟦1⟧", "")
                        .replace("⟦2⟧", "")
                        .contains(['⟦', '⟧']))
            {
                return Some(field);
            }
            if field.starts_with("mistake.reasons") && !angle(value) {
                return Some(field);
            }
            if field.starts_with("letters")
                && (value.chars().count() != 1 || !value.chars().all(char::is_alphabetic))
            {
                return Some(field);
            }
        }
        None
    }
}

/// Check native letters and marks, requiring a native base for generic marks.
pub(super) fn scripted(value: &str, code: &str) -> bool {
    native_scripted(value, &explanation_scripts(code))
}

/// Check learning fragments against their native scripts, including attached marks.
pub(crate) fn native_scripted(value: &str, scripts: &[unicode_script::Script]) -> bool {
    characters(value, scripts, false)
}

/// Find a word that splices reader letters into learning text, preserving the reviewed term.
pub(crate) fn mixed_sentence_word<'a>(
    value: &'a str,
    term: &str,
    learning: &str,
    reader: &str,
) -> Option<&'a str> {
    use unicode_script::Script;
    let mut native = explanation_scripts(learning);
    if native.contains(&Script::Han) {
        native.push(Script::Latin);
    }
    let foreign: Vec<_> = explanation_scripts(reader)
        .into_iter()
        .filter(|s| !native.contains(s))
        .collect();
    if foreign.is_empty() {
        return None;
    }
    value.unicode_words().find(|word| {
        let scripts = || {
            word.chars()
                .filter(|ch| ch.is_alphabetic())
                .map(|ch| ch.script())
        };
        scripts().any(|script| native.contains(&script))
            && scripts().any(|script| foreign.contains(&script))
            && !term
                .unicode_words()
                .any(|approved| approved.to_lowercase() == word.to_lowercase())
    })
}

/// Diagnose terminal punctuation in a complete learning-language example.
pub(crate) fn example_punctuation(
    value: &str,
    scripts: &[unicode_script::Script],
) -> Option<&'static str> {
    (scripts.contains(&unicode_script::Script::Armenian) && value.trim_end().ends_with([':', '?']))
        .then_some("end the Armenian sentence with its native full stop (։); for a direct question, place the Armenian question mark (՞) on the stressed vowel")
}

/// Check rendered instructions while allowing their ASCII prose outside native examples.
#[cfg(test)]
pub(crate) fn prompt_scripted(value: &str, code: &str) -> bool {
    characters(value, &explanation_scripts(code), true)
}

fn characters(value: &str, scripts: &[unicode_script::Script], ascii: bool) -> bool {
    let pattern = regex::Regex::new(r"(?s)(\p{L})|(\p{M})|(.)")
        .expect("invariant: script classification pattern must compile");
    let mut base = false;
    for capture in pattern.captures_iter(value) {
        if capture.get(3).is_some() {
            base = false;
            continue;
        }
        let character = capture[0]
            .chars()
            .next()
            .expect("invariant: script classification must consume one character");
        if capture.get(2).is_some() && !base {
            return false;
        }
        let extension = character.script_extension();
        if extension.is_common() || extension.is_inherited() {
            if !base {
                return false;
            }
        } else {
            let native = scripts
                .iter()
                .any(|script| extension.contains_script(*script));
            if !(native || ascii && character.is_ascii()) {
                return false;
            }
            if capture.get(1).is_some() {
                base = native;
            }
        }
    }
    true
}

fn angle(value: &str) -> bool {
    value.matches('<').count() == 1
        && value.matches('>').count() == 1
        && value
            .split_once('<')
            .and_then(|(_, rest)| rest.split_once('>'))
            .is_some_and(|(inside, _)| !inside.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::{ExplanationExamples, prompt_scripted, scripted};
    use serde_json::{Value, json};

    #[test]
    fn sentence_spelling_cannot_reject_loanwords_native_script_combinations_or_the_reviewed_term() {
        let faults = [
            ("В этом стакане вода из IKEA.", "вода", "ru", "hi"),
            ("PDFを読みました。", "資料", "ja", "en"),
            ("K팝을 좋아합니다.", "음악", "ko", "en"),
            ("Das Logo sagt КиївCity.", "КиївCity", "de", "uk"),
        ]
        .map(|(text, term, learning, reader)| {
            super::mixed_sentence_word(text, term, learning, reader)
        });
        assert_eq!(
            faults, [None; 4],
            "an intact word, a native script combination or an approved spelling was refused"
        );
    }

    fn block(code: &str) -> Value {
        serde_json::from_str::<Value>(include_str!("../../assets/prompt_examples.json")).unwrap()
            [code]["card"]["explanation"]
            .clone()
    }

    #[test]
    fn native_diacritics_cannot_be_rejected_as_foreign_scripts() {
        let rejected = [
            ("ar", "يُقال"),
            ("ar", "شيئًا"),
            ("fa", "دربارهٔ"),
            ("fa", "بعداً"),
            ("ru", "а\u{301}"),
            ("en", "e\u{301}"),
        ]
        .into_iter()
        .filter(|(code, text)| !scripted(text, code))
        .collect::<Vec<_>>();
        assert!(
            rejected.is_empty(),
            "native diacritics were rejected as foreign scripts: {rejected:?}"
        );
    }

    #[test]
    fn foreign_letters_and_marks_cannot_borrow_a_native_base() {
        let accepted = [
            ("ar", "قالя"),
            ("fa", "گفت猫"),
            ("ar", "ق\u{5b0}"),
            ("ar", "ق\u{591}"),
            ("fa", "گ\u{5b0}"),
            ("ru", "а\u{64f}"),
            ("en", "e\u{64f}"),
        ]
        .into_iter()
        .filter(|(code, text)| scripted(text, code))
        .collect::<Vec<_>>();
        assert!(
            accepted.is_empty(),
            "a foreign letter or mark borrowed a native base: {accepted:?}"
        );
    }

    #[test]
    fn generic_marks_cannot_survive_without_a_native_letter_base() {
        let accepted = [
            ("ar", "\u{301}"),
            ("fa", "\u{301}گ"),
            ("en", "\u{301}"),
            ("ru", "а \u{301}"),
            ("en", "e.\u{301}"),
            ("ru", "7\u{301}"),
            ("ar", "\u{64f}"),
            ("fa", "7\u{654}"),
        ]
        .into_iter()
        .filter(|(code, text)| scripted(text, code))
        .collect::<Vec<_>>();
        assert!(
            accepted.is_empty(),
            "a generic combining mark survived without a native base: {accepted:?}"
        );
    }

    #[test]
    fn prompt_marks_cannot_use_ascii_prose_as_a_foreign_base() {
        let accepted = [("ar", "a\u{301}"), ("fa", "a\u{301}"), ("ru", "e\u{301}")]
            .into_iter()
            .filter(|(code, text)| prompt_scripted(text, code))
            .collect::<Vec<_>>();
        assert!(
            accepted.is_empty(),
            "an ASCII instruction letter lent its script to a foreign mark: {accepted:?}"
        );
    }

    #[test]
    fn checked_seed_blocks_cannot_violate_the_language_policy() {
        assert!(
            ["ru", "en"].into_iter().all(|code| {
                serde_json::from_value::<ExplanationExamples>(block(code))
                    .unwrap()
                    .fault(code, true)
                    .is_none()
            }),
            "a checked seed violates its explanation policy"
        );
    }

    #[test]
    fn malformed_fields_cannot_pass_the_explanation_policy() {
        let cases = [
            ("/voice/active", json!(["say", "hear"])),
            ("/voice/active", json!(["say", "hear", "tell", "speak"])),
            ("/voice/active", json!(["say", "say", "hear"])),
            ("/voice/office", json!(["one", "two", "three", "four"])),
            ("/voice/office", json!(vec!["word"; 14])),
            ("/voice/vague", json!(["thing", "object"])),
            ("/voice/vague", json!(vec!["word"; 7])),
            ("/voice/verbless/bad", json!("No marker")),
            ("/voice/verbless/good", json!("Say ⟦1⟧ and ⟦1⟧")),
            ("/voice/verbless/bad", json!("Say ⟦1⟧ instead of ⟦2⟧")),
            (
                "/voice/verbless/bad",
                block("en")["voice"]["verbless"]["good"].clone(),
            ),
            ("/meaning/notes", json!([])),
            ("/meaning/notes/0/head", json!("")),
            (
                "/meaning/notes/0/bad",
                block("en")["meaning"]["notes"][0]["good"].clone(),
            ),
            ("/meaning/notes/1/good", json!("A concrete ⟦1⟧")),
            ("/meaning/openers", json!(["At home", "At home", "At work"])),
            ("/meaning/openers/0", json!("At ⟦2⟧")),
            ("/mistake/contrast", json!("Say ⟦1⟧")),
            ("/mistake/contrast", json!("Say ⟦1⟧ and ⟦2⟧ and ⟦3⟧")),
            ("/mistake/reasons", json!(["⟦1⟧ shows <a reason>."])),
            ("/mistake/reasons/0", json!("⟦2⟧ shows <a reason>.")),
            ("/mistake/reasons/1", json!("⟦2⟧ means nothing.")),
            ("/mistake/reasons/0", json!("⟦1⟧ shows <>.")),
            ("/mistake/reasons/0", json!("⟦1⟧ shows <this> and <that>.")),
            ("/mistake/reasons/0", json!("⟦1⟧ shows >a reason<.")),
            ("/mistake/narrated", json!([])),
            ("/mistake/narrated", json!(["first", "second", "third"])),
            ("/voice/office/0", json!("слово")),
            ("/letters", json!(["ab"])),
            ("/letters", json!(["7"])),
            ("/letters", json!(["ё"])),
        ];
        let accepted = cases
            .into_iter()
            .filter_map(|(path, value)| {
                let mut sample = block("en");
                *sample.pointer_mut(path).unwrap() = value;
                serde_json::from_value::<ExplanationExamples>(sample)
                    .unwrap()
                    .fault("en", true)
                    .is_none()
                    .then_some(path)
            })
            .collect::<Vec<_>>();
        assert!(
            accepted.is_empty(),
            "malformed explanation fields were accepted: {accepted:?}"
        );
    }

    #[test]
    fn unspaced_examples_cannot_gain_a_space() {
        let sample: ExplanationExamples = serde_json::from_value(block("en")).unwrap();
        assert!(
            sample.fault("en", false).is_some(),
            "an unspaced explanation accepted spaces between words"
        );
    }

    #[test]
    fn explanation_blocks_cannot_omit_required_fields() {
        let accepted = ["voice", "meaning", "mistake", "letters"]
            .into_iter()
            .filter(|field| {
                let mut sample = block("en");
                sample.as_object_mut().unwrap().remove(*field);
                serde_json::from_value::<ExplanationExamples>(sample).is_ok()
            })
            .collect::<Vec<_>>();
        assert!(
            accepted.is_empty(),
            "required explanation fields were optional: {accepted:?}"
        );
    }

    #[test]
    fn every_authored_string_cannot_be_empty_or_change_its_script() {
        let rejected = ["ru", "en"]
            .into_iter()
            .flat_map(|code| {
                let sample: ExplanationExamples = serde_json::from_value(block(code)).unwrap();
                sample
                    .strings()
                    .into_iter()
                    .map(|(field, _)| field)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .flat_map(move |field| {
                        ["", "猫"].into_iter().filter_map(move |replacement| {
                            let path =
                                format!("/{}", field.replace(['.', '['], "/").replace(']', ""));
                            let mut value = block(code);
                            *value.pointer_mut(&path).unwrap() = json!(replacement);
                            serde_json::from_value::<ExplanationExamples>(value)
                                .unwrap()
                                .fault(code, true)
                                .is_none()
                                .then_some(format!("{code}.{field}"))
                        })
                    })
            })
            .collect::<Vec<_>>();
        assert!(
            rejected.is_empty(),
            "an authored string accepted empty or foreign-script content: {rejected:?}"
        );
    }
}
