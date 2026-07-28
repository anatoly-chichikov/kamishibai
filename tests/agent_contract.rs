//! Binary-level checks for the embedded, version-matched agent contract.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;

/// Return the checked-in contract path.
fn contract_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("llms.txt")
}

/// Read the checked-in agent contract.
fn contract() -> String {
    fs::read_to_string(contract_path()).expect("agent contract must exist")
}

/// Extract the application release declared by the contract.
fn release(document: &str) -> &str {
    document
        .lines()
        .find_map(|line| line.strip_prefix("Release: "))
        .expect("agent contract must declare its release")
}

#[test]
fn agent_contract_prints_the_exact_embedded_document() {
    let output = Command::cargo_bin("kamishibai")
        .expect("the binary must build")
        .arg("agent-contract")
        .output()
        .expect("agent-contract must run");
    assert_eq!(
        (
            output.status.success(),
            String::from_utf8(output.stdout).expect("contract output must be UTF-8"),
        ),
        (true, contract()),
        "agent-contract no longer prints the exact checked-in contract"
    );
}

#[test]
fn agent_contract_release_matches_the_package_version() {
    assert_eq!(
        release(contract().as_str()),
        env!("CARGO_PKG_VERSION"),
        "llms.txt Release no longer matches the application version"
    );
}

#[test]
fn agent_contract_remote_url_is_release_pinned() {
    let expected = format!(
        "https://raw.githubusercontent.com/anatoly-chichikov/kamishibai/v{}/llms.txt",
        env!("CARGO_PKG_VERSION")
    );
    let document = contract();
    assert!(
        document.contains(expected.as_str()) && !document.contains("kamishibai/main/llms.txt"),
        "llms.txt no longer points agents to its matching release tag"
    );
}

#[test]
fn root_help_recommends_the_local_contract_before_the_remote_copy() {
    let output = Command::cargo_bin("kamishibai")
        .expect("the binary must build")
        .arg("--help")
        .output()
        .expect("root help must run");
    let help = String::from_utf8(output.stdout).expect("root help must be UTF-8");
    let local = help.find("kamishibai agent-contract");
    let remote = help.find(format!("kamishibai/v{}/llms.txt", env!("CARGO_PKG_VERSION")).as_str());
    assert!(
        local.is_some() && remote.is_some() && local < remote,
        "root help no longer recommends the local contract before the release-pinned copy"
    );
}

#[test]
fn agent_contract_refuses_json_in_json() {
    let output = Command::cargo_bin("kamishibai")
        .expect("the binary must build")
        .args(["agent-contract", "--json"])
        .output()
        .expect("agent-contract refusal must run");
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("refusal must be JSON");
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["exit"].as_u64(),
        ),
        (Some(2), Some("usage"), Some(2)),
        "agent-contract --json no longer returns a usage envelope"
    );
}

#[test]
fn clap_usage_failures_remain_machine_readable_in_json_mode() {
    let output = Command::cargo_bin("kamishibai")
        .expect("the binary must build")
        .args(["new", "--json"])
        .output()
        .expect("the malformed command must run");
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("usage refusal must be JSON");
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["exit"].as_u64(),
            document["error"]["hint"].is_null(),
            document["error"]["retryable"].as_bool(),
        ),
        (Some(2), Some("usage"), Some(2), true, Some(false)),
        "clap failures bypassed the JSON error contract"
    );
}
