//! Exit-code coverage for the startup path, run against the real binary.
//!
//! Every case controls `PATH` itself: `--help`/`-V` get an empty directory, and the file
//! errors get a directory holding an inert placeholder named like the harness. That way
//! the tests never depend on `claude` or `pi` being installed, and nothing is spawned —
//! the binary exits (or would exit) before reaching the event loop.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_prosecraft");

/// Removed on drop, so a failing assertion cannot leave sandboxes behind.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "prosecraft-it-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    /// An empty directory: `PATH` here resolves no harness at all.
    fn empty(tag: &str) -> Self {
        Self::new(tag)
    }

    /// A directory that satisfies the harness lookup without a real harness in it.
    fn with_placeholder_harness(tag: &str) -> Self {
        let dir = Self::new(tag);
        let placeholder = dir.0.join("claude");
        fs::write(&placeholder, "not a real harness; never executed").expect("write placeholder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&placeholder, fs::Permissions::from_mode(0o755))
                .expect("mark placeholder executable");
        }
        dir
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], path: &Path) -> Output {
    Command::new(BIN)
        .args(args)
        .env("PATH", path)
        .output()
        .expect("run prosecraft")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_exits_zero_with_no_harness_on_path() {
    let path = TempDir::empty("help");
    let output = run(&["--help"], path.path());

    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).is_empty(), "help printed to stderr");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--harness"), "no flags in help: {stdout}");
}

#[test]
fn version_exits_zero_with_no_harness_on_path() {
    let path = TempDir::empty("version");
    let output = run(&["-V"], path.path());

    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).is_empty(), "version printed to stderr");
    assert!(String::from_utf8_lossy(&output.stdout).contains("prosecraft"));
}

#[test]
fn unknown_harness_exits_two_with_usage() {
    let path = TempDir::empty("bad-harness");
    let output = run(&["--harness", "nope"], path.path());

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("nope"), "value not named: {message}");
    assert!(message.contains("Usage"), "no usage line: {message}");
}

#[test]
fn prompt_and_prompt_file_exit_two() {
    let path = TempDir::empty("conflict");
    let output = run(
        &["--harness", "claude", "--prompt", "x", "--prompt-file", "y"],
        path.path(),
    );

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("--prompt"), "flags not named: {message}");
}

#[test]
fn missing_harness_exits_one_with_one_stderr_line_and_no_terminal_output() {
    let path = TempDir::empty("missing-harness");
    let output = run(&["--harness", "claude"], path.path());

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert_eq!(message.lines().count(), 1, "not one line: {message:?}");
    assert!(message.contains("claude"), "binary not named: {message}");
    // Alternate-screen setup writes escape codes to stdout; an empty stdout is the
    // observable proof that raw mode and the first draw were never reached.
    assert!(output.stdout.is_empty(), "terminal was touched");
}

#[test]
fn missing_text_file_exits_one_with_a_clear_message() {
    let path = TempDir::with_placeholder_harness("missing-text-file");
    let output = run(
        &[
            "--harness",
            "claude",
            "--text-file",
            "missing.txt",
            "--no-clipboard",
        ],
        path.path(),
    );

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("--text-file"), "flag missing: {message}");
    assert!(message.contains("missing.txt"), "path missing: {message}");
    assert!(output.stdout.is_empty(), "terminal was touched");
}

#[test]
fn missing_prompt_file_exits_one_with_a_clear_message() {
    let path = TempDir::with_placeholder_harness("missing-prompt-file");
    let output = run(
        &[
            "--harness",
            "claude",
            "--prompt-file",
            "missing.txt",
            "--no-clipboard",
        ],
        path.path(),
    );

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("--prompt-file"), "flag missing: {message}");
    assert!(message.contains("missing.txt"), "path missing: {message}");
    assert!(output.stdout.is_empty(), "terminal was touched");
}
