//! PTY-level end-to-end flow using `expectrl`.
//!
//! Spawns the `tui_skeleton` example in a real pseudoterminal, types the
//! locked-in keyboard contract, and asserts on the plain-text screen markers
//! it prints. No LLM calls — the skeleton never reaches the generation phase.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use expectrl::process::Healthcheck;
use expectrl::{Expect, spawn};

fn example_binary() -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--example", "tui_skeleton", "--quiet"])
        .status()
        .expect("cargo build must succeed");
    assert!(status.success(), "tui_skeleton example must build");
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("examples")
        .join("tui_skeleton")
}

#[test]
fn pty_flow_advances_from_your_words_to_what_i_understood() {
    let binary = example_binary();
    let mut session =
        spawn(binary.to_str().expect("path must be utf-8")).expect("spawn must succeed");
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session
        .expect("[screen] Your words")
        .expect("skeleton must render Your words on launch");
    session.send("\r").expect("must send Enter");
    session
        .expect("[screen] What I understood")
        .expect("skeleton must advance to What I understood after Enter");
    session.send_line("q").expect("must send quit key");
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !session.is_alive().unwrap_or(true),
        "skeleton process must exit after receiving q"
    );
}
