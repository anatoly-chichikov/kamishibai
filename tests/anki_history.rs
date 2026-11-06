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
        .env("PATH", root.join("no-system-probes"))
        .args(["import", "anki", "--out"])
        .arg(out)
        .arg("--json");
    command.timeout(std::time::Duration::from_secs(30));
    command
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
