use crate::markdown::{parse_markdown, to_html};
use crate::vocabulary::VocabularyEntry;

use super::Model;

const BASE91: [char; 91] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
    'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4',
    '5', '6', '7', '8', '9', '!', '#', '$', '%', '&', '(', ')', '*', '+', ',', '-', '.', '/', ':',
    ';', '<', '=', '>', '?', '@', '[', ']', '^', '_', '`', '{', '|', '}', '~',
];

/// Assemble one note from one canonical entry.
pub trait NoteFormat {
    /// Return one formatted note for the entry and relative media tags.
    fn note(&self, entry: &VocabularyEntry, audio: &str, image: &str) -> Note;
    /// Return the model used for note serialization.
    fn model(&self) -> &Model;
}

/// Wrap one phonetic value in slash notation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transcription {
    value: String,
}

impl Transcription {
    /// Create one transcription formatter.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Return the slash-wrapped transcription or an empty string.
    pub fn formatted(&self) -> String {
        let stripped = self.value.trim_matches('/');
        if stripped.is_empty() {
            return String::new();
        }
        format!("/{stripped}/")
    }
}

/// Replace newlines with HTML line breaks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlLineBreaks {
    value: String,
}

impl HtmlLineBreaks {
    /// Create one HTML line-break formatter.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Return the value with newlines converted to br tags.
    pub fn formatted(&self) -> String {
        if self.value.is_empty() {
            return String::new();
        }
        self.value.replace('\n', "<br>")
    }
}

/// One formatted Anki note payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub fields: Vec<String>,
    pub guid: String,
    pub sort_field: String,
}

/// Assemble vocabulary notes from canonical entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocabularyNote {
    model: Model,
}

impl VocabularyNote {
    /// Create one vocabulary note formatter.
    pub fn new(model: Model) -> Self {
        Self { model }
    }
}

impl NoteFormat for VocabularyNote {
    /// Return one formatted note for the entry and relative media tags.
    fn note(&self, entry: &VocabularyEntry, audio: &str, image: &str) -> Note {
        let source = entry.source.sentence.as_str().replace(
            entry.source.highlight.as_str(),
            format!(
                "<strong><em>{}</em></strong>",
                entry.source.highlight.as_str()
            )
            .as_str(),
        );
        let fields = vec![
            source,
            entry.term.as_str().to_lowercase(),
            Transcription::new(entry.pronunciation.as_str()).formatted(),
            String::from(entry.meaning.as_str()),
            HtmlLineBreaks::new(entry.target.sentence.as_str()).formatted(),
            entry.importance.to_string(),
            String::from(audio),
            String::from(image),
            String::from(entry.source.hint.as_str()),
            to_html(&parse_markdown(entry.source.context.as_str())),
            Transcription::new(entry.transcription.as_str()).formatted(),
        ];
        Note {
            guid: guid(fields.as_slice()),
            sort_field: fields[0].clone(),
            fields,
        }
    }

    /// Return the model used for note serialization.
    fn model(&self) -> &Model {
        &self.model
    }
}

/// Return the deterministic Anki guid for one field set.
pub(crate) fn guid(fields: &[String]) -> String {
    use sha2::{Digest, Sha256};

    let value = fields.join("__");
    let digest = Sha256::digest(value.as_bytes());
    let mut number = 0u64;
    for item in digest.iter().take(8) {
        number <<= 8;
        number += u64::from(*item);
    }
    let mut value = Vec::new();
    while number > 0 {
        value.push(BASE91[(number % BASE91.len() as u64) as usize]);
        number /= BASE91.len() as u64;
    }
    value.iter().rev().collect()
}
