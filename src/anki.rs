//! Anki note formatting and APKG writing.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::input::NormalizedEntry;

const BASE91: [char; 91] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
    'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4',
    '5', '6', '7', '8', '9', '!', '#', '$', '%', '&', '(', ')', '*', '+', ',', '-', '.', '/', ':',
    ';', '<', '=', '>', '?', '@', '[', ']', '^', '_', '`', '{', '|', '}', '~',
];
const MODEL_NAME: &str = "Kamishibai Vocabulary Model";
const SCHEMA: &str = "
CREATE TABLE col (
    id              integer primary key,
    crt             integer not null,
    mod             integer not null,
    scm             integer not null,
    ver             integer not null,
    dty             integer not null,
    usn             integer not null,
    ls              integer not null,
    conf            text not null,
    models          text not null,
    decks           text not null,
    dconf           text not null,
    tags            text not null
);
CREATE TABLE notes (
    id              integer primary key,
    guid            text not null,
    mid             integer not null,
    mod             integer not null,
    usn             integer not null,
    tags            text not null,
    flds            text not null,
    sfld            integer not null,
    csum            integer not null,
    flags           integer not null,
    data            text not null
);
CREATE TABLE cards (
    id              integer primary key,
    nid             integer not null,
    did             integer not null,
    ord             integer not null,
    mod             integer not null,
    usn             integer not null,
    type            integer not null,
    queue           integer not null,
    due             integer not null,
    ivl             integer not null,
    factor          integer not null,
    reps            integer not null,
    lapses          integer not null,
    left            integer not null,
    odue            integer not null,
    odid            integer not null,
    flags           integer not null,
    data            text not null
);
CREATE TABLE revlog (
    id              integer primary key,
    cid             integer not null,
    usn             integer not null,
    ease            integer not null,
    ivl             integer not null,
    lastIvl         integer not null,
    factor          integer not null,
    time            integer not null,
    type            integer not null
);
CREATE TABLE graves (
    usn             integer not null,
    oid             integer not null,
    type            integer not null
);
CREATE INDEX ix_notes_usn on notes (usn);
CREATE INDEX ix_cards_usn on cards (usn);
CREATE INDEX ix_revlog_usn on revlog (usn);
CREATE INDEX ix_cards_nid on cards (nid);
CREATE INDEX ix_cards_sched on cards (did, queue, due);
CREATE INDEX ix_revlog_cid on revlog (cid);
CREATE INDEX ix_notes_csum on notes (csum);
";
const COLLECTION: &str = r#"
INSERT INTO col VALUES(
    null,
    1411124400,
    1425279151694,
    1425279151690,
    11,
    0,
    0,
    0,
    '{
        "activeDecks": [
            1
        ],
        "addToCur": true,
        "collapseTime": 1200,
        "curDeck": 1,
        "curModel": "1425279151691",
        "dueCounts": true,
        "estTimes": true,
        "newBury": true,
        "newSpread": 0,
        "nextPos": 1,
        "sortBackwards": false,
        "sortType": "noteFld",
        "timeLim": 0
    }',
    '{}',
    '{
        "1": {
            "collapsed": false,
            "conf": 1,
            "desc": "",
            "dyn": 0,
            "extendNew": 10,
            "extendRev": 50,
            "id": 1,
            "lrnToday": [
                0,
                0
            ],
            "mod": 1425279151,
            "name": "Default",
            "newToday": [
                0,
                0
            ],
            "revToday": [
                0,
                0
            ],
            "timeToday": [
                0,
                0
            ],
            "usn": 0
        }
    }',
    '{
        "1": {
            "autoplay": true,
            "id": 1,
            "lapse": {
                "delays": [
                    10
                ],
                "leechAction": 0,
                "leechFails": 8,
                "minInt": 1,
                "mult": 0
            },
            "maxTaken": 60,
            "mod": 0,
            "name": "Default",
            "new": {
                "bury": true,
                "delays": [
                    1,
                    10
                ],
                "initialFactor": 2500,
                "ints": [
                    1,
                    4,
                    7
                ],
                "order": 1,
                "perDay": 20,
                "separate": true
            },
            "replayq": true,
            "rev": {
                "bury": true,
                "ease4": 1.3,
                "fuzz": 0.05,
                "ivlFct": 1,
                "maxIvl": 36500,
                "minSpace": 1,
                "perDay": 100
            },
            "timer": 0,
            "usn": 0
        }
    }',
    '{}'
)
"#;

/// Assemble one note from one normalized entry.
pub trait NoteFormat {
    /// Return one formatted note for the entry and relative media tags.
    fn note(&self, entry: &NormalizedEntry, audio: &str, image: &str) -> Note;
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
    fn json(&self, timestamp: i64) -> Value {
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

/// One formatted Anki note payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub fields: Vec<String>,
    pub guid: String,
    pub sort_field: String,
}

/// Assemble vocabulary notes from normalized entries.
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
    fn note(&self, entry: &NormalizedEntry, audio: &str, image: &str) -> Note {
        let source = if entry.highlight.is_empty() {
            entry.sentence.clone()
        } else {
            entry.sentence.replace(
                entry.highlight.as_str(),
                format!("<strong><em>{}</em></strong>", entry.highlight).as_str(),
            )
        };
        let fields = vec![
            source,
            entry.word.to_lowercase(),
            Transcription::new(entry.pronunciation.clone()).formatted(),
            entry.translation.clone(),
            HtmlLineBreaks::new(entry.example.clone()).formatted(),
            entry.importance.clone(),
            String::from(audio),
            String::from(image),
            entry.hint.clone(),
            HtmlLineBreaks::new(entry.context.clone()).formatted(),
            Transcription::new(entry.transcription.clone()).formatted(),
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

/// Assemble notes and media into one Anki deck package.
#[derive(Clone, Debug)]
pub struct VocabularyDeck<F> {
    format: F,
    id: i64,
    media: Vec<PathBuf>,
    name: String,
    notes: Vec<Note>,
    seen: BTreeSet<PathBuf>,
}

impl<F> VocabularyDeck<F>
where
    F: NoteFormat,
{
    /// Create one vocabulary deck container.
    pub fn new(
        id: i64,
        name: impl Into<String>,
        format: F,
        media: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let media = media.into_iter().collect::<Vec<_>>();
        let seen = media.iter().cloned().collect::<BTreeSet<_>>();
        Self {
            format,
            id,
            media,
            name: name.into(),
            notes: Vec::new(),
            seen,
        }
    }

    /// Add one formatted note to the deck.
    pub fn add(&mut self, entry: &NormalizedEntry, audio: &str, image: &str) {
        self.notes.push(self.format.note(entry, audio, image));
    }

    /// Attach one media file path without duplicating earlier paths.
    pub fn attach(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if self.seen.insert(path.clone()) {
            self.media.push(path);
        }
    }

    /// Return the attached media files in packaging order.
    pub fn media(&self) -> &[PathBuf] {
        self.media.as_slice()
    }

    /// Return the formatted notes in insertion order.
    pub fn notes(&self) -> &[Note] {
        self.notes.as_slice()
    }

    /// Export the deck as one APKG archive.
    pub fn save(&self, output: impl AsRef<Path>) -> Result<()> {
        self.save_at(output.as_ref(), stamp()?)
    }

    fn save_at(&self, output: &Path, timestamp: f64) -> Result<()> {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let directory = TempDir::new()?;
        let database = directory.path().join("collection.anki2");
        let mut conn = Connection::open(&database)?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(COLLECTION)?;
        let mut ids = Identifiers::new(timestamp);
        let second = timestamp as i64;
        self.write(&mut conn, second, &mut ids)?;
        conn.close().map_err(|(_, error)| error)?;
        self.zip(&database, output)?;
        Ok(())
    }

    fn write(&self, conn: &mut Connection, timestamp: i64, ids: &mut Identifiers) -> Result<()> {
        let mut decks = parsed(conn, "decks")?;
        decks.insert(self.id.to_string(), self.deck());
        stored(conn, "decks", Value::Object(decks))?;
        let model = self.format.model();
        let mut models = parsed(conn, "models")?;
        models.insert(model.id.to_string(), model.json(timestamp));
        stored(conn, "models", Value::Object(models))?;
        for note in &self.notes {
            let note_id = ids.next();
            conn.execute(
                "INSERT INTO notes VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    note_id,
                    note.guid,
                    model.id,
                    timestamp,
                    -1,
                    "",
                    note.fields.join("\x1f"),
                    note.sort_field,
                    0,
                    0,
                    "",
                ],
            )?;
            conn.execute(
                "INSERT INTO cards VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    ids.next(),
                    note_id,
                    self.id,
                    0,
                    timestamp,
                    -1,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    "",
                ],
            )?;
        }
        Ok(())
    }

    fn zip(&self, database: &Path, output: &Path) -> Result<()> {
        let file = fs::File::create(output)?;
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer.start_file("collection.anki2", options)?;
        copy(database, &mut writer)?;
        writer.start_file("media", options)?;
        writer.write_all(media(self.media.as_slice())?.as_bytes())?;
        for (index, path) in self.media.iter().enumerate() {
            writer.start_file(index.to_string(), options)?;
            copy(path, &mut writer)?;
        }
        writer.finish()?;
        Ok(())
    }

    fn deck(&self) -> Value {
        json!({
            "collapsed": false,
            "conf": 1,
            "desc": "",
            "dyn": 0,
            "extendNew": 0,
            "extendRev": 50,
            "id": self.id,
            "lrnToday": [163, 2],
            "mod": 1425278051,
            "name": self.name,
            "newToday": [163, 2],
            "revToday": [163, 0],
            "timeToday": [163, 23598],
            "usn": -1,
        })
    }
}

fn stamp() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

fn parsed(conn: &Connection, column: &str) -> Result<Map<String, Value>> {
    let value = conn.query_row(format!("SELECT {column} FROM col").as_str(), [], |row| {
        row.get::<_, String>(0)
    })?;
    let Value::Object(map) = serde_json::from_str::<Value>(value.as_str())? else {
        bail!("Collection column '{column}' is not a JSON object");
    };
    Ok(map)
}

fn stored(conn: &Connection, column: &str, value: Value) -> Result<()> {
    conn.execute(
        format!("UPDATE col SET {column} = ?").as_str(),
        params![serde_json::to_string(&value)?],
    )?;
    Ok(())
}

fn media(paths: &[PathBuf]) -> Result<String> {
    let mut value = Map::new();
    for (index, path) in paths.iter().enumerate() {
        let Some(name) = path.file_name() else {
            bail!("Attached media path has no filename: {}", path.display());
        };
        value.insert(
            index.to_string(),
            Value::String(name.to_string_lossy().into_owned()),
        );
    }
    Ok(serde_json::to_string(&Value::Object(value))?)
}

fn copy(path: &Path, writer: &mut ZipWriter<fs::File>) -> Result<()> {
    let mut reader = fs::File::open(path)?;
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer)?;
    writer.write_all(buffer.as_slice())?;
    Ok(())
}

fn guid(fields: &[String]) -> String {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identifiers {
    next: i64,
}

impl Identifiers {
    fn new(timestamp: f64) -> Self {
        Self {
            next: (timestamp * 1000.0) as i64,
        }
    }

    fn next(&mut self) -> i64 {
        let value = self.next;
        self.next += 1;
        value
    }
}
