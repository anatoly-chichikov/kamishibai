use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MODEL_NAME: &str = "Kamishibai Vocabulary Model";

/// Derive a deterministic 31-bit identifier from one name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StableId {
    name: String,
}

impl StableId {
    /// Create one stable identifier source.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Return the deterministic 31-bit integer identifier.
    pub fn value(&self) -> i64 {
        let digest = Sha256::digest(self.name.as_bytes());
        let mut hex = String::new();
        for item in digest.iter().take(4) {
            hex.push_str(format!("{item:02x}").as_str());
        }
        i64::from(u32::from_str_radix(hex.as_str(), 16).expect("hex digest must parse"))
            % (1_i64 << 31)
    }
}

/// One Anki card template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    pub afmt: String,
    pub bafmt: String,
    pub bfont: String,
    pub bqfmt: String,
    pub bsize: i64,
    pub did: Option<i64>,
    pub name: String,
    pub ord: i64,
    pub qfmt: String,
}

/// One card model contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    pub fields: Vec<String>,
    pub id: i64,
    pub name: String,
    pub template: Template,
}

impl Model {
    /// Return the serialized Anki model representation.
    pub(crate) fn json(&self, timestamp: i64) -> Value {
        json!({
            "css": "",
            "did": Value::Null,
            "flds": self.fields.iter().enumerate().map(|(index, name)| {
                json!({
                    "font": "Liberation Sans",
                    "media": [],
                    "name": name,
                    "ord": index,
                    "rtl": false,
                    "size": 20,
                    "sticky": false,
                })
            }).collect::<Vec<_>>(),
            "id": self.id.to_string(),
            "latexPost": "\\end{document}",
            "latexPre": "\\documentclass[12pt]{article}\n\\special{papersize=3in,5in}\n\\usepackage[utf8]{inputenc}\n\\usepackage{amssymb,amsmath}\n\\pagestyle{empty}\n\\setlength{\\parindent}{0in}\n\\begin{document}\n",
            "latexsvg": false,
            "mod": timestamp,
            "name": self.name,
            "req": [[0, "all", [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]]],
            "sortf": 0,
            "tags": [],
            "tmpls": [json!({
                "afmt": self.template.afmt,
                "bafmt": self.template.bafmt,
                "bfont": self.template.bfont,
                "bqfmt": self.template.bqfmt,
                "bsize": self.template.bsize,
                "did": self.template.did,
                "name": self.template.name,
                "ord": self.template.ord,
                "qfmt": self.template.qfmt,
            })],
            "type": 0,
            "usn": -1,
            "vers": [],
        })
    }
}

/// Vocabulary model builder with the frozen 11-field contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardModel {
    identifier: i64,
    name: String,
}

impl CardModel {
    /// Create one frozen vocabulary card model builder.
    pub fn new() -> Self {
        Self {
            identifier: StableId::new(MODEL_NAME).value(),
            name: String::from(MODEL_NAME),
        }
    }

    /// Return the frozen vocabulary model contract.
    pub fn model(&self) -> Model {
        Model {
            fields: vec![
                String::from("SourceSentence"),
                String::from("Term"),
                String::from("Pronunciation"),
                String::from("Meaning"),
                String::from("TargetSentence"),
                String::from("Importance"),
                String::from("Audio"),
                String::from("Illustration"),
                String::from("Hint"),
                String::from("Context"),
                String::from("PronunciationAll"),
            ],
            id: self.identifier,
            name: self.name.clone(),
            template: Template {
                afmt: String::from(
                    "{{FrontSide}}<hr id=\"answer\"><div style=\"max-width: 600px; margin: 0 auto; text-align: center; padding: 0 20px;\">{{Audio}}<div style=\"font-size: 22px; font-weight: bold; margin: 20px 0 4px 0;\">{{TargetSentence}}</div>{{#PronunciationAll}}<div style=\"font-size: 13px; color: #aaa; margin-top: 4px;\">{{PronunciationAll}}</div>{{/PronunciationAll}}<div style=\"font-size: 17px; margin-top: 15px;\"><strong style=\"color: #ddd;\">{{Term}}</strong> <span style=\"color: #aaa;\">{{Pronunciation}}</span></div><div style=\"font-size: 15px; color: #bbb; margin-top: 3px;\">{{Meaning}}</div><div style=\"font-size: 13px; color: #999; margin-top: 8px;\">{{Importance}}/10</div>{{#Context}}<div style=\"font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px; text-align: left;\">{{Context}}</div>{{/Context}}</div>",
                ),
                bafmt: String::new(),
                bfont: String::new(),
                bqfmt: String::new(),
                bsize: 0,
                did: None,
                name: String::from("Card 1"),
                ord: 0,
                qfmt: String::from(
                    "<div style=\"max-width: 600px; margin: 0 auto; text-align: center; padding: 20px;\">{{Illustration}}<div style=\"font-size: 20px; margin-top: 15px;\">{{SourceSentence}}</div>{{#Hint}}<div style=\"font-size: 14px; color: #888; margin-top: 8px; font-style: italic;\">{{Hint}}</div>{{/Hint}}</div>",
                ),
            },
        }
    }
}

impl Default for CardModel {
    /// Return the frozen vocabulary card model builder.
    fn default() -> Self {
        Self::new()
    }
}
