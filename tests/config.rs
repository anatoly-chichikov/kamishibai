//! Persisted user preference round-trip.

use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use kamishibai::config::{PreferenceStore, Preferences};
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::process::Command;

#[test]
fn preference_store_persists_my_language_across_reads() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("ru"))
        .expect("write must succeed");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        restored,
        Preferences::new("ru"),
        "persisted my_language must survive a round trip"
    );
}

#[test]
fn preference_store_reports_default_english_on_first_run() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("missing.json"));
    let fresh = store.read().expect("read must succeed");
    assert_eq!(
        fresh,
        Preferences::default(),
        "missing preference file must collapse to English default"
    );
}

#[test]
fn stored_api_key_does_not_confirm_the_default_language() {
    let preferences = Preferences::default().with_api_key("123456789012345678901234567890");
    assert!(
        preferences.requires_language_choice(),
        "saving an API key alone must not mark the language as user-confirmed"
    );
}

#[test]
fn clearing_api_key_preserves_the_confirmed_language() {
    let preferences = Preferences::new("ru")
        .with_api_key("123456789012345678901234567890")
        .without_api_key();
    assert_eq!(
        (
            preferences.my_language,
            preferences.my_language_confirmed,
            preferences.api_key,
        ),
        (String::from("ru"), true, None),
        "clearing a rejected API key must not reset the confirmed language"
    );
}

#[test]
fn legacy_preference_without_confirmation_cannot_silently_pick_german() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path().parent().expect("store must have parent"))
        .expect("parent must be writable");
    fs::write(
        store.path(),
        r#"{"my_language":"de","api_key":"123456789012345678901234567890"}"#,
    )
    .expect("legacy preference must be writable");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        (
            restored.requires_language_choice(),
            restored.startup_language().to_string(),
        ),
        (true, String::from("en")),
        "legacy preferences without confirmation must not silently select German"
    );
}

#[test]
fn preferences_default_uses_english() {
    assert_eq!(
        Preferences::default().my_language,
        "en",
        "first-run my_language must default to English"
    );
}

#[test]
fn preferences_debug_redacts_the_saved_key() {
    let rendered = format!(
        "{:?}",
        Preferences::new("en").with_api_key("debug-secret-preference")
    );
    assert_eq!(
        (
            rendered.contains("debug-secret-preference"),
            rendered.contains("[REDACTED]")
        ),
        (false, true),
        "Preferences Debug exposed the saved API key"
    );
}

#[test]
fn a_known_language_update_preserves_the_saved_key() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("en").with_api_key("saved-secret"))
        .expect("seed preferences must succeed");
    store
        .update(|preferences| preferences.adopt("ru"))
        .expect("language update must succeed");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        (restored.my_language.as_str(), restored.api_key.as_deref()),
        ("ru", Some("saved-secret")),
        "updating the known language must not erase a saved key"
    );
}

#[test]
fn concurrent_updates_preserve_independent_fields() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    let barrier = Arc::new(Barrier::new(3));
    let language_store = store.clone();
    let language_barrier = Arc::clone(&barrier);
    let language = thread::spawn(move || {
        language_barrier.wait();
        language_store.update(|preferences| {
            thread::sleep(Duration::from_millis(25));
            preferences.adopt("ru")
        })
    });
    let key_store = store.clone();
    let key_barrier = Arc::clone(&barrier);
    let key = thread::spawn(move || {
        key_barrier.wait();
        key_store.update(|preferences| preferences.with_api_key("concurrent-secret"))
    });
    barrier.wait();
    language
        .join()
        .expect("language thread must finish")
        .expect("language update must succeed");
    key.join()
        .expect("key thread must finish")
        .expect("key update must succeed");
    let restored = store.read().expect("read must succeed");
    assert_eq!(
        (restored.my_language.as_str(), restored.api_key.as_deref()),
        ("ru", Some("concurrent-secret")),
        "serialized updates lost one independently changed field"
    );
}

#[test]
fn corrupt_preferences_are_not_first_run_defaults() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path().parent().expect("store must have parent"))
        .expect("parent must be writable");
    fs::write(store.path(), "{broken").expect("corrupt preferences must be writable");
    let error = store.read().expect_err("corrupt preferences must fail");
    assert!(
        error
            .to_string()
            .contains(store.path().to_string_lossy().as_ref())
            && !error.hint().is_empty(),
        "corrupt preferences must report their path and recovery action"
    );
}

#[test]
fn a_directory_at_the_preferences_path_is_an_error() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path()).expect("directory collision must be writable");
    assert!(
        store.read().is_err(),
        "a directory at preferences.json must not collapse to default preferences"
    );
}

#[test]
fn a_failed_locked_write_preserves_the_previous_file() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("en").with_api_key("original-secret"))
        .expect("seed preferences must succeed");
    let lock = store.path().with_extension("lock");
    fs::remove_file(lock.as_path()).expect("seed lock must be removable");
    fs::create_dir(lock.as_path()).expect("lock collision must be creatable");
    let failed = store.write(&Preferences::new("ru").with_api_key("replacement-secret"));
    let restored = store
        .read()
        .expect("previous preferences must remain readable");
    assert_eq!(
        (
            failed.is_err(),
            restored.my_language.as_str(),
            restored.api_key.as_deref()
        ),
        (true, "en", Some("original-secret")),
        "a failed write must leave the previous valid preferences unchanged"
    );
}

#[cfg(unix)]
#[test]
fn preference_files_and_directories_use_private_permissions() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("en").with_api_key("saved-secret"))
        .expect("preferences must be writable");
    let directory = fs::metadata(store.path().parent().expect("store must have parent"))
        .expect("directory metadata must be readable")
        .permissions()
        .mode()
        & 0o777;
    let file = fs::metadata(store.path())
        .expect("file metadata must be readable")
        .permissions()
        .mode()
        & 0o777;
    let lock = fs::metadata(store.path().with_extension("lock"))
        .expect("lock metadata must be readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        (directory, file, lock),
        (0o700, 0o600, 0o600),
        "preferences data, file, and lock permissions are not private"
    );
}

#[cfg(unix)]
#[test]
fn reading_repairs_legacy_world_readable_permissions() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path().parent().expect("store must have parent"))
        .expect("parent must be writable");
    fs::write(
        store.path(),
        serde_json::to_vec(&Preferences::new("en").with_api_key("legacy-secret"))
            .expect("preferences must serialize"),
    )
    .expect("legacy preferences must be writable");
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644))
        .expect("legacy permissions must be set");
    store
        .read()
        .expect("legacy preferences must remain readable");
    let mode = fs::metadata(store.path())
        .expect("file metadata must be readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "a successful access must repair legacy world-readable preferences"
    );
}

#[cfg(windows)]
#[test]
fn legacy_windows_preferences_are_stamped_after_first_read() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    fs::create_dir_all(store.path().parent().expect("store must have parent"))
        .expect("parent must be writable");
    fs::write(
        store.path(),
        serde_json::to_vec(&Preferences::new("en").with_api_key("legacy-secret"))
            .expect("preferences must serialize"),
    )
    .expect("legacy preferences must be writable");
    let restored = store.read().expect("legacy preferences must migrate");
    let stored: serde_json::Value = serde_json::from_slice(
        fs::read(store.path())
            .expect("migrated preferences must be readable")
            .as_slice(),
    )
    .expect("migrated preferences must remain JSON");
    assert_eq!(
        (
            restored.api_key.as_deref(),
            stored["windows_acl_version"].as_u64(),
        ),
        (Some("legacy-secret"), Some(1)),
        "legacy Windows preferences returned before recording ACL migration"
    );
}

#[cfg(windows)]
#[test]
fn preferences_use_a_verified_private_windows_acl() {
    let home = tempdir().expect("must create temp home");
    let store = PreferenceStore::at(home.path().join("kamishibai").join("preferences.json"));
    store
        .write(&Preferences::new("en").with_api_key("saved-secret"))
        .expect("preferences with a private ACL must be writable");
    let script = r#"
$ErrorActionPreference = 'Stop'
$target = [Environment]::GetEnvironmentVariable('KAMISHIBAI_ACL_TEST_TARGET')
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$administrators = [System.Security.Principal.SecurityIdentifier]::new([System.Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null)
function Test-Private([System.Security.AccessControl.FileSystemSecurity]$acl) {
    if (-not $acl.AreAccessRulesProtected) { return $false }
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier])
    if (($owner.Value -ne $sid.Value) -and ($owner.Value -ne $administrators.Value)) { return $false }
    $rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
    if ($rules.Count -ne 1) { return $false }
    $rule = $rules[0]
    $full = [System.Security.AccessControl.FileSystemRights]::FullControl
    return $rule.IdentityReference.Value -eq $sid.Value -and $rule.AccessControlType -eq [System.Security.AccessControl.AccessControlType]::Allow -and ($rule.FileSystemRights -band $full) -eq $full
}
$file = [System.IO.FileInfo]::new($target).GetAccessControl()
$parent = [System.IO.Path]::GetDirectoryName($target)
$directory = [System.IO.DirectoryInfo]::new($parent).GetAccessControl()
if ((Test-Private $file) -and (Test-Private $directory)) { [Console]::Out.WriteLine('private') } else { exit 30 }
"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("KAMISHIBAI_ACL_TEST_TARGET", store.path())
        .env("PSModulePath", "")
        .output()
        .expect("Windows ACL verification must run");
    assert_eq!(
        (
            output.status.success(),
            String::from_utf8_lossy(output.stdout.as_slice())
                .trim()
                .to_string()
        ),
        (true, String::from("private")),
        "preferences or their directory allow an unverified Windows principal"
    );
}
