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
use expectrl::{Expect, Session, spawn};

fn example_binary(name: &str) -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--example", name, "--quiet"])
        .status()
        .expect("cargo build must succeed");
    assert!(status.success(), "{name} example must build");
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("examples")
        .join(name)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn pty_flow_advances_from_your_words_to_what_i_understood() {
    let binary = example_binary("tui_skeleton");
    let mut session =
        spawn(binary.to_str().expect("path must be utf-8")).expect("spawn must succeed");
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session
        .expect("[screen] Your words")
        .expect("skeleton must render Your words on launch");
    session.send("a").expect("must seed blob character");
    session.send("\x07\r").expect("must send Ctrl+G");
    session
        .expect("[screen] What I understood")
        .expect("skeleton must advance to What I understood after Ctrl+G");
    session.send_line("q").expect("must send quit key");
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !session.is_alive().unwrap_or(true),
        "skeleton process must exit after receiving q"
    );
}

#[test]
fn pty_state_demo_switches_mouse_pointer_between_link_and_plain_cells() {
    let binary = example_binary("tui_states");
    let mut command = Command::new(binary);
    command.env("TERM_PROGRAM", "iTerm.app");
    let mut session = Session::spawn(command).expect("spawn must succeed");
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    let coordinate_mode = session
        .expect(b"\x1b[?1006h".as_slice())
        .expect("state demo must enable SGR mouse coordinates");
    let movement_mode = session
        .expect(b"\x1b[?1003h".as_slice())
        .expect("state demo must enable all-motion mouse reporting");
    let launch_pointer = session
        .expect(b"\x1b]22;left_ptr\x1b\\".as_slice())
        .expect("state demo must set the iTerm arrow pointer on launch");
    let click_only = b"\x1b[?1002h";
    let enabled_click_only = contains_bytes(coordinate_mode.as_bytes(), click_only)
        || contains_bytes(movement_mode.as_bytes(), click_only)
        || contains_bytes(launch_pointer.as_bytes(), click_only);
    session
        .send("\x1b[<35;1;1M")
        .expect("must send mouse move over plain launch cell");
    session
        .expect(b"\x1b]22;left_ptr\x1b\\".as_slice())
        .expect("state demo must reassert the iTerm arrow over plain cells on every move");
    session
        .expect(b"\x1b]22;left_ptr\x1b\\".as_slice())
        .expect("state demo must keep reasserting the iTerm arrow while the pointer is stationary");
    for _ in 0..5 {
        session.send(" ").expect("must advance one state");
        std::thread::sleep(Duration::from_millis(150));
    }
    session
        .send("\x1b[<35;20;6M")
        .expect("must send mouse move over artifact file name");
    session
        .expect(b"\x1b]22;hand2\x1b\\".as_slice())
        .expect("state demo must switch to the iTerm hand pointer over a file-backed artifact row");
    session
        .send("\x1b[<35;1;1M")
        .expect("must send mouse move over plain cell");
    session
        .expect(b"\x1b]22;left_ptr\x1b\\".as_slice())
        .expect("state demo must switch back to the iTerm arrow over plain cells");
    for _ in 0..4 {
        session.send(" ").expect("must advance one state");
        std::thread::sleep(Duration::from_millis(150));
    }
    session
        .send("\x1b[<35;9;5M")
        .expect("must send mouse move over done artifact placeholder");
    session
        .expect(b"\x1b]22;hand2\x1b\\".as_slice())
        .expect("state demo must switch to the iTerm hand pointer over a done placeholder");
    let exited = session
        .get_process_mut()
        .exit(true)
        .expect("state demo must terminate");
    assert!(
        exited && !enabled_click_only,
        "state demo process must exit after pointer verification and must use all-motion reporting instead of click-only reporting"
    );
}
