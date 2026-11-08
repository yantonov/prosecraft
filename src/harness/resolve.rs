//! Turning a harness name into the program a spawn actually needs (spec §3).
//!
//! This module exists because the startup check and the spawn used to answer "is the harness
//! installed?" differently, and the app opened a window it could not use. A `bool` was the
//! defect: a boolean can be true for a file that `Command::new` will never run. So the single
//! question here is "which program do I hand to the OS, or why is there none", answered once
//! per run and reused by both callers.
//!
//! Measured on Windows, where the difference is not academic. The `pi` harness ships as
//! `pi`, `pi.cmd` and `pi.ps1` (npm's shims) and no `pi.exe`:
//!
//! | call | result |
//! |---|---|
//! | `Command::new("pi")` | `program not found` — `CreateProcess` appends only `.exe` |
//! | `Command::new("pi.cmd")` | runs |
//! | `Command::new(<dir>/pi)` | `os error 5` — the extensionless file is a shell script |
//! | `Command::new(<dir>/bar)` (extensionless text) | `os error 193`, "not a valid Win32 application" |
//!
//! So on Windows an extensionless candidate is never a program, and the `.exe`-style
//! candidates are what to return. On unix the opposite rule holds: the name is the program and
//! the execute bit decides. Those are two rule sets, not one rule with parameters, which is why
//! they live in separate `cfg` bodies below rather than in a shared "extensions" list.
//!
//! Nothing here spawns anything: [`Resolved`] is a description, and the caller decides what a
//! non-spawnable answer means. That keeps this testable against a fixture directory with no
//! process and no terminal, and it keeps every platform rule out of `cli.rs` and `process.rs`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a harness name resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The program to give to `Command::new`: the absolute path of the file that will
    /// actually be started.
    Program { program: PathBuf },
    /// Nothing on this machine can run the harness.
    Missing,
    /// The only candidate is a Windows batch shim, which this crate cannot drive yet.
    ///
    /// Separate from [`Resolved::Missing`] on purpose: an installation *is* present, and
    /// telling the user to install something that is already installed would be wrong.
    ShimUnsupported,
}

impl Resolved {
    /// The program, if this resolution can be spawned.
    ///
    /// This is the single verdict both the startup check and the spawn read, which is what
    /// keeps them from disagreeing.
    pub fn program(&self) -> Option<&Path> {
        match self {
            Self::Program { program } => Some(program.as_path()),
            Self::Missing | Self::ShimUnsupported => None,
        }
    }
}

impl fmt::Display for Resolved {
    /// The one line spec §3 asks for on stderr, phrased so the reader knows whether to install
    /// something or to stop using a platform.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Reaching this with a name would mean the caller formatted a non-missing answer as
            // an error; the message stays useful rather than panicking.
            Self::Program { program } => write!(f, "`{}`", program.display()),
            Self::Missing => f.write_str("not found on PATH"),
            Self::ShimUnsupported => f.write_str(
                "is only installed as a Windows batch shim (a runnable .exe is missing), \
                 which this build cannot drive",
            ),
        }
    }
}

/// Resolves `binary` against the machine's own `PATH` and platform rules.
///
/// The production entry point. The rules themselves are in [`lookup_in`], which takes the
/// environment as arguments so a test never mutates the process environment.
pub fn lookup(binary: &str) -> Resolved {
    let path_var = std::env::var_os("PATH");
    let pathext = std::env::var("PATHEXT").ok();
    lookup_in(path_var.as_deref(), pathext.as_deref(), binary)
}

/// The lookup with the environment passed in, so every rule below is testable as a table.
///
/// `pathext` is `None` on unix, where nothing reads it; on Windows `None` means the shell's
/// documented default. Passing it in rather than reading it is what keeps the tests from
/// racing over `PATHEXT`.
pub fn lookup_in(
    path_var: Option<&std::ffi::OsStr>,
    pathext: Option<&str>,
    binary: &str,
) -> Resolved {
    let candidate = Path::new(binary);
    // A name with a separator is already a path: searching `PATH` for it would be wrong, and
    // this is the one case where the user's own answer is taken as given.
    if candidate.components().count() > 1 {
        return classify(candidate).unwrap_or(Resolved::Missing);
    }
    let Some(path_var) = path_var else {
        return Resolved::Missing;
    };
    for dir in std::env::split_paths(path_var) {
        for name in candidate_names(binary, pathext) {
            if let Some(resolved) = classify(&dir.join(&name)) {
                return resolved;
            }
        }
    }
    Resolved::Missing
}

/// The candidate file names for one `PATH` entry, in the order they are tried.
///
/// Windows: `PATHEXT` order first, because that is the rule the shell uses and the order is
/// what makes a real `.exe` win over the extensionless npm shim sitting beside it. The
/// extensionless name is tried last, and only because a file named exactly that might still be
/// a batch shim the user renamed; it is never returned as a program (see [`classify`]).
#[cfg(windows)]
fn candidate_names(binary: &str, pathext: Option<&str>) -> Vec<OsString> {
    const DEFAULT: &str = ".COM;.EXE;.BAT;.CMD";
    let pathext = pathext.filter(|value| !value.is_empty()).unwrap_or(DEFAULT);
    let mut names: Vec<OsString> = pathext
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| OsString::from(format!("{binary}{ext}")))
        .collect();
    names.push(OsString::from(binary));
    names
}

/// Unix: the name is the program. `PATHEXT` does not exist, so it is not consulted and no
/// extension is ever appended.
#[cfg(not(windows))]
fn candidate_names(binary: &str, _pathext: Option<&str>) -> Vec<OsString> {
    vec![OsString::from(binary)]
}

/// Whether one concrete path is a runnable harness, and what it resolved to.
///
/// `None` means "not this candidate, keep looking". The distinction that matters: on Windows a
/// found-but-unusable file is *still an answer*, because stopping the search there keeps the
/// resolver from picking a worse candidate further along `PATH`.
fn classify(path: &Path) -> Option<Resolved> {
    let metadata = std::fs::metadata(path).ok()?;
    // A directory is not a program, on either platform.
    if !metadata.is_file() {
        return None;
    }
    classify_file(path)
}

/// Unix: the execute bit is what makes a file runnable, so a readable-but-not-executable file
/// must not pass.
#[cfg(not(windows))]
fn classify_file(path: &Path) -> Option<Resolved> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    if mode & 0o111 == 0 {
        return None;
    }
    Some(Resolved::Program {
        program: path.to_path_buf(),
    })
}

/// Windows: the extension decides, not the bytes and not the name.
///
/// A `.exe`/`.com` file is a program. A `.cmd`/`.bat` file is a program by the shell's rules
/// but a batch shim for this crate's purposes, so it is reported as such rather than hidden.
/// The extensionless name is never a program: measured, an extensionless file is either a
/// shell script (`os error 5`) or not an image at all (`os error 193`), and `CreateProcess`
/// refuses a bare name that has no extension anyway.
#[cfg(windows)]
fn classify_file(path: &Path) -> Option<Resolved> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("exe" | "com") => Some(Resolved::Program {
            program: path.to_path_buf(),
        }),
        Some("cmd" | "bat") => Some(Resolved::ShimUnsupported),
        // A `.ps1`, an extensionless npm shim, or anything else: present but not a program
        // this crate can start, so keep looking for a candidate that is.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fixture directory that removes itself, so the tests leave nothing behind.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "prosecraft-resolve-{}-{}-{tag}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn root(&self) -> &Path {
            &self.0
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Writes a stand-in harness for `base` and returns the file it created.
        ///
        /// The platform decides the name: on unix the bare name is the program, while on
        /// Windows an extensionless file is deliberately *not* one, so a fixture for "found"
        /// has to carry `.exe`.
        fn write_binary(&self, base: &str) -> PathBuf {
            let name = if cfg!(windows) {
                format!("{base}.exe")
            } else {
                base.to_owned()
            };
            let path = self.path(&name);
            fs::write(&path, "not a real harness; never executed").expect("write fixture");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .expect("mark executable");
            }
            path
        }

        /// A plain file with no execute bit, for the "found but not runnable" rows.
        fn write_plain(&self, name: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, "not a program").expect("write fixture");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn path_var(dirs: &[&TempDir]) -> std::ffi::OsString {
        std::env::join_paths(dirs.iter().map(|dir| dir.root())).expect("join PATH entries")
    }

    #[test]
    fn a_missing_binary_is_missing() {
        let dir = TempDir::new("absent");

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "claude"),
            Resolved::Missing
        );
    }

    #[test]
    fn an_empty_or_absent_path_variable_finds_nothing() {
        assert_eq!(lookup_in(None, None, "claude"), Resolved::Missing);
    }

    #[test]
    fn a_directory_named_like_the_binary_is_not_a_program() {
        let dir = TempDir::new("dir-named");
        fs::create_dir(dir.path("claude")).unwrap();

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "claude"),
            Resolved::Missing
        );
    }

    #[test]
    fn a_binary_in_a_later_path_entry_is_found() {
        let empty = TempDir::new("shadowed");
        let dir = TempDir::new("second-entry");
        dir.write_binary("claude");

        let resolved = lookup_in(Some(&path_var(&[&empty, &dir])), None, "claude");
        // The returned path is the PATHEXT candidate (`.EXE`), not the stem, so the two are
        // compared case-insensitively: on Windows the disk spelling and the shell's spelling
        // differ in case and mean the same file.
        let found = resolved
            .program()
            .expect("the second entry was not searched");
        assert!(
            found
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(&dir.root().to_string_lossy().to_ascii_lowercase()),
            "the second entry was not searched: {resolved:?}"
        );
    }

    #[test]
    fn a_name_with_a_separator_is_taken_as_a_path() {
        let dir = TempDir::new("by-path");
        let path = dir.write_binary("claude");

        // `PATH` is deliberately empty: an absolute name must not need one.
        let resolved = lookup_in(None, None, path.to_str().unwrap());
        assert_eq!(resolved.program(), Some(path.as_path()));
    }

    #[test]
    fn a_path_that_does_not_exist_is_missing() {
        let dir = TempDir::new("absent-path");

        assert_eq!(
            lookup_in(None, None, dir.path("claude").to_str().unwrap()),
            Resolved::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_without_the_execute_bit_is_not_a_program() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("noexec");
        let path = dir.write_plain("claude");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "claude"),
            Resolved::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_never_appends_an_extension() {
        // The rule that makes this a separate implementation rather than a shared one: a
        // `claude.cmd` is not a program on unix, and nothing should go looking for one.
        let dir = TempDir::new("no-ext");
        dir.write_binary("claude.cmd");

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "claude"),
            Resolved::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_resolves_the_bare_name() {
        let dir = TempDir::new("bare");
        let path = dir.write_binary("pi");

        let resolved = lookup_in(Some(&path_var(&[&dir])), None, "pi");
        assert_eq!(resolved.program(), Some(path.as_path()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefers_pathext_candidates_over_the_extensionless_shim() {
        // Measured: the npm install of pi is `pi` (a shell script) beside `pi.cmd`, with no
        // `pi.exe`. `CreateProcess` refuses the extensionless file with `os error 5`, so
        // returning it as a program is the defect this test pins down.
        let dir = TempDir::new("shim-order");
        dir.write_plain("pi");
        dir.write_plain("pi.cmd");

        let resolved = lookup_in(Some(&path_var(&[&dir])), Some(".COM;.EXE;.BAT;.CMD"), "pi");
        assert_eq!(
            resolved,
            Resolved::ShimUnsupported,
            "a batch shim must be reported, not returned as a program"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_never_returns_an_extensionless_file_as_a_program() {
        let dir = TempDir::new("extless-only");
        dir.write_plain("pi");

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "pi"),
            Resolved::Missing,
            "an extensionless file cannot be started by CreateProcess"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_returns_an_exe_whatever_its_case() {
        let dir = TempDir::new("exe");
        let path = dir.write_plain("pi.EXE");

        let resolved = lookup_in(Some(&path_var(&[&dir])), None, "pi");
        assert_eq!(resolved, Resolved::Program { program: path });
    }

    #[cfg(windows)]
    #[test]
    fn windows_stops_at_the_first_candidate_that_exists() {
        // A `.exe` in an earlier entry wins over a `.cmd` in a later one: the shell's order,
        // not "keep looking for something nicer".
        let first = TempDir::new("first");
        let second = TempDir::new("second");
        let exe = first.write_plain("pi.exe");
        second.write_plain("pi.cmd");

        let resolved = lookup_in(Some(&path_var(&[&first, &second])), None, "pi");
        // Compared case-insensitively on purpose: the candidate name comes from `PATHEXT`
        // (`.EXE`) while the file on disk is lowercase, and Windows matches the two. Spelling
        // the expectation as the disk spelling is what the user's installation looks like.
        let found = resolved.program().expect("the .exe was not found");
        assert_eq!(
            found.to_string_lossy().to_ascii_lowercase(),
            exe.to_string_lossy().to_ascii_lowercase()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_ignores_a_ps1_shim() {
        // Present in the real npm install; not startable through `Command`.
        let dir = TempDir::new("ps1");
        dir.write_plain("pi.ps1");

        assert_eq!(
            lookup_in(Some(&path_var(&[&dir])), None, "pi"),
            Resolved::Missing
        );
    }

    #[test]
    fn a_batch_shim_is_never_reported_as_missing() {
        // The message differs because the remedy does: nothing is left to install.
        let missing = Resolved::Missing.to_string();
        let shim = Resolved::ShimUnsupported.to_string();
        assert_ne!(missing, shim);
        assert!(
            shim.contains("batch shim"),
            "the shim message must name what is wrong: {shim}"
        );
    }
}
