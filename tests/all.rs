//! Single integration-test harness: every sibling test file is one module,
//! so the whole suite links once instead of once per file. `snapshot.rs`
//! stays its own target to keep insta snapshot filenames stable. Run one
//! module with `cargo test --test all your_cards::`.

mod agent_contract;
mod agent_onboarding;
mod anki;
mod assets;
mod audio;
mod batch_sentence_settings;
mod cache;
mod change_something;
mod config;
mod config_cli;
mod done;
mod failure_recovery;
mod gemini;
mod illustration;
mod input;
mod keyboard;
mod language_context;
mod language_flow;
mod learning_target;
mod new_languages;
mod paths;
mod profile;
mod pty;
mod reference_contract;
mod report;
mod retry_inline;
mod scene;
mod sentence_labels;
mod sentence_labels_editor;
mod sentence_settings;
mod separation;
mod session;
mod session_contracts;
mod session_engine;
mod sessions;
mod state_machine;
mod ui_contract;
mod what_i_understood;
mod your_cards;
mod your_words;

/// Guard the harness roster: with `autotests = false` a test file missing
/// from the module list above would silently stop running.
#[test]
fn every_sibling_test_file_is_declared_in_the_harness() {
    let harness = include_str!("all.rs");
    let missing: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"))
        .expect("the tests directory must be readable")
        .filter_map(|entry| {
            let name = entry.expect("directory entry must be readable").file_name();
            let name = name.to_string_lossy().into_owned();
            let stem = name.strip_suffix(".rs")?.to_string();
            (stem != "all" && stem != "snapshot" && !harness.contains(&format!("mod {stem};")))
                .then_some(name)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "test files never run because the harness does not declare them: {missing:?}"
    );
}
