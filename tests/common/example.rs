//! Repository examples built in the running test's own Cargo profile and run within a deadline.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
        let output = bounded(command, Duration::from_secs(600));
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
        let output = bounded(command, timeout);
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

fn bounded(mut command: Command, timeout: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("invariant: child process must be launchable");
    let stdout = drained(child.stdout.take());
    let stderr = drained(child.stderr.take());
    let status = reaped(&mut child, timeout);
    Output {
        status,
        stdout: stdout
            .join()
            .expect("invariant: stdout reader must not panic"),
        stderr: stderr
            .join()
            .expect("invariant: stderr reader must not panic"),
    }
}

fn drained(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

fn reaped(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .expect("invariant: child status must be readable")
        {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "child process {} outlived its {timeout:?} deadline",
                child.id()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{executable, named};

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
