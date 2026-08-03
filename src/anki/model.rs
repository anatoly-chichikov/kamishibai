use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MODEL_NAME: &str = "Kamishibai Vocabulary Model";
const CARD_STYLE: &str = r#".card {
  --kamishibai-hint: #666;
  --kamishibai-phonetics: #5f6368;
  --kamishibai-term: #333;
  --kamishibai-meaning: #555;
  --kamishibai-importance: #666;
  --kamishibai-context: #555;
  --kamishibai-context-background: rgba(0, 0, 0, 0.045);
}
.card.nightMode,
.card.night_mode {
  --kamishibai-hint: #888;
  --kamishibai-phonetics: #aaa;
  --kamishibai-term: #ddd;
  --kamishibai-meaning: #bbb;
  --kamishibai-importance: #999;
  --kamishibai-context: #aaa;
  --kamishibai-context-background: rgba(255, 255, 255, 0.05);
}
.card:not(.nightMode):not(.night_mode) img {
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.14);
}
.kamishibai-hint {
  color: var(--kamishibai-hint);
}
.kamishibai-phonetics {
  color: var(--kamishibai-phonetics);
}
.kamishibai-term {
  color: var(--kamishibai-term);
}
.kamishibai-meaning {
  color: var(--kamishibai-meaning);
}
.kamishibai-importance {
  color: var(--kamishibai-importance);
}
.kamishibai-context {
  color: var(--kamishibai-context);
  background-color: var(--kamishibai-context-background);
}"#;

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
            "css": CARD_STYLE,
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
                    "{{FrontSide}}<hr id=\"answer\"><div style=\"max-width: 600px; margin: 0 auto; text-align: center; padding: 0 20px;\">{{Audio}}<div style=\"font-size: 22px; font-weight: bold; margin: 20px 0 4px 0;\">{{TargetSentence}}</div>{{#PronunciationAll}}<div class=\"kamishibai-phonetics\" style=\"font-size: 13px; margin-top: 4px;\">{{PronunciationAll}}</div>{{/PronunciationAll}}<div style=\"font-size: 17px; margin-top: 15px;\"><strong class=\"kamishibai-term\">{{Term}}</strong> <span class=\"kamishibai-phonetics\">{{Pronunciation}}</span></div><div class=\"kamishibai-meaning\" style=\"font-size: 15px; margin-top: 3px;\">{{Meaning}}</div><div class=\"kamishibai-importance\" style=\"font-size: 13px; margin-top: 8px;\">{{Importance}}/10</div>{{#Context}}<div class=\"kamishibai-context\" style=\"font-size: 14px; margin-top: 12px; padding: 10px; border-radius: 5px; text-align: left;\">{{Context}}</div>{{/Context}}</div>",
                ),
                bafmt: String::new(),
                bfont: String::new(),
                bqfmt: String::new(),
                bsize: 0,
                did: None,
                name: String::from("Card 1"),
                ord: 0,
                qfmt: String::from(
                    "<div style=\"max-width: 600px; margin: 0 auto; text-align: center; padding: 20px;\">{{Illustration}}<div style=\"font-size: 20px; margin-top: 15px;\">{{SourceSentence}}</div>{{#Hint}}<div class=\"kamishibai-hint\" style=\"font-size: 14px; margin-top: 8px; font-style: italic;\">{{Hint}}</div>{{/Hint}}</div>",
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

#[cfg(test)]
mod tests {
    use super::{CARD_STYLE, CardModel};

    /// The theme stylesheet keeps every published dark-mode color unchanged.
    #[test]
    fn the_theme_stylesheet_keeps_every_published_dark_mode_color_unchanged() {
        let dark = ".card.nightMode,\n.card.night_mode {\n  --kamishibai-hint: #888;\n  --kamishibai-phonetics: #aaa;\n  --kamishibai-term: #ddd;\n  --kamishibai-meaning: #bbb;\n  --kamishibai-importance: #999;\n  --kamishibai-context: #aaa;\n  --kamishibai-context-background: rgba(255, 255, 255, 0.05);\n}";
        assert!(
            CARD_STYLE.contains(dark),
            "the theme stylesheet no longer preserves the published dark-mode palette"
        );
    }

    /// The illustration outline separates its white matte only from a light card.
    #[test]
    fn the_illustration_outline_only_separates_its_white_matte_from_a_light_card() {
        let light = ".card:not(.nightMode):not(.night_mode) img {\n  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.14);\n}";
        assert!(
            CARD_STYLE.contains(light),
            "the illustration outline no longer belongs exclusively to the light theme"
        );
    }

    /// Theme-sensitive template content delegates every color to the stylesheet.
    #[test]
    fn theme_sensitive_template_content_delegates_every_color_to_the_stylesheet() {
        let template = CardModel::new().model().template;
        assert!(
            !template.afmt.contains("color:")
                && !template.afmt.contains("background-color:")
                && !template.qfmt.contains("color:")
                && template.afmt.contains("kamishibai-context")
                && template.qfmt.contains("kamishibai-hint"),
            "theme-sensitive template content still bypasses the stylesheet"
        );
    }
}
