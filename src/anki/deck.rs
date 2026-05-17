use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::vocabulary::VocabularyEntry;

use super::{Model, Note, NoteFormat};

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
    pub fn add(&mut self, entry: &VocabularyEntry, audio: &str, image: &str) {
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
        let tx = conn.transaction()?;
        let mut decks = parsed(&tx, "decks")?;
        decks.insert(self.id.to_string(), self.deck());
        stored(&tx, "decks", Value::Object(decks))?;
        let model: &Model = self.format.model();
        let mut models = parsed(&tx, "models")?;
        models.insert(model.id.to_string(), model.json(timestamp));
        stored(&tx, "models", Value::Object(models))?;
        let mut notes = tx.prepare("INSERT INTO notes VALUES(?,?,?,?,?,?,?,?,?,?,?)")?;
        let mut cards =
            tx.prepare("INSERT INTO cards VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")?;
        for note in &self.notes {
            let note_id = ids.next();
            notes.execute(params![
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
            ])?;
            cards.execute(params![
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
            ])?;
        }
        drop(notes);
        drop(cards);
        tx.commit()?;
        Ok(())
    }

    fn zip(&self, database: &Path, output: &Path) -> Result<()> {
        let file = fs::File::create(output)?;
        let mut writer = ZipWriter::new(file);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("collection.anki2", deflated)?;
        copy(database, &mut writer)?;
        writer.start_file("media", deflated)?;
        writer.write_all(media(self.media.as_slice())?.as_bytes())?;
        for (index, path) in self.media.iter().enumerate() {
            writer.start_file(index.to_string(), stored)?;
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
