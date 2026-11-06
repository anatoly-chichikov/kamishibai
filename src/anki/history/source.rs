//! Read-only schema-18 collection access and optional system process inspection.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{Connection, OpenFlags};

use super::manifest::{Card, Collection, Deck, Manifest, Note, Notetype, Revlog};
use super::summary::DeckSummary;
use super::{ArchiveSource, HistoryError, Snapshot, media};

/// Process-inspection port; tests can model running Anki and changes during a read.
pub trait AnkiProcess {
    /// Report whether Anki or an open collection handle was observed.
    fn running(&self, collection: &Path) -> io::Result<bool>;
}

/// Optional OS probes supplement the mandatory collection WAL check.
pub struct SystemAnkiProcess;

impl AnkiProcess for SystemAnkiProcess {
    fn running(&self, collection: &Path) -> io::Result<bool> {
        #[cfg(unix)]
        {
            let running = probe(
                Command::new("pgrep")
                    .args(["-if", "(^|/)(anki|anki-bin)( |$)|Anki.app/Contents/MacOS/"]),
            )?;
            let open = probe(Command::new("lsof").arg("-t").arg(collection))?;
            Ok(running || open)
        }
        #[cfg(windows)]
        {
            let _ = collection;
            match Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq anki.exe", "/FO", "CSV", "/NH"])
                .output()
            {
                Ok(output) if output.status.success() => {
                    Ok(String::from_utf8_lossy(&output.stdout)
                        .to_ascii_lowercase()
                        .contains("\"anki.exe\""))
                }
                Ok(_) => Err(io::Error::other("cannot inspect Anki processes")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = collection;
            Ok(false)
        }
    }
}

#[cfg(unix)]
fn probe(command: &mut Command) -> io::Result<bool> {
    match command.output() {
        Ok(output) if output.status.success() => Ok(!output.stdout.is_empty()),
        Ok(output) if output.status.code() == Some(1) && output.stderr.is_empty() => Ok(false),
        Ok(_) => Err(io::Error::other(
            "cannot inspect Anki processes or open collection handles",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Resolve standard Anki roots, with `KAMISHIBAI_ANKI` as an exact override.
#[must_use]
pub fn roots() -> Vec<PathBuf> {
    if let Some(root) = std::env::var_os("KAMISHIBAI_ANKI") {
        return vec![PathBuf::from(root)];
    }
    let mut roots: Vec<_> = dirs::data_dir()
        .map(|path| path.join("Anki2"))
        .into_iter()
        .collect();
    if cfg!(target_os = "linux")
        && let Some(home) = dirs::home_dir()
    {
        roots.push(home.join(".var/app/net.ankiweb.Anki/data/Anki2"));
    }
    roots
}

/// List existing profile collection paths in stable order without creating files.
pub fn discover(roots: &[PathBuf]) -> Result<Vec<PathBuf>, HistoryError> {
    let mut paths = BTreeSet::new();
    for root in roots {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path().join("collection.anki2");
            if path.is_file() {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

/// Read the installed Anki version when the platform provides it, else `unknown`.
#[must_use]
pub fn application_version() -> String {
    if cfg!(target_os = "macos") {
        return Command::new("defaults")
            .args([
                "read",
                "/Applications/Anki.app/Contents/Info",
                "CFBundleShortVersionString",
            ])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|version| !version.is_empty())
            .unwrap_or_else(|| "unknown".into());
    }
    "unknown".into()
}

/// Read a closed collection, selecting exact deck names or Kamishibai defaults.
/// An empty selection result is a refusal; descendants are separate deck names.
pub fn read(
    path: &Path,
    names: &[String],
    process: &dyn AnkiProcess,
    application: &str,
) -> Result<Snapshot, HistoryError> {
    if !path.is_file() {
        return Err(HistoryError::Selection(format!(
            "Anki collection not found at {}",
            path.display()
        )));
    }
    let path = path.canonicalize()?;
    closed(&path, process)?;
    let before = media::digest(&mut File::open(&path)?)?;
    let connection = Connection::open_with_flags(
        uri(&path)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.create_collation("unicase", |left: &str, right: &str| {
        left.to_lowercase().cmp(&right.to_lowercase())
    })?;
    connection.pragma_update(None, "query_only", "ON")?;
    let mut manifest = rows(&connection, application)?;
    drop(connection);
    let selected = selection(&manifest, names)?;
    let decks = manifest
        .decks
        .iter()
        .map(|deck| {
            DeckSummary::from_rows(
                deck,
                selected.contains(&deck.id),
                &manifest.cards,
                &manifest.revlog,
            )
        })
        .collect();
    manifest.decks.retain(|deck| selected.contains(&deck.id));
    manifest.cards.retain(|card| selected.contains(&card.deck));
    let cards: BTreeSet<_> = manifest.cards.iter().map(|card| card.id).collect();
    let notes: BTreeSet<_> = manifest.cards.iter().map(|card| card.note).collect();
    manifest.notes.retain(|note| notes.contains(&note.id));
    manifest
        .revlog
        .retain(|review| cards.contains(&review.card));
    let types: BTreeSet<_> = manifest.notes.iter().map(|note| note.notetype).collect();
    manifest.notetypes.retain(|kind| types.contains(&kind.id));
    let media = media::collect(
        &path
            .parent()
            .expect("invariant: canonical collection has a parent")
            .join("collection.media"),
        &manifest.notes,
    )?;
    manifest.media = media.rows;
    closed(&path, process)?;
    if media::digest(&mut File::open(&path)?)? != before {
        return Err(HistoryError::Changed);
    }
    let source = ArchiveSource::new(
        path.parent()
            .expect("invariant: canonical collection has a parent")
            .to_path_buf(),
        media.files,
    );
    let snapshot = Snapshot::new(manifest, source, decks, media.missing);
    snapshot.preview()?;
    Ok(snapshot)
}

fn closed(path: &Path, process: &dyn AnkiProcess) -> Result<(), HistoryError> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    if Path::new(&wal).try_exists()? || process.running(path)? {
        return Err(HistoryError::Open);
    }
    Ok(())
}

fn uri(path: &Path) -> Result<String, HistoryError> {
    let name = path
        .to_str()
        .ok_or_else(|| HistoryError::Selection("collection path must be valid UTF-8".into()))?;
    let encoded: String = name
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect();
    Ok(format!("file:{encoded}?mode=ro&immutable=1"))
}

fn selection(manifest: &Manifest, names: &[String]) -> Result<BTreeSet<i64>, HistoryError> {
    let selected: BTreeSet<i64> = if names.is_empty() {
        let types: BTreeSet<_> = manifest
            .notetypes
            .iter()
            .filter(|kind| kind.name.starts_with("Kamishibai Vocabulary Model"))
            .map(|kind| kind.id)
            .collect();
        let notes: BTreeSet<_> = manifest
            .notes
            .iter()
            .filter(|note| types.contains(&note.notetype))
            .map(|note| note.id)
            .collect();
        manifest
            .cards
            .iter()
            .filter(|card| notes.contains(&card.note))
            .map(|card| card.deck)
            .collect()
    } else {
        names
            .iter()
            .map(|name| {
                manifest
                    .decks
                    .iter()
                    .find(|deck| &deck.name == name)
                    .map(|deck| deck.id)
                    .ok_or_else(|| HistoryError::Selection(format!("Anki deck not found: {name}")))
            })
            .collect::<Result<_, _>>()?
    };
    if selected.is_empty() {
        return Err(HistoryError::Selection(
            "no Anki decks selected; pass --deck NAME".into(),
        ));
    }
    Ok(selected)
}

fn rows(connection: &Connection, application: &str) -> Result<Manifest, HistoryError> {
    let (schema, crt, modified): (i64, i64, i64) =
        connection.query_row("SELECT ver,crt,mod FROM col", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if schema != 18 {
        return Err(HistoryError::Schema(schema));
    }
    let decks = connection
        .prepare("SELECT id,name FROM decks ORDER BY id")?
        .query_map([], |row| {
            Ok(Deck {
                id: row.get(0)?,
                name: row.get::<_, String>(1)?.replace('\u{1f}', "::"),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut notetypes = connection
        .prepare("SELECT id,name FROM notetypes ORDER BY id")?
        .query_map([], |row| {
            Ok(Notetype {
                id: row.get(0)?,
                name: row.get(1)?,
                fields: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for kind in &mut notetypes {
        kind.fields = connection
            .prepare("SELECT name FROM fields WHERE ntid=? ORDER BY ord")?
            .query_map([kind.id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
    }
    let notes = connection
        .prepare("SELECT id,guid,mid,mod,tags,flds FROM notes ORDER BY id")?
        .query_map([], |row| {
            Ok(Note {
                id: row.get(0)?,
                guid: row.get(1)?,
                notetype: row.get(2)?,
                modified: row.get(3)?,
                tags: row.get(4)?,
                fields: row
                    .get::<_, String>(5)?
                    .split('\u{1f}')
                    .map(str::to_owned)
                    .collect(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let cards = connection.prepare("SELECT id,nid,CASE WHEN odid<>0 THEN odid ELSE did END,ord,type,queue,CASE WHEN odid<>0 AND odue<>0 THEN odue ELSE due END,ivl,factor,reps,lapses FROM cards ORDER BY id")?.query_map([], |row| Ok(Card { id: row.get(0)?, note: row.get(1)?, deck: row.get(2)?, ordinal: row.get(3)?, kind: row.get(4)?, queue: row.get(5)?, due: row.get(6)?, interval: row.get(7)?, factor: row.get(8)?, reps: row.get(9)?, lapses: row.get(10)? }))?.collect::<Result<Vec<_>, _>>()?;
    let revlog = connection
        .prepare("SELECT id,cid,ease,ivl,lastIvl,factor,time,type FROM revlog ORDER BY id")?
        .query_map([], |row| {
            Ok(Revlog {
                id: row.get(0)?,
                card: row.get(1)?,
                ease: row.get(2)?,
                interval: row.get(3)?,
                last_interval: row.get(4)?,
                factor: row.get(5)?,
                time: row.get(6)?,
                kind: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Manifest {
        collection: Collection {
            crt,
            modified,
            schema,
            profile: String::new(),
            application: application.to_owned(),
        },
        decks,
        notetypes,
        notes,
        cards,
        revlog,
        media: Vec::new(),
    })
}
