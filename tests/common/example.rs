//! Repository examples built in the running test's own Cargo profile and run within a deadline.
//!
//! Each launch runs in its own process group on Unix. One deadline covers the process and the
//! reading of its output; once it passes, the whole group is killed so no descendant outlives it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

/// One example binary Cargo built for this test run; build it with `built` and launch it with `command` or `printed`.
pub(crate) struct Example {
    binary: PathBuf,
}

impl Example {
    /// Build the example `name` in the profile of the running test and locate it through Cargo's artifact message.
    pub(crate) fn built(name: &str) -> Self {
        let mut command = Command::new(env!("CARGO"));
        command.current_dir(env!("CARGO_MANIFEST_DIR")).args([
            "build",
            "--example",
            name,
            "--profile",
            &profile(),
            "--message-format=json",
            "--offline",
            "--locked",
            "--quiet",
        ]);
        let output = bounded(command, Duration::from_secs(600))
            .unwrap_or_else(|error| panic!("example {name} could not be built: {error:#}"));
        if !output.status.success() {
            panic!(
                "example {name} failed to build: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self {
            binary: executable(&String::from_utf8_lossy(&output.stdout), name),
        }
    }

    /// A fresh command that launches this example; callers add arguments and environment.
    #[must_use]
    pub(crate) fn command(&self) -> Command {
        Command::new(&self.binary)
    }

    /// Run the example with `arguments` and return its standard output, killing it after `timeout`.
    #[must_use]
    pub(crate) fn printed(&self, arguments: &[&str], timeout: Duration) -> String {
        let mut command = self.command();
        command.args(arguments);
        let output = bounded(command, timeout).unwrap_or_else(|error| {
            panic!(
                "example {} {arguments:?} did not finish: {error:#}",
                self.binary.display()
            )
        });
        if !output.status.success() {
            panic!(
                "example {} {arguments:?} failed: {}",
                self.binary.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8(output.stdout).expect("invariant: examples print UTF-8")
    }
}

fn profile() -> String {
    let executable =
        std::env::current_exe().expect("invariant: running test executable must have a path");
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("invariant: test executable must live in the deps directory of its profile");
    named(directory)
}

fn named(directory: &str) -> String {
    match directory {
        "debug" => String::from("dev"),
        profile => profile.to_owned(),
    }
}

fn executable(messages: &str, name: &str) -> PathBuf {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == name
                && message["target"]["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "example"))
        })
        .and_then(|message| message["executable"].as_str().map(PathBuf::from))
        .expect("invariant: cargo must report the executable of the example it built")
}

fn bounded(mut command: Command, timeout: Duration) -> Result<Output> {
    let deadline = Instant::now() + timeout;
    isolated(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("invariant: child process must be launchable");
    let stdout = drained(child.stdout.take());
    let stderr = drained(child.stderr.take());
    let status = reaped(&mut child, deadline);
    let stdout = collected(&stdout, deadline);
    let stderr = collected(&stderr, deadline);
    let (Some(status), Some(stdout), Some(stderr)) = (status, stdout, stderr) else {
        terminate(&mut child);
        bail!(
            "process {} or a descendant holding its output outlived the {timeout:?} deadline",
            child.id()
        );
    };
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn drained(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    });
    receiver
}

fn collected(output: &Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    output
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}

fn reaped(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        if let Some(status) = child
            .try_wait()
            .expect("invariant: child status must be readable")
        {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn isolated(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn isolated(_command: &mut Command) {}

#[cfg(unix)]
fn terminate(child: &mut Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
    let _ = child.wait();
}

#[cfg(not(unix))]
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    use super::bounded;
    use super::{executable, named};

    #[cfg(unix)]
    fn launcher(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    fn gone(record: &Path, patience: Duration) -> bool {
        let raw = std::fs::read_to_string(record)
            .expect("invariant: the launcher must record its descendant")
            .trim()
            .parse::<i32>()
            .expect("invariant: the record must hold a process ID");
        let pid = rustix::process::Pid::from_raw(raw)
            .expect("invariant: a recorded process ID must be positive");
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            if rustix::process::test_kill_process(pid).is_err() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn a_descendant_holding_the_output_cannot_outlast_the_deadline() {
        let started = Instant::now();
        let outcome = bounded(launcher("sleep 5 &"), Duration::from_millis(200));
        assert!(
            outcome.is_err() && started.elapsed() < Duration::from_secs(2),
            "a background descendant kept the output open past the deadline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_deadline_cannot_leave_a_descendant_running() {
        let directory = tempfile::tempdir().expect("invariant: temporary directory must exist");
        let record = directory.path().join("descendant.pid");
        let script = format!("sleep 5 & echo $! > '{}'; wait", record.display());
        let _ = bounded(launcher(&script), Duration::from_millis(300));
        assert!(
            gone(&record, Duration::from_secs(2)),
            "the deadline stopped the launcher but left its descendant running"
        );
    }

    #[test]
    fn the_debug_directory_cannot_be_passed_to_cargo_as_a_profile_name() {
        assert_eq!(
            named("debug"),
            "dev",
            "the debug output directory was handed to cargo as a profile name"
        );
    }

    #[test]
    fn a_custom_profile_cannot_lose_its_own_directory_name() {
        assert_eq!(
            named("release"),
            "release",
            "a named profile was rewritten instead of reused"
        );
    }

    #[test]
    fn the_example_path_cannot_come_from_another_artifact() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"tui_states","kind":["lib"]},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"kamishibai","kind":["bin"]},"executable":"/target/release/kamishibai"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"tui_states","kind":["example"]},"executable":"/target/release/examples/tui_states"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ]
        .join("\n");
        assert_eq!(
            executable(&messages, "tui_states"),
            PathBuf::from("/target/release/examples/tui_states"),
            "the example path was taken from an artifact that is not the example"
        );
    }
}
