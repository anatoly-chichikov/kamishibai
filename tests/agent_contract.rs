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

fn bash_lines(document: &str) -> Vec<String> {
    let mut bash = false;
    let mut pending = String::new();
    let mut lines = Vec::new();
    for line in document.lines() {
        let trimmed = line.trim();
        if trimmed == "```bash" {
            bash = true;
            continue;
        }
        if trimmed == "```" && bash {
            bash = false;
            continue;
        }
        if !bash || trimmed.is_empty() {
            continue;
        }
        if let Some(part) = trimmed.strip_suffix('\\') {
            pending.push_str(part.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(trimmed);
        lines.push(std::mem::take(&mut pending));
    }
    lines
}

fn command_argv(line: &str) -> Result<Option<Vec<String>>, String> {
    let start = line.match_indices("kamishibai").find_map(|(start, name)| {
        let before = line[..start].chars().next_back();
        let after = line[start + name.len()..].chars().next();
        let begins = match before {
            Some(character) => !character.is_alphanumeric() && character != '_' && character != '-',
            None => true,
        };
        let ends = after.is_none() || after.is_some_and(char::is_whitespace);
        (begins && ends).then_some(start)
    });
    let Some(start) = start else {
        return Ok(None);
    };
    let words = shlex::split(&line[start..]).ok_or_else(|| line.to_owned())?;
    let argv = words
        .into_iter()
        .take_while(|word| !word.starts_with(['|', '>', '<', ';', '&']))
        .collect::<Vec<_>>();
    Ok(Some(argv))
}

#[test]
fn every_contract_bash_command_parses_with_the_binary_grammar() {
    let commands = bash_lines(include_str!("../llms.txt"))
        .into_iter()
        .filter_map(|line| command_argv(&line).transpose().map(|argv| (line, argv)))
        .collect::<Vec<_>>();
    let failures = commands
        .iter()
        .filter(|(_, argv)| match argv {
            Ok(argv) => kamishibai::cli::command()
                .try_get_matches_from(argv)
                .is_err(),
            Err(_) => true,
        })
        .map(|(line, _)| line)
        .collect::<Vec<_>>();
    assert!(
        commands.len() == 14 && failures.is_empty(),
        "documented bash commands no longer parse or one was skipped: found {}, failures: {failures:?}",
        commands.len()
    );
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
