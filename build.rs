//! Stamps the revision the binary was built from into `--version`.
//!
//! Build time only: the script shells out to `git` and records the answer in the
//! environment, so the crate keeps its "no filesystem side effects at runtime" promise.
//! A missing `git`, or a build outside a checkout, is not an error — the stamp falls
//! back to a constant so `cargo build`, `make check` and `--version` all still work.

use std::fs;
use std::process::Command;

/// Reported by `--version` when no revision can be determined.
const UNKNOWN: &str = "unknown";

fn main() {
    emit_rerun_triggers();
    println!("cargo:rustc-env=PROSECRAFT_BUILD={}", describe());
}

/// The commit, plus `-dirty` when the worktree differs from it.
///
/// `--always` makes this work in a repo with no tags, which is the normal state here;
/// `--dirty` is what keeps a build from uncommitted changes from claiming a clean
/// revision it does not have.
fn describe() -> String {
    let output = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();

    let Ok(output) = output else {
        return UNKNOWN.to_string();
    };
    if !output.status.success() {
        return UNKNOWN.to_string();
    }

    let stamp = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stamp.is_empty() {
        UNKNOWN.to_string()
    } else {
        stamp
    }
}

/// Makes a commit or a checkout rebuild the stamp.
///
/// Without these the first value seen would be frozen for the life of the target
/// directory, which is worse than no stamp: `--version` would name a revision that is no
/// longer what the binary contains. `.git/index` is deliberately not listed, because
/// `git diff` can refresh it and a script that both reads and rewrites a watched file
/// rebuilds forever.
fn emit_rerun_triggers() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/packed-refs");
    if let Some(reference) = head_reference() {
        println!("cargo:rerun-if-changed={reference}");
    }
}

/// The file `.git/HEAD` points at, e.g. `.git/refs/heads/main`, when it is a symbolic
/// reference. A detached HEAD, or a worktree where `.git` is a file, yields `None`.
fn head_reference() -> Option<String> {
    let head = fs::read_to_string(".git/HEAD").ok()?;
    let name = head.trim().strip_prefix("ref: ")?;
    Some(format!(".git/{name}"))
}
