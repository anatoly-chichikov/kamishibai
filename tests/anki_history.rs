//! Offline history import contracts over a real synthetic SQLite collection.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

/// Build an isolated schema-18 profile with filtered, nested and unrelated decks.
fn collection(root: &Path) -> PathBuf {
    let folder = root.join("Private-profile-947");
    fs::create_dir_all(folder.join("collection.media")).unwrap();
    let path = folder.join("collection.anki2");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        r#"CREATE TABLE col(ver INTEGER,crt INTEGER,mod INTEGER);
         INSERT INTO col VALUES(18,1700000000,1700500000000);
         CREATE TABLE decks(id INTEGER PRIMARY KEY,name TEXT);
         INSERT INTO decks VALUES(17,'French'),(29,'French'||char(31)||'Daily'),(41,'Unselected-private-deck'),(53,'Filtered');
         CREATE TABLE notetypes(id INTEGER PRIMARY KEY,name TEXT);
         INSERT INTO notetypes VALUES(71,'Kamishibai Vocabulary Model FR'),(83,'Other private model');
         CREATE TABLE fields(ntid INTEGER,ord INTEGER,name TEXT);
         INSERT INTO fields VALUES(71,0,'Term'),(71,1,'Meaning'),(71,2,'SourceSentence'),(71,3,'TargetSentence'),(71,4,'Audio'),(71,5,'Illustration'),(83,0,'Word');
         CREATE TABLE notes(id INTEGER PRIMARY KEY,guid TEXT,mid INTEGER,mod INTEGER,tags TEXT,flds TEXT);
         INSERT INTO notes VALUES(101,'fixture-chat',71,1700400000,' fixture ','chat'||char(31)||'кот'||char(31)||'Это <b>кот</b>.'||char(31)||'Le chat dort.'||char(31)||'[sound:voice.wav]'||char(31)||'<img src="picture.png">'),(103,'fixture-ete',71,1700400001,'','été'||char(31)||'лето'||char(31)||'Наступило лето.'||char(31)||'C’est l’été.'||char(31)||'[sound:alias.wav]'||char(31)||''),(107,'private-note',83,1700400002,'private-tag','[sound:private.wav]');
         CREATE TABLE cards(id INTEGER PRIMARY KEY,nid INTEGER,did INTEGER,ord INTEGER,type INTEGER,queue INTEGER,due INTEGER,ivl INTEGER,factor INTEGER,reps INTEGER,lapses INTEGER,odid INTEGER,odue INTEGER);
         INSERT INTO cards VALUES(211,101,53,0,2,2,-7,13,2450,9,2,17,412),(223,103,29,0,2,2,419,19,2510,4,1,0,0),(227,107,41,0,2,2,427,23,2310,7,3,0,0);
         CREATE TABLE revlog(id INTEGER PRIMARY KEY,cid INTEGER,ease INTEGER,ivl INTEGER,lastIvl INTEGER,factor INTEGER,time INTEGER,type INTEGER);
         INSERT INTO revlog VALUES(1700100000001,211,1,-60,9,2450,3142,1),(1700100000002,211,3,13,-60,2450,2913,2),(1700100000003,223,4,19,11,2510,1981,1),(1700100000004,227,1,23,17,2310,2711,1),(1700100000005,999,1,1,0,2500,1000,0);"#,
    ).unwrap();
    let sound = b"RIFF&\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x02\0\0\0\0\0";
    fs::write(folder.join("collection.media/voice.wav"), sound).unwrap();
    fs::write(folder.join("collection.media/alias.wav"), sound).unwrap();
    image::RgbaImage::from_pixel(3, 2, image::Rgba([19, 41, 67, 255]))
        .save(folder.join("collection.media/picture.png"))
        .unwrap();
    fs::write(
        folder.join("collection.media/private.wav"),
        b"excluded-private-media",
    )
    .unwrap();
    path
}

/// Capture bytes and metadata recursively, excluding no profile files.
fn inventory(root: &Path) -> BTreeMap<PathBuf, (u64, SystemTime, Vec<u8>)> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let meta = entry.metadata().unwrap();
        files.insert(
            path.clone(),
            (
                meta.len(),
                meta.modified().unwrap(),
                if meta.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                },
            ),
        );
        if meta.is_dir() {
            files.extend(inventory(&path));
        }
    }
    files
}

/// Run the import without provider credentials or live profile discovery.
fn command(root: &Path, out: &Path) -> Command {
    let mut command = Command::cargo_bin("kamishibai").unwrap();
    command
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENROUTER_API_KEY");
    command
        .env("KAMISHIBAI_ANKI", root)
        .env("KAMISHIBAI_DATA", root.join("preferences"))
        .env("KAMISHIBAI_CACHE", root.join("cache"))
        .env("KAMISHIBAI_OUTPUT", root.join("packages"))
        .env("PATH", root.join("no-system-probes"))
        .args(["export", "--out"])
        .arg(out)
        .arg("--json");
    command.timeout(std::time::Duration::from_secs(30));
    command
}

#[test]
fn a_missing_collection_cannot_suggest_choosing_decks() {
    let root = TempDir::new().unwrap();
    let output = command(root.path(), &root.path().join("output"))
        .arg("--collection")
        .arg(root.path().join("absent-739.anki2"))
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(2)
            && doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Anki collection not found")
            && !doc["error"]["hint"]
                .as_str()
                .unwrap_or("")
                .contains("--deck"),
        "a missing collection incorrectly suggested choosing decks: {doc}"
    );
}

#[test]
fn preview_cannot_write_or_change_the_profile() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let before = inventory(root.path());
    let output = command(root.path(), &root.path().join("absent-output"))
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.success()
            && doc["preview"]["counts"]["cards"] == 2
            && before == inventory(root.path()),
        "preview wrote files, changed the source or failed: {doc}"
    );
}

#[test]
fn even_an_empty_wal_cannot_be_read() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    fs::write(path.with_file_name("collection.anki2-wal"), []).unwrap();
    let output = command(root.path(), root.path()).output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(4) && doc["error"]["retryable"] == true,
        "empty WAL did not produce a retryable not-ready refusal: {doc}"
    );
}

#[test]
fn unsupported_schema_cannot_be_read() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    Connection::open(path)
        .unwrap()
        .execute("UPDATE col SET ver=19", [])
        .unwrap();
    let output = command(root.path(), root.path()).output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(1)
            && doc["error"]["hint"].as_str().unwrap().contains("update"),
        "unsupported schema was accepted or lost its recovery hint: {doc}"
    );
}

#[test]
fn explicit_decks_cannot_leak_other_rows_or_media() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let before = inventory(path.parent().unwrap());
    let output = command(root.path(), &root.path().join("output"))
        .args(["--deck", "French", "--save"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    let file = fs::File::open(doc["file"].as_str().unwrap()).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let snapshot: Value = serde_json::from_reader(zip.by_name("snapshot.json").unwrap()).unwrap();
    let bytes = fs::read(doc["file"].as_str().unwrap()).unwrap();
    assert!(
        output.status.success()
            && snapshot["cards"].as_array().unwrap().len() == 1
            && snapshot["cards"][0]["deck"] == 17
            && snapshot["cards"][0]["due"] == 412
            && snapshot["revlog"].as_array().unwrap().len() == 2
            && snapshot["collection"]["profile"] == ""
            && !String::from_utf8_lossy(&bytes).contains("Private-profile-947")
            && !String::from_utf8_lossy(&bytes).contains("private-")
            && !String::from_utf8_lossy(&bytes).contains(root.path().to_str().unwrap())
            && before == inventory(path.parent().unwrap()),
        "selection leaked excluded data or changed the source: {snapshot}"
    );
}

#[test]
fn repeated_saves_cannot_overwrite_a_file() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("output");
    let first = command(root.path(), &out).arg("--save").output().unwrap();
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    let bytes = fs::read(first["file"].as_str().unwrap()).unwrap();
    let second = command(root.path(), &out).arg("--save").output().unwrap();
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert!(
        second["file"].as_str().unwrap().ends_with("-2.zip")
            && fs::read(first["file"].as_str().unwrap()).unwrap() == bytes
            && fs::read_dir(out).unwrap().count() == 2,
        "saving overwrote a file or left a partial archive"
    );
}

#[test]
fn an_unknown_deck_cannot_fall_back_to_the_defaults() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let output = command(root.path(), root.path())
        .args(["--deck", "missing"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(2) && doc["error"]["code"] == "usage",
        "unknown deck fell back to a default selection: {doc}"
    );
}

#[test]
fn nested_decks_are_selected_by_their_full_name() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let output = command(root.path(), root.path())
        .args(["--deck", "French::Daily"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.success()
            && doc["preview"]["counts"]["cards"] == 1
            && doc["preview"]["counts"]["reviews"] == 1,
        "nested deck did not resolve to its exact name: {doc}"
    );
}

#[test]
fn absent_profiles_require_an_explicit_collection() {
    let root = TempDir::new().unwrap();
    let output = command(root.path(), root.path()).output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(2)
            && doc["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("--collection"),
        "missing Anki did not explain the collection override: {doc}"
    );
}

#[test]
fn multiple_profiles_cannot_be_chosen_implicitly() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    fs::create_dir(root.path().join("Second-profile")).unwrap();
    fs::copy(path, root.path().join("Second-profile/collection.anki2")).unwrap();
    let output = command(root.path(), root.path()).output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(2)
            && doc["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("Second-profile"),
        "ambiguous profiles did not produce a usage refusal with candidates: {doc}"
    );
}

#[test]
fn a_profile_name_cannot_escape_the_anki_root() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let output = command(root.path(), root.path())
        .args(["--profile", "../Private-profile-947"])
        .output()
        .unwrap();
    assert!(
        output.status.code() == Some(2),
        "profile traversal was not refused"
    );
}

#[test]
fn profile_and_collection_cannot_be_combined() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let output = command(root.path(), root.path())
        .args(["--profile", "Private-profile-947", "--collection"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.code() == Some(2),
        "conflicting source selectors were accepted"
    );
}

#[test]
fn an_empty_default_selection_cannot_be_saved() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    Connection::open(path)
        .unwrap()
        .execute("UPDATE notetypes SET name='Unrelated model'", [])
        .unwrap();
    let output = command(root.path(), &root.path().join("output"))
        .arg("--save")
        .output()
        .unwrap();
    assert!(
        output.status.code() == Some(2) && !root.path().join("output").exists(),
        "empty default selection created an archive or wrote its output folder"
    );
}

#[test]
fn missing_media_remain_visible_in_the_preview() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    fs::remove_file(path.parent().unwrap().join("collection.media/picture.png")).unwrap();
    let output = command(root.path(), root.path()).output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.success()
            && doc["preview"]["missing_media"] == serde_json::json!(["picture.png"]),
        "preview hid the missing picture: {doc}"
    );
}

#[test]
fn a_write_failure_cannot_leave_a_partial_archive() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("output");
    fs::write(&out, b"existing file").unwrap();
    let output = command(root.path(), &out).arg("--save").output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(1)
            && doc["error"]["code"] == "operational"
            && fs::read(out).unwrap() == b"existing file",
        "write failure did not preserve the destination: {doc}"
    );
}

/// Closed-process fake keeps library tests independent of desktop applications.
struct Closed;

impl kamishibai::anki::history::AnkiProcess for Closed {
    fn running(&self, _: &Path) -> std::io::Result<bool> {
        Ok(false)
    }
}

/// Save a deterministic, synthetic archive through the public library API.
fn saved(root: &Path) -> PathBuf {
    let path = collection(root);
    kamishibai::anki::history::read(&path, &[], &Closed, "unknown")
        .unwrap()
        .save(
            &root.join("output"),
            time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
            "1.14.0",
        )
        .unwrap()
}

/// Repack tiny synthetic entries while independently changing the wire contract.
fn repack(
    path: &Path,
    method: zip::CompressionMethod,
    change: impl FnOnce(&mut BTreeMap<String, Vec<u8>>),
) -> PathBuf {
    use std::io::{Read, Write};
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut entries = BTreeMap::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        entries.insert(file.name().to_owned(), bytes);
    }
    change(&mut entries);
    let output = path.with_file_name("changed.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&output).unwrap());
    for (name, bytes) in entries {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default().compression_method(method),
        )
        .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    output
}

/// Recompute the header to test strict snapshot decoding independently of its hash.
fn patch_snapshot(entries: &mut BTreeMap<String, Vec<u8>>, change: impl FnOnce(&mut Value)) {
    use sha2::{Digest, Sha256};
    let mut snapshot: Value = serde_json::from_slice(&entries["snapshot.json"]).unwrap();
    change(&mut snapshot);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let mut header: Value = serde_json::from_slice(&entries["header.json"]).unwrap();
    header["snapshot"]["sha256"] = Value::String(
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    );
    header["snapshot"]["size"] = serde_json::json!(bytes.len());
    entries.insert("snapshot.json".into(), bytes);
    entries.insert("header.json".into(), serde_json::to_vec(&header).unwrap());
}

#[test]
fn a_running_process_cannot_bypass_the_missing_wal() {
    struct Running;
    impl kamishibai::anki::history::AnkiProcess for Running {
        fn running(&self, _: &Path) -> std::io::Result<bool> {
            Ok(true)
        }
    }
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    assert!(
        matches!(
            kamishibai::anki::history::read(&path, &[], &Running, "unknown"),
            Err(kamishibai::anki::history::HistoryError::Open)
        ),
        "a live process was ignored when WAL was absent"
    );
}

#[test]
fn a_collection_changed_during_read_cannot_be_exported() {
    struct Changing(std::cell::Cell<u8>);
    impl kamishibai::anki::history::AnkiProcess for Changing {
        fn running(&self, path: &Path) -> std::io::Result<bool> {
            self.0.set(self.0.get() + 1);
            if self.0.get() == 2 {
                Connection::open(path)
                    .unwrap()
                    .execute("UPDATE col SET mod=mod+1", [])
                    .unwrap();
            }
            Ok(false)
        }
    }
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    assert!(
        matches!(
            kamishibai::anki::history::read(
                &path,
                &[],
                &Changing(std::cell::Cell::new(0)),
                "unknown"
            ),
            Err(kamishibai::anki::history::HistoryError::Changed)
        ),
        "a changed collection escaped the final fingerprint check"
    );
}

#[test]
fn version_one_fixture_cannot_drift_between_two_exports() {
    let root = TempDir::new().unwrap();
    let first = saved(root.path());
    let second = kamishibai::anki::history::read(
        &root.path().join("Private-profile-947/collection.anki2"),
        &[],
        &Closed,
        "unknown",
    )
    .unwrap()
    .save(
        &root.path().join("output"),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    )
    .unwrap();
    let bytes = fs::read(&first).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anki-history/v1.zip");
    if std::env::var_os("UPDATE_ANKI_HISTORY_FIXTURE").as_deref() == Some(std::ffi::OsStr::new("1"))
    {
        fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        fs::write(&fixture, &bytes).unwrap();
    }
    assert!(
        bytes == fs::read(second).unwrap() && bytes == fs::read(fixture).unwrap(),
        "version-one fixture drifted or two deterministic exports differed"
    );
}

#[test]
fn default_selection_drops_deleted_reviews_and_deduplicates_media() {
    let root = TempDir::new().unwrap();
    let file = saved(root.path());
    let preview =
        serde_json::to_value(kamishibai::anki::history::validate(&file).unwrap()).unwrap();
    assert!(
        preview["counts"]["notes"] == 2
            && preview["counts"]["reviews"] == 3
            && preview["counts"]["mistakes"] == 1
            && preview["counts"]["media"] == 3
            && preview["counts"]["files"] == 2,
        "selection kept deleted reviews or duplicated media payloads: {preview}"
    );
}

#[test]
fn a_corrupt_snapshot_hash_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            let mut header: Value = serde_json::from_slice(&entries["header.json"]).unwrap();
            header["snapshot"]["sha256"] = Value::String("0".repeat(64));
            entries.insert("header.json".into(), serde_json::to_vec(&header).unwrap());
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "corrupt snapshot hash passed validation"
    );
}

#[test]
fn extra_entries_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            entries.insert("private/profile.txt".into(), b"private".to_vec());
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "extra archive entry passed validation"
    );
}

#[test]
fn missing_entries_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            let name = entries
                .keys()
                .find(|name| name.starts_with("media/"))
                .unwrap()
                .clone();
            entries.remove(&name);
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "missing media entry passed validation"
    );
}

#[test]
fn compressed_entries_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Deflated,
        |_| {},
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "compressed archive passed validation"
    );
}

#[test]
fn changed_media_cannot_pass_a_matching_zip_crc() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            let (_, bytes) = entries
                .iter_mut()
                .find(|(name, _)| name.starts_with("media/"))
                .unwrap();
            bytes[0] ^= 1;
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "changed media with valid CRC escaped the SHA-256 check"
    );
}

#[test]
fn unknown_snapshot_fields_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            patch_snapshot(entries, |snapshot| {
                snapshot["source_path"] = Value::String("/private/profile".into())
            })
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "unknown manifest fields passed the frozen wire schema"
    );
}

#[test]
fn unsupported_archive_versions_cannot_pass_validation() {
    let root = TempDir::new().unwrap();
    let file = repack(
        &saved(root.path()),
        zip::CompressionMethod::Stored,
        |entries| {
            let mut header: Value = serde_json::from_slice(&entries["header.json"]).unwrap();
            header["version"] = serde_json::json!(2);
            entries.insert("header.json".into(), serde_json::to_vec(&header).unwrap());
        },
    );
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "an unsupported version passed validation"
    );
}

#[test]
fn changed_media_cannot_leave_a_part_file_or_publish() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    fs::write(
        path.parent().unwrap().join("collection.media/picture.png"),
        b"changed after preview",
    )
    .unwrap();
    let out = root.path().join("output");
    let result = snapshot.save(
        &out,
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && fs::read_dir(out).unwrap().count() == 0,
        "a failed media verification left an archive or part file behind"
    );
}

#[cfg(unix)]
#[test]
fn a_history_file_cannot_be_read_by_other_users() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let file = saved(root.path());
    assert_eq!(
        fs::metadata(file).unwrap().permissions().mode() & 0o777,
        0o600,
        "history file permissions exposed its contents"
    );
}

#[cfg(unix)]
#[test]
fn media_symlinks_cannot_read_outside_the_profile() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let picture = path.parent().unwrap().join("collection.media/picture.png");
    fs::remove_file(&picture).unwrap();
    fs::write(root.path().join("private.png"), b"outside-profile-secret").unwrap();
    std::os::unix::fs::symlink(root.path().join("private.png"), picture).unwrap();
    let preview = kamishibai::anki::history::read(&path, &[], &Closed, "unknown")
        .unwrap()
        .preview()
        .unwrap();
    assert_eq!(
        serde_json::to_value(preview).unwrap()["missing_media"],
        serde_json::json!(["picture.png"]),
        "a media symlink escaped the source directory"
    );
}

#[test]
fn oversized_snapshots_are_refused_before_creating_output() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE notes SET flds=CAST(zeroblob(12000000) AS TEXT) WHERE id=101",
            [],
        )
        .unwrap();
    let out = root.path().join("output");
    let output = command(root.path(), &out).arg("--save").output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(1)
            && doc["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("fewer decks")
            && !out.exists(),
        "oversized snapshot wrote output or lost its limit refusal: {doc}"
    );
}

#[test]
fn concurrent_saves_cannot_replace_each_other() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let barrier = std::sync::Barrier::new(3);
    let out = root.path().join("output");
    let paths = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..3)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    snapshot
                        .save(
                            &out,
                            time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
                            "1.14.0",
                        )
                        .unwrap()
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    let names: std::collections::BTreeSet<_> = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.len() == 3
            && names.iter().any(|name| name.ends_with("-3.zip"))
            && fs::read_dir(out).unwrap().count() == 3
            && paths
                .iter()
                .all(|path| kamishibai::anki::history::validate(path).is_ok()),
        "concurrent saves replaced each other or left partial output"
    );
}

#[test]
fn small_zip64_entries_remain_valid() {
    use std::io::{Read, Write};
    let root = TempDir::new().unwrap();
    let original = saved(root.path());
    let mut source = zip::ZipArchive::new(fs::File::open(original).unwrap()).unwrap();
    let file = root.path().join("zip64.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&file).unwrap());
    for index in 0..source.len() {
        let mut entry = source.by_index(index).unwrap();
        writer
            .start_file(
                entry.name(),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored)
                    .large_file(true),
            )
            .unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
    assert!(
        kamishibai::anki::history::validate(&file).is_ok(),
        "valid Zip64 entries were rejected"
    );
}

#[test]
fn corrupted_zip_crc_cannot_pass_even_with_unchanged_content() {
    use std::io::{Seek, SeekFrom, Write};
    let root = TempDir::new().unwrap();
    let file = saved(root.path());
    let mut source = zip::ZipArchive::new(fs::File::open(&file).unwrap()).unwrap();
    let entry = source.by_name("snapshot.json").unwrap();
    let offset = entry.central_header_start() + 16;
    let crc = entry.crc32() ^ 1;
    let mut writer = fs::OpenOptions::new().write(true).open(&file).unwrap();
    writer.seek(SeekFrom::Start(offset)).unwrap();
    writer.write_all(&crc.to_le_bytes()).unwrap();
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "a corrupt ZIP checksum was ignored"
    );
}

#[test]
fn duplicate_zip_names_cannot_hide_an_extra_entry() {
    let root = TempDir::new().unwrap();
    let file = saved(root.path());
    let source = zip::ZipArchive::new(fs::File::open(&file).unwrap()).unwrap();
    let names = source
        .file_names()
        .filter(|name| name.starts_with("media/"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut bytes = fs::read(&file).unwrap();
    let offsets = bytes
        .windows(names[1].len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == names[1].as_bytes())
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    for offset in offsets {
        bytes[offset..offset + names[0].len()].copy_from_slice(names[0].as_bytes());
    }
    fs::write(&file, bytes).unwrap();
    assert!(
        kamishibai::anki::history::validate(&file).is_err(),
        "ZIP name deduplication hid an extra archive entry"
    );
}

#[test]
fn review_save_cannot_write_inside_the_anki_profile() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let _ = snapshot.save(
        profile,
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        before == inventory(profile),
        "saving changed the Anki profile directory"
    );
}

#[test]
fn saving_cannot_create_a_missing_profile_subdirectory() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &profile.join("exports-529/nested-83"),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(profile),
        "saving created a directory inside the Anki profile"
    );
}

#[cfg(unix)]
#[test]
fn saving_cannot_follow_a_symlink_into_the_profile() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let alias = root.path().join("linked-721");
    std::os::unix::fs::symlink(profile, &alias).unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &alias,
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(profile),
        "saving followed a symlink into the Anki profile"
    );
}

#[cfg(unix)]
#[test]
fn saving_cannot_create_a_missing_subdirectory_through_a_profile_symlink() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let alias = root.path().join("linked-361");
    std::os::unix::fs::symlink(profile, &alias).unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &alias.join("missing-957/nested-31"),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(profile),
        "saving created a directory through a profile symlink"
    );
}

#[test]
fn saving_cannot_write_into_the_anki_root() {
    let root = TempDir::new().unwrap();
    let anki = root.path().join("Anki2");
    let path = collection(&anki);
    let before = inventory(&anki);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &anki,
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(&anki),
        "saving changed the Anki root directory"
    );
}

#[test]
fn saving_cannot_create_a_sibling_inside_the_anki_root() {
    let root = TempDir::new().unwrap();
    let anki = root.path().join("Anki2");
    let path = collection(&anki);
    let before = inventory(&anki);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &anki.join("exports-493/nested-17"),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(&anki),
        "saving created a sibling directory inside the Anki root"
    );
}

#[test]
fn console_saving_into_the_profile_requires_a_different_output_directory() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let output = command(root.path(), profile)
        .args(["--collection"])
        .arg(&path)
        .arg("--save")
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.code() == Some(2)
            && doc["error"]["code"] == "usage"
            && doc["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("--out"))
            && before == inventory(profile),
        "unsafe output was accepted, lost its usage refusal or changed the profile: {doc}"
    );
}

#[test]
fn saving_cannot_reenter_the_profile_after_missing_parent_components() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(root.path());
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &root
            .path()
            .join("missing-619/..")
            .join(profile.file_name().unwrap()),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(root.path()),
        "saving traversed missing parents and changed the source tree"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_collection_cannot_hide_its_profile_from_saving() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let alias = root.path().join("linked-collection-197.anki2");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&alias, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        profile,
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_err() && before == inventory(profile),
        "a collection symlink hid the source profile from the output guard"
    );
}

#[test]
fn a_similarly_named_sibling_remains_a_valid_output_directory() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    let result = snapshot.save(
        &root.path().join("Private-profile-947-export/nested-23"),
        time::OffsetDateTime::from_unix_timestamp(1_700_600_000).unwrap(),
        "1.14.0",
    );
    assert!(
        result.is_ok() && before == inventory(profile),
        "saving refused a safe sibling or changed the profile"
    );
}

/// Open only the export terminal with isolated preferences and synthetic Anki files.
#[cfg(unix)]
fn terminal_command(root: &Path, out: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_kamishibai"));
    command
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .env("KAMISHIBAI_ANKI", root)
        .env("KAMISHIBAI_DATA", root.join("preferences"))
        .env("KAMISHIBAI_CACHE", root.join("cache"))
        .env("KAMISHIBAI_OUTPUT", root.join("packages"))
        .env("PATH", root.join("no-system-probes"))
        .args(["export", "--out"])
        .arg(out);
    command
}

/// Read painted terminal cells rather than matching fragments of ANSI repaint output.
#[cfg(unix)]
struct ExportTerminal {
    terminal: expectrl::session::OsSession,
    parser: termwiz::escape::parser::Parser,
    surface: termwiz::surface::Surface,
}

#[cfg(unix)]
impl ExportTerminal {
    /// Forward a physical key sequence to the export process.
    fn send(&mut self, keys: &str) -> Result<(), expectrl::Error> {
        use expectrl::Expect;
        self.terminal.send(keys)
    }

    /// Wait for a visible phrase with a deadline, reconstructing incremental repaints.
    fn expect(&mut self, phrase: &str) -> std::io::Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut bytes = [0_u8; 16384];
        loop {
            match self.terminal.try_read(&mut bytes) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        self.surface.screen_chars_to_string(),
                    ));
                }
                Ok(count) => {
                    for action in self.parser.parse_as_vec(&bytes[..count]) {
                        if let Some(change) = painted(action) {
                            self.surface.add_change(change);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            let screen = self.surface.screen_chars_to_string();
            if screen.contains(phrase) {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("missing {phrase:?} in painted screen:\n{screen}"),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Walk to a named visible deck instead of assuming its sorted position.
    fn focus(&mut self, name: &str) {
        let screen = self.surface.screen_chars_to_string();
        let index = screen
            .lines()
            .skip(3)
            .take(18)
            .filter(|line| line.contains(" card"))
            .position(|line| {
                line.trim_start_matches([' ', '✓'])
                    .strip_prefix(name)
                    .is_some_and(|tail| tail.starts_with(' '))
            })
            .expect("invariant: the requested deck is visible");
        for _ in 0..index {
            self.send("\x1b[B").unwrap();
        }
    }

    /// Wait for the process to release its terminal after exit or cancellation.
    fn finish(&mut self) -> Result<expectrl::Captures, expectrl::Error> {
        use expectrl::Expect;
        self.terminal.expect(expectrl::Eof)
    }
}

/// Apply the text, cursor and erase operations emitted by the Crossterm backend.
#[cfg(unix)]
fn painted(action: termwiz::escape::Action) -> Option<termwiz::surface::Change> {
    use termwiz::color::ColorAttribute;
    use termwiz::escape::csi::{Cursor, Edit, EraseInDisplay, EraseInLine};
    use termwiz::escape::{Action, CSI, ControlCode};
    use termwiz::surface::{Change, Position};
    match action {
        Action::Print(character) => Some(Change::Text(character.to_string())),
        Action::PrintString(text) => Some(Change::Text(text)),
        Action::Control(ControlCode::CarriageReturn) => Some(Change::Text("\r".into())),
        Action::Control(ControlCode::LineFeed) => Some(Change::Text("\n".into())),
        Action::CSI(CSI::Cursor(Cursor::Position { line, col })) => Some(Change::CursorPosition {
            x: Position::Absolute(usize::try_from(col.as_zero_based()).unwrap()),
            y: Position::Absolute(usize::try_from(line.as_zero_based()).unwrap()),
        }),
        Action::CSI(CSI::Edit(Edit::EraseInDisplay(EraseInDisplay::EraseDisplay))) => {
            Some(Change::ClearScreen(ColorAttribute::Default))
        }
        Action::CSI(CSI::Edit(Edit::EraseInDisplay(EraseInDisplay::EraseToEndOfDisplay))) => {
            Some(Change::ClearToEndOfScreen(ColorAttribute::Default))
        }
        Action::CSI(CSI::Edit(Edit::EraseInLine(EraseInLine::EraseToEndOfLine))) => {
            Some(Change::ClearToEndOfLine(ColorAttribute::Default))
        }
        _ => None,
    }
}

/// Start the isolated terminal with a bounded expectation timeout.
#[cfg(unix)]
fn export_terminal(root: &Path, out: &Path) -> ExportTerminal {
    let mut terminal = expectrl::Session::spawn(terminal_command(root, out)).unwrap();
    terminal.set_expect_timeout(Some(std::time::Duration::from_secs(15)));
    ExportTerminal {
        terminal,
        parser: termwiz::escape::parser::Parser::new(),
        surface: termwiz::surface::Surface::new(80, 24),
    }
}

#[test]
#[cfg(unix)]
fn export_terminal_cannot_write_before_confirmation() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let before = inventory(path.parent().unwrap());
    let out = root.path().join("not-created");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("save for kamishibai web").unwrap();
    let absent = !out.exists();
    terminal.send("\x1b").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.finish().unwrap();
    assert!(
        absent && !out.exists() && before == inventory(path.parent().unwrap()),
        "back or exit wrote an export or changed the profile"
    );
}

#[test]
#[cfg(unix)]
fn control_c_cannot_return_from_decks_to_profiles() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("not-created");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\x03").unwrap();
    assert!(
        terminal.finish().is_ok() && !out.exists(),
        "Ctrl+C returned to profiles or wrote output"
    );
}

#[test]
#[cfg(unix)]
fn control_c_cannot_return_from_summary_to_decks() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("not-created");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Enter] save").unwrap();
    terminal.send("\x03").unwrap();
    assert!(
        terminal.finish().is_ok() && !out.exists(),
        "Ctrl+C returned to decks or wrote output"
    );
}

#[test]
#[cfg(unix)]
fn a_profile_destination_cannot_offer_save_on_the_summary() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let profile = path.parent().unwrap();
    let before = inventory(profile);
    let mut terminal = export_terminal(root.path(), profile);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    let refused = terminal.expect("can't save inside the anki folder").is_ok();
    terminal.send("\x03").unwrap();
    terminal.finish().unwrap();
    assert!(
        refused && before == inventory(profile),
        "the summary offered saving inside the profile or changed Anki"
    );
}

#[test]
#[cfg(unix)]
fn cancelling_a_save_cannot_hide_cleanup_or_its_outcome() {
    let root = TempDir::new().unwrap();
    slow_collection(root.path());
    let out = root.path().join("export");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Enter] save").unwrap();
    terminal.send("\r").unwrap();
    staged(&out);
    terminal.send("\x03").unwrap();
    let cleanup = terminal.expect("removing the unfinished file").is_ok();
    let outcome = terminal.expect("export cancelled, nothing saved").is_ok();
    terminal.finish().unwrap();
    assert!(
        cleanup && outcome && fs::read_dir(out).unwrap().next().is_none(),
        "cancellation hid cleanup, lost the outcome or left an archive"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn saved_file_links_cannot_unpack_the_archive() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    collection(root.path());
    let probes = root.path().join("no-system-probes");
    fs::create_dir(&probes).unwrap();
    let log = root.path().join("opened");
    let executable = probes.join("open");
    fs::write(
        &executable,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let out = root.path().join("export");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Enter] save").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("import from kamishibai cli").unwrap();
    for (keys, count) in [
        ("\x1b[<0;9;4M\x1b[<0;9;4m", 1),
        ("\x1b[<0;9;5M\x1b[<0;9;5m", 2),
    ] {
        terminal.send(keys).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline
            && !fs::read_to_string(&log).is_ok_and(|text| text.lines().count() == count)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    terminal.send("\r").unwrap();
    terminal.finish().unwrap();
    let opened = fs::read_to_string(&log).unwrap_or_default();
    let archive = fs::read_dir(&out).unwrap().next().unwrap().unwrap().path();
    assert_eq!(
        opened,
        format!(
            "{}\n-R {}\n",
            out.canonicalize().unwrap().display(),
            archive.canonicalize().unwrap().display()
        ),
        "saved links ignored the labels or opened the ZIP instead of revealing it"
    );
}

#[test]
#[cfg(unix)]
fn export_terminal_saves_the_reviewed_decks_after_confirmation() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let before = inventory(path.parent().unwrap());
    let out = root.path().join("export");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.focus("French");
    terminal.send(" ").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("save for kamishibai web").unwrap();
    let absent = !out.exists();
    terminal.send("\r").unwrap();
    terminal.expect("your file is ready").unwrap();
    terminal.expect("import from kamishibai cli").unwrap();
    terminal.send("\r").unwrap();
    terminal.finish().unwrap();
    let file = fs::read_dir(&out).unwrap().next().unwrap().unwrap().path();
    let preview = kamishibai::anki::history::validate(&file).unwrap();
    let preview = serde_json::to_value(preview).unwrap();
    assert!(
        absent && preview["counts"]["cards"] == 1 && before == inventory(path.parent().unwrap()),
        "confirmed export ignored deck selection, saved early or changed Anki"
    );
}

#[test]
#[cfg(unix)]
fn export_terminal_rechecks_anki_after_the_user_closes_it() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let wal = path.with_file_name("collection.anki2-wal");
    fs::write(&wal, []).unwrap();
    let out = root.path().join("export");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("close anki first").unwrap();
    fs::remove_file(wal).unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.finish().unwrap();
    assert!(!out.exists(), "rechecking closed Anki wrote an export");
}

#[test]
fn deck_inspection_cannot_hide_non_kamishibai_decks() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE notetypes SET name='Unrelated model'", [])
        .unwrap();
    let before = inventory(root.path());
    let decks = kamishibai::anki::history::inspect(&path, &[], &Closed, "unknown").unwrap();
    let decks = serde_json::to_value(decks).unwrap();
    assert!(
        decks.as_array().unwrap().len() == 4
            && decks
                .as_array()
                .unwrap()
                .iter()
                .all(|deck| deck["selected"] == false)
            && before == inventory(root.path()),
        "inspection refused an empty default selection, hid available decks or wrote to Anki"
    );
}

#[test]
fn a_refused_save_gate_cannot_create_the_output_directory() {
    struct Refused;
    impl kamishibai::anki::history::SaveGate for Refused {
        fn verify(&self) -> Result<(), kamishibai::anki::history::HistoryError> {
            Err(kamishibai::anki::history::HistoryError::Open)
        }
    }
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let out = root.path().join("absent-output");
    let result = kamishibai::anki::history::read(&path, &[], &Closed, "unknown")
        .unwrap()
        .save_guarded(&out, time::OffsetDateTime::UNIX_EPOCH, "test", &Refused);
    assert!(
        matches!(result, Err(kamishibai::anki::history::HistoryError::Open)) && !out.exists(),
        "a failed source check created an output directory"
    );
}

#[test]
fn a_late_refusal_cannot_publish_or_leave_a_partial_file() {
    struct Changed(std::cell::Cell<bool>);
    impl kamishibai::anki::history::SaveGate for Changed {
        fn verify(&self) -> Result<(), kamishibai::anki::history::HistoryError> {
            if self.0.replace(true) {
                Err(kamishibai::anki::history::HistoryError::Changed)
            } else {
                Ok(())
            }
        }
    }
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let out = root.path().join("export");
    let result = kamishibai::anki::history::read(&path, &[], &Closed, "unknown")
        .unwrap()
        .save_guarded(
            &out,
            time::OffsetDateTime::UNIX_EPOCH,
            "test",
            &Changed(std::cell::Cell::new(false)),
        );
    assert!(
        matches!(
            result,
            Err(kamishibai::anki::history::HistoryError::Changed)
        ) && fs::read_dir(out).unwrap().next().is_none(),
        "a source change before publication left a ZIP or partial file"
    );
}

#[test]
#[cfg(unix)]
fn export_terminal_cannot_save_if_anki_reopens_after_review() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let out = root.path().join("absent-output");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("save for kamishibai web").unwrap();
    fs::write(path.with_file_name("collection.anki2-wal"), []).unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("close anki first").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.finish().unwrap();
    assert!(
        !out.exists(),
        "reopened Anki did not prevent export publication"
    );
}

#[test]
#[cfg(unix)]
fn export_terminal_cannot_save_a_changed_closed_collection() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let out = root.path().join("absent-output");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("save for kamishibai web").unwrap();
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE cards SET ivl=47 WHERE id=211", [])
        .unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("close anki first").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.finish().unwrap();
    assert!(
        !out.exists(),
        "a changed collection published a snapshot the user had not reviewed"
    );
}

#[test]
fn export_help_explains_the_screen_and_script_save() {
    let root = TempDir::new().unwrap();
    let output = command(root.path(), root.path())
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success()
            && help.contains("guided screen")
            && help.contains("--save")
            && help.contains("--collection"),
        "export help omitted the screen or its script interface"
    );
}

#[test]
#[cfg(unix)]
fn a_console_preview_cannot_hide_the_save_decision_in_technical_counts() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("not-created");
    let output = terminal_command(root.path(), &out).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success()
            && text.contains("cards     2\nreviews   3\nfolder    ")
            && text.contains("to save, run: kamishibai export --out ")
            && !text.contains("references")
            && !text.contains("mistakes")
            && !out.exists(),
        "the preview hid the save decision in technical counts or wrote output"
    );
}

#[test]
#[cfg(unix)]
fn a_saved_console_file_cannot_omit_the_web_language_choice() {
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("export");
    let output = terminal_command(root.path(), &out)
        .arg("--save")
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success() && text.contains(" · saved\n") && text.contains("file      ") && text.contains("1. choose your language pair\n  2. open your account\n  3. import from kamishibai cli"), "the console receipt lost its file or language-pair step");
}

#[test]
#[cfg(unix)]
fn export_terminal_cannot_be_blocked_by_an_unselected_stale_profile() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let other = root.path().join("Second-profile");
    fs::create_dir(&other).unwrap();
    fs::copy(path, other.join("collection.anki2")).unwrap();
    fs::write(other.join("collection.anki2-wal"), []).unwrap();
    let out = root.path().join("absent-output");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\x1b").unwrap();
    terminal.finish().unwrap();
    assert!(
        !out.exists(),
        "profile selection created output before confirmation"
    );
}

#[test]
#[cfg(unix)]
fn save_alone_bypasses_the_screen_even_inside_a_terminal() {
    use expectrl::Expect;
    let root = TempDir::new().unwrap();
    collection(root.path());
    let out = root.path().join("export");
    let mut command = terminal_command(root.path(), &out);
    command.arg("--save");
    let mut terminal = expectrl::Session::spawn(command).unwrap();
    terminal.set_expect_timeout(Some(std::time::Duration::from_secs(15)));
    terminal.expect("import from kamishibai cli").unwrap();
    terminal.expect(expectrl::Eof).unwrap();
    assert_eq!(
        fs::read_dir(out).unwrap().count(),
        1,
        "--save did not directly publish one archive in a terminal"
    );
}

/// Keep a synthetic console child owned until a bounded exit or test teardown.
#[cfg(unix)]
struct SavingExport(std::process::Child);

#[cfg(unix)]
impl Drop for SavingExport {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(unix)]
impl SavingExport {
    /// Wait for cleanup before inspecting output; kill only this child on timeout.
    fn finish(&mut self) -> (std::process::ExitStatus, Vec<u8>) {
        use std::io::Read;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                let mut bytes = Vec::new();
                self.0
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                return (status, bytes);
            }
            if std::time::Instant::now() >= deadline {
                panic!("export failed to exit before the cleanup deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// Stretch actual archive copying so an observed part precedes publication.
#[cfg(unix)]
fn slow_collection(root: &Path) -> PathBuf {
    let path = collection(root);
    fs::OpenOptions::new()
        .write(true)
        .open(path.parent().unwrap().join("collection.media/voice.wav"))
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE notes SET flds=replace(flds,'alias.wav','voice.wav')",
            [],
        )
        .unwrap();
    path
}

/// Wait for a real writer stage rather than delaying a presumed save window.
#[cfg(unix)]
fn staged(out: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if fs::read_dir(out).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "part"))
        }) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("export never staged its private archive");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Start the console writer with every storage root isolated and no provider key.
#[cfg(unix)]
fn saving(root: &Path, out: &Path, json: bool) -> SavingExport {
    let mut command = terminal_command(root, out);
    command.arg("--save");
    if json {
        command.arg("--json");
    }
    SavingExport(
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    )
}

/// Change a collection only after its snapshot has reached the archive writer.
#[cfg(unix)]
fn changed_save(json: bool) -> (bool, Value, bool) {
    let root = TempDir::new().unwrap();
    let path = slow_collection(root.path());
    let out = root.path().join("export");
    let mut child = saving(root.path(), &out, json);
    staged(&out);
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO revlog VALUES(1700100000999,211,1,1,1,2500,1000,1)",
            [],
        )
        .unwrap();
    let (status, bytes) = child.finish();
    let document = if json {
        serde_json::from_slice(&bytes).unwrap()
    } else {
        Value::Null
    };
    (
        status.success(),
        document,
        fs::read_dir(out).unwrap().next().is_none(),
    )
}

#[test]
#[cfg(unix)]
fn console_json_cannot_publish_a_collection_changed_during_save() {
    let (success, document, empty) = changed_save(true);
    assert!(
        !success
            && document["ok"] == false
            && document["error"]["message"]
                .as_str()
                .unwrap()
                .contains("changed")
            && empty,
        "console JSON published stale history or left a private part: {document}"
    );
}

#[test]
#[cfg(unix)]
fn console_text_cannot_publish_a_collection_changed_during_save() {
    let (success, _, empty) = changed_save(false);
    assert!(
        !success && empty,
        "console text published stale history or left a private part"
    );
}

/// Deliver one catchable signal after staging and inspect files only after exit.
#[cfg(unix)]
fn interrupted_save(signal: rustix::process::Signal) -> (bool, bool, bool) {
    let root = TempDir::new().unwrap();
    let path = slow_collection(root.path());
    let before = fs::read(&path).unwrap();
    let out = root.path().join("export");
    let mut child = saving(root.path(), &out, true);
    staged(&out);
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(i32::try_from(child.0.id()).unwrap()).unwrap(),
        signal,
    )
    .unwrap();
    let (status, bytes) = child.finish();
    let refused = serde_json::from_slice::<Value>(&bytes).is_ok_and(|doc| doc["ok"] == false);
    (
        !status.success() && refused,
        fs::read_dir(out).unwrap().next().is_none(),
        fs::read(path).unwrap() == before,
    )
}

#[test]
#[cfg(unix)]
fn sigint_cannot_leave_an_archive_during_save() {
    assert_eq!(
        interrupted_save(rustix::process::Signal::INT),
        (true, true, true),
        "SIGINT left an archive, reported success or changed Anki"
    );
}

#[test]
#[cfg(unix)]
fn sigterm_cannot_leave_an_archive_during_save() {
    assert_eq!(
        interrupted_save(rustix::process::Signal::TERM),
        (true, true, true),
        "SIGTERM left an archive, reported success or changed Anki"
    );
}

#[test]
#[cfg(unix)]
fn sighup_cannot_leave_an_archive_during_save() {
    assert_eq!(
        interrupted_save(rustix::process::Signal::HUP),
        (true, true, true),
        "SIGHUP left an archive, reported success or changed Anki"
    );
}

#[test]
#[cfg(unix)]
fn terminal_sighup_cannot_leave_an_archive_during_save() {
    let root = TempDir::new().unwrap();
    let path = slow_collection(root.path());
    let before = fs::read(&path).unwrap();
    let out = root.path().join("export");
    let mut terminal = export_terminal(root.path(), &out);
    terminal.expect("[Enter] choose decks").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("[Space] select").unwrap();
    terminal.send("\r").unwrap();
    terminal.expect("save for kamishibai web").unwrap();
    terminal.send("\r").unwrap();
    staged(&out);
    terminal
        .terminal
        .get_process_mut()
        .signal(expectrl::process::unix::Signal::SIGHUP)
        .unwrap();
    terminal.finish().unwrap();
    assert!(
        fs::read_dir(out).unwrap().next().is_none() && fs::read(path).unwrap() == before,
        "terminal SIGHUP exited before removing the private archive or changed Anki"
    );
}

/// Keep output as the controlling PTY while stdin belongs to a second PTY.
#[cfg(unix)]
fn split_terminal(root: &Path, out: &Path) -> (fs::File, ExportTerminal) {
    use rustix::fs::{Mode, OFlags, open};
    use rustix::pty::{grantpt, ptsname, unlockpt};
    let input = open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    grantpt(&input).unwrap();
    unlockpt(&input).unwrap();
    let path = ptsname(&input, Vec::new()).unwrap();
    let template = terminal_command(root, out);
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args([
            "-c",
            "exec \"$@\" < \"$KAMISHIBAI_TEST_INPUT\"",
            "export-input",
        ])
        .arg(template.get_program())
        .args(template.get_args());
    for (name, value) in template.get_envs() {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command.env("KAMISHIBAI_TEST_INPUT", path.to_str().unwrap());
    let mut terminal = expectrl::Session::spawn(command).unwrap();
    terminal.set_expect_timeout(Some(std::time::Duration::from_secs(3)));
    (
        input.into(),
        ExportTerminal {
            terminal,
            parser: termwiz::escape::parser::Parser::new(),
            surface: termwiz::surface::Surface::new(80, 24),
        },
    )
}

/// Walk the real screen using only the separate input terminal.
#[cfg(unix)]
fn split_save(input: &mut fs::File, terminal: &mut ExportTerminal) {
    use std::io::Write;
    for phrase in [
        "[Enter] choose decks",
        "[Space] select",
        "save for kamishibai web",
    ] {
        terminal.expect(phrase).unwrap();
        input.write_all(b"\r").unwrap();
    }
}

/// Confirm process death within the same EOF deadline instead of relying on PTY EOF.
#[cfg(unix)]
fn exited(
    terminal: &ExportTerminal,
    deadline: std::time::Instant,
) -> expectrl::process::unix::WaitStatus {
    loop {
        let status = terminal.terminal.get_process().status().unwrap();
        if !matches!(status, expectrl::process::unix::WaitStatus::StillAlive)
            || std::time::Instant::now() >= deadline
        {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
#[cfg(unix)]
fn input_eof_cannot_publish_or_keep_the_export_alive_during_save() {
    let root = TempDir::new().unwrap();
    let path = slow_collection(root.path());
    let before = fs::read(&path).unwrap();
    let out = root.path().join("export");
    let (mut input, mut terminal) = split_terminal(root.path(), &out);
    split_save(&mut input, &mut terminal);
    staged(&out);
    let started = std::time::Instant::now();
    drop(input);
    let ended = terminal.finish().is_ok();
    let status = exited(&terminal, started + std::time::Duration::from_secs(3));
    assert!(
        ended
            && started.elapsed() < std::time::Duration::from_secs(3)
            && matches!(status, expectrl::process::unix::WaitStatus::Exited(_, code) if code != 0)
            && fs::read_dir(out).unwrap().next().is_none()
            && fs::read(path).unwrap() == before,
        "input EOF left a live export, reported success, published an archive or changed Anki: {status:?}"
    );
}

#[test]
#[cfg(unix)]
fn input_eof_cannot_remove_an_already_published_export() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let before = inventory(path.parent().unwrap());
    let out = root.path().join("export");
    let (mut input, mut terminal) = split_terminal(root.path(), &out);
    split_save(&mut input, &mut terminal);
    terminal.expect("your file is ready").unwrap();
    let archive = fs::read_dir(&out).unwrap().next().unwrap().unwrap().path();
    let bytes = fs::read(&archive).unwrap();
    let started = std::time::Instant::now();
    drop(input);
    let ended = terminal.finish().is_ok();
    let status = exited(&terminal, started + std::time::Duration::from_secs(3));
    assert!(
        ended
            && matches!(status, expectrl::process::unix::WaitStatus::Exited(_, code) if code != 0)
            && fs::read(archive).unwrap() == bytes
            && fs::read_dir(out).unwrap().count() == 1
            && inventory(path.parent().unwrap()) == before,
        "input EOF kept the saved screen alive, reported success or changed the published export or Anki: {status:?}"
    );
}

#[test]
fn a_changed_snapshot_cannot_create_output_with_the_default_writer() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    Connection::open(path)
        .unwrap()
        .execute("UPDATE cards SET ivl=47 WHERE id=211", [])
        .unwrap();
    let out = root.path().join("absent-output");
    let result = snapshot.save(&out, time::OffsetDateTime::UNIX_EPOCH, "test");
    assert!(
        matches!(
            result,
            Err(kamishibai::anki::history::HistoryError::Changed)
        ) && !out.exists(),
        "the default writer bypassed source verification or created output"
    );
}

#[test]
fn a_reopened_snapshot_cannot_create_output_with_the_default_writer() {
    let root = TempDir::new().unwrap();
    let path = collection(root.path());
    let snapshot = kamishibai::anki::history::read(&path, &[], &Closed, "unknown").unwrap();
    fs::write(path.with_file_name("collection.anki2-wal"), []).unwrap();
    let out = root.path().join("absent-output");
    let result = snapshot.save(&out, time::OffsetDateTime::UNIX_EPOCH, "test");
    assert!(
        matches!(result, Err(kamishibai::anki::history::HistoryError::Open)) && !out.exists(),
        "the default writer bypassed the closed-Anki check or created output"
    );
}
