//! Typed capture procedure for TUI screenshots and the demo GIF, checked against the tapes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The recording procedure; load with `try_from_document` and inspect it against the tapes on disk.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recording {
    version: u32,
    tapes: Vec<Tape>,
    demo: Demo,
    steps: Vec<Step>,
    sections: Vec<Section>,
    slices: Vec<Slice>,
    pitfalls: Vec<Pitfall>,
}

/// One VHS tape: what it drives, how it is framed, and why it exists.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tape {
    file: PathBuf,
    drives: Drive,
    purpose: String,
    geometry: String,
}

/// What a tape launches: prepared gallery scenes by stable ID, or the real binary against Gemini.
#[derive(Deserialize, PartialEq, Eq)]
enum Drive {
    Scenes,
    Binary,
}

/// The live demo input and the story the live tape records.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Demo {
    known: String,
    learning: String,
    words: Vec<String>,
    story: Vec<String>,
}

/// One operator step: the commands to run and what the operator must know while running them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    title: String,
    commands: Vec<String>,
    explanation: Vec<String>,
}

/// One kind of GIF section: how to recognise it and how to sample it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    kind: String,
    signal: String,
    sampling: String,
}

/// One section of the standard demo flow, classified by a declared section kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slice {
    label: String,
    window: String,
    kind: String,
    output: String,
}

/// A recording mistake and the damage it does.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pitfall {
    rule: String,
    reason: String,
}

/// A procedure defect found against the files on disk.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Fault {
    Version(u32),
    Empty(String),
    MissingTape(String),
    UnlistedTape(String),
    MisdrivenTape(String),
    UntypedWord(String),
    UnknownSection(String),
    UnknownFile(String),
}

impl Recording {
    /// Decode the authored RON document without accepting unknown fields.
    pub(super) fn try_from_document(document: &str) -> Result<Self> {
        ron::from_str(document).context("Cannot decode TUI recording procedure")
    }

    /// Inspect the procedure against the tapes and helper files under `root`.
    #[must_use]
    pub(super) fn inspect(&self, root: &Path) -> Vec<Fault> {
        let mut faults = Vec::new();
        if self.version != 1 {
            faults.push(Fault::Version(self.version));
        }
        let directory = root.join("docs/tui-states");
        let listed = self
            .tapes
            .iter()
            .map(|tape| tape.file.clone())
            .collect::<HashSet<_>>();
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "tape")
                    && let Ok(relative) = path.strip_prefix(root)
                    && !listed.contains(relative)
                {
                    faults.push(Fault::UnlistedTape(relative.display().to_string()));
                }
            }
        }
        let mut live = String::new();
        for tape in &self.tapes {
            text(&tape.purpose, "tape purpose", &mut faults);
            text(&tape.geometry, "tape geometry", &mut faults);
            let Ok(document) = fs::read_to_string(root.join(&tape.file)) else {
                faults.push(Fault::MissingTape(tape.file.display().to_string()));
                continue;
            };
            let scenes = document.contains("tui_states --state ");
            if scenes != (tape.drives == Drive::Scenes) {
                faults.push(Fault::MisdrivenTape(tape.file.display().to_string()));
            }
            if tape.drives == Drive::Binary {
                live.push_str(&document);
            }
        }
        text(&self.demo.known, "demo known language", &mut faults);
        text(&self.demo.learning, "demo learning language", &mut faults);
        if self.demo.words.is_empty() {
            faults.push(Fault::Empty(String::from("demo words")));
        }
        for word in &self.demo.words {
            if !live.contains(&format!("Type \"{word}\"")) {
                faults.push(Fault::UntypedWord(word.clone()));
            }
        }
        for line in &self.demo.story {
            text(line, "demo story", &mut faults);
        }
        if self.steps.is_empty() {
            faults.push(Fault::Empty(String::from("steps")));
        }
        for step in &self.steps {
            text(&step.title, "step title", &mut faults);
            for line in step.commands.iter().chain(&step.explanation) {
                text(line, &step.title, &mut faults);
            }
            for command in &step.commands {
                for word in command.split_whitespace() {
                    let named = word.strip_prefix("./").unwrap_or(word);
                    let local = named.ends_with(".tape") || named == "encode.sh";
                    if local && !directory.join(named).is_file() {
                        faults.push(Fault::UnknownFile(format!("{}: {word}", step.title)));
                    }
                }
            }
        }
        let kinds = self
            .sections
            .iter()
            .map(|section| section.kind.as_str())
            .collect::<HashSet<_>>();
        for section in &self.sections {
            text(&section.kind, "section kind", &mut faults);
            text(&section.signal, &section.kind, &mut faults);
            text(&section.sampling, &section.kind, &mut faults);
        }
        for slice in &self.slices {
            text(&slice.label, "slice label", &mut faults);
            text(&slice.window, &slice.label, &mut faults);
            text(&slice.output, &slice.label, &mut faults);
            if !kinds.contains(slice.kind.as_str()) {
                faults.push(Fault::UnknownSection(format!(
                    "{}: {}",
                    slice.label, slice.kind
                )));
            }
        }
        if self.pitfalls.is_empty() {
            faults.push(Fault::Empty(String::from("pitfalls")));
        }
        for pitfall in &self.pitfalls {
            text(&pitfall.rule, "pitfall rule", &mut faults);
            text(&pitfall.reason, &pitfall.rule, &mut faults);
        }
        faults
    }
}

fn text(value: &str, owner: &str, faults: &mut Vec<Fault>) {
    if value.trim().is_empty() {
        faults.push(Fault::Empty(owner.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{Demo, Drive, Fault, Pitfall, Recording, Section, Slice, Step, Tape};

    fn recording(tapes: Vec<Tape>, words: Vec<String>, slices: Vec<Slice>) -> Recording {
        Recording {
            version: 1,
            tapes,
            demo: Demo {
                known: String::from("EN"),
                learning: String::from("FR"),
                words,
                story: vec![String::from("Type the words and build one batch")],
            },
            steps: vec![Step {
                title: String::from("Record"),
                commands: vec![String::from("vhs states.tape")],
                explanation: vec![String::from("Synthetic scenes need no provider")],
            }],
            sections: vec![Section {
                kind: String::from("workflow"),
                signal: String::from("New content lands"),
                sampling: String::from("25 fps on the natural window"),
            }],
            slices,
            pitfalls: vec![Pitfall {
                rule: String::from("Never resample a spinner"),
                reason: String::from("It spins faster than real time"),
            }],
        }
    }

    fn tape(file: &str, drives: Drive) -> Tape {
        Tape {
            file: Path::new("docs/tui-states").join(file),
            drives,
            purpose: String::from("Capture scenes"),
            geometry: String::from("2250x1688 px"),
        }
    }

    fn root(files: &[(&str, &str)]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let tapes = directory.path().join("docs/tui-states");
        fs::create_dir_all(&tapes).expect("invariant: tape directory must be writable");
        for (name, body) in files {
            fs::write(tapes.join(name), body).expect("invariant: tape fixture must be writable");
        }
        directory
    }

    #[test]
    fn recording_cannot_ignore_a_tape_on_disk() {
        let directory = root(&[
            ("states.tape", "Type \"tui_states --state cards.closed\"\n"),
            ("capture.tape", "Type \"canard\"\n"),
        ]);
        assert!(
            recording(
                vec![tape("states.tape", Drive::Scenes)],
                Vec::new(),
                Vec::new()
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Fault::UnlistedTape(_))),
            "an unlisted tape escaped the recording procedure"
        );
    }

    #[test]
    fn demo_cannot_name_a_word_the_live_tape_never_types() {
        let directory = root(&[("capture.tape", "Type \"canard\"\n")]);
        assert!(
            recording(
                vec![tape("capture.tape", Drive::Binary)],
                vec![String::from("chouette")],
                Vec::new()
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Fault::UntypedWord(_))),
            "the demo listed a word the live tape never types"
        );
    }

    #[test]
    fn scene_tape_cannot_be_declared_as_a_live_run() {
        let directory = root(&[("states.tape", "Type \"tui_states --state cards.closed\"\n")]);
        assert!(
            recording(
                vec![tape("states.tape", Drive::Binary)],
                Vec::new(),
                Vec::new()
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Fault::MisdrivenTape(_))),
            "a gallery tape was accepted as the live binary run"
        );
    }

    #[test]
    fn slice_cannot_use_an_undeclared_section_kind() {
        let directory = root(&[("states.tape", "Type \"tui_states --state cards.closed\"\n")]);
        assert!(
            recording(
                vec![tape("states.tape", Drive::Scenes)],
                Vec::new(),
                vec![Slice {
                    label: String::from("A typing"),
                    window: String::from("0 s to first busy"),
                    kind: String::from("montage"),
                    output: String::from("1.5 s"),
                }]
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Fault::UnknownSection(_))),
            "a slice used a section kind the procedure never defines"
        );
    }
}
