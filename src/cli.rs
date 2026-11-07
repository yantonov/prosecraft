//! Command line parsing and startup validation (spec §3).
//!
//! Everything here is terminal-free and I/O-injectable: [`assemble`] takes the binary
//! lookup and the clipboard as parameters, so the whole validation order can be tested
//! without a TTY, a real harness, or a real clipboard.
//!
//! Order matters and is fixed by the spec: clap parse, then the harness binary, then the
//! `--*-file` reads. A missing harness must never surface as a file error, and neither
//! must happen after raw mode is entered.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::error::{ContextKind, ContextValue};
use clap::{CommandFactory, Parser, ValueEnum};

/// The harness adapter selected on the command line. It cannot change at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Harness {
    Claude,
    Pi,
}

impl Harness {
    /// The binary name that must resolve on `PATH`, which is also the name users know.
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Pi => "pi",
        }
    }
}

/// The crate version plus the revision `build.rs` stamped in, so a binary on `PATH` can
/// say which commit it is. One string for both `-V` and `--version`.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("PROSECRAFT_BUILD"),
    ")"
);

/// Parsed command line. Required-ness, conflicts, and values are all clap's job, so an
/// unknown `--harness` or a `--prompt`/`--prompt-file` pair exits 2 with usage.
#[derive(Debug, Parser)]
#[command(
    name = "prosecraft",
    bin_name = "prosecraft",
    version = VERSION,
    about = "Rewrite a block of text with an installed LLM harness"
)]
pub struct Args {
    /// Harness adapter to run the rewrite with
    #[arg(long, value_name = "HARNESS", value_enum)]
    pub harness: Harness,

    /// Instructions for the rewrite, read once at startup
    #[arg(long, value_name = "TEXT", conflicts_with = "prompt_file")]
    pub prompt: Option<String>,

    /// File to read the instructions from once at startup
    #[arg(long, value_name = "PATH")]
    pub prompt_file: Option<PathBuf>,

    /// Initial source text; wins over --text-file and the clipboard
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// File to read the initial source text from once at startup
    #[arg(long, value_name = "PATH")]
    pub text_file: Option<PathBuf>,

    /// Do not read the clipboard at startup
    #[arg(long)]
    pub no_clipboard: bool,

    /// Model name, passed through to the harness unchanged
    #[arg(long, value_name = "NAME")]
    pub model: Option<String>,
}

/// Parses the real command line, exiting 0 for `-h`/`-V` and 2 for misuse.
///
/// This is the only place that exits, which keeps the rest of the module testable.
pub fn parse() -> Args {
    match try_parse_from(std::env::args_os()) {
        Ok(args) => args,
        Err(err) => err.exit(),
    }
}

/// [`parse`] without the exit, so tests can assert on the error clap built.
pub fn try_parse_from<I, T>(argv: I) -> Result<Args, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Args::try_parse_from(argv).map_err(attach_usage)
}

/// clap omits the usage line for an invalid value, but spec §3 requires `--harness nope`
/// to exit 2 *with usage*, so the line it renders is attached to any parsing error.
fn attach_usage(err: clap::Error) -> clap::Error {
    // Help and version are not errors; adding a usage block would drown the help text.
    if !err.use_stderr() || err.get(ContextKind::Usage).is_some() {
        return err;
    }
    let usage = Args::command().render_usage();
    let mut err = err;
    err.insert(ContextKind::Usage, ContextValue::StyledStr(usage));
    err
}

/// What the TUI starts from. Deliberately terminal-free: the startup path builds this
/// before any raw mode is entered, and the TUI only consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub harness: Harness,
    pub model: Option<String>,
    /// `None` until the first rewrite creates a session (spec §5). The status line shows an
    /// em-dash while it is absent, which is why this is not defaulted to an empty string.
    pub session_id: Option<String>,
    pub initial_source: String,
    pub initial_instructions: String,
}

/// Reads the system clipboard at startup.
///
/// A trait rather than a direct call so the clipboard ticket can plug in a real reader
/// without moving any of the precedence logic, and so tests can prove whether the
/// clipboard was consulted at all.
pub trait ClipboardRead {
    /// Returns the clipboard text, or `None` when it is empty or unavailable.
    fn read_text(&mut self) -> Option<String>;
}

/// Placeholder used until the clipboard ticket lands: it never touches the system
/// clipboard, so startup has no side effects beyond the files named on the command line.
#[derive(Debug, Default)]
pub struct PlaceholderClipboard;

impl ClipboardRead for PlaceholderClipboard {
    fn read_text(&mut self) -> Option<String> {
        None
    }
}

/// A startup failure that is not command-line misuse. Misuse is clap's exit code 2; these
/// are runtime failures of an otherwise valid invocation, so they exit 1.
#[derive(Debug)]
pub enum Error {
    /// The harness program named by `--harness` is not on `PATH`.
    HarnessMissing(Harness),
    /// A `--text-file` / `--prompt-file` path could not be read as UTF-8 text.
    ReadFile {
        flag: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl Error {
    /// 1, not 2: the command line was well formed, the machine is not ready.
    pub fn exit_code(&self) -> u8 {
        1
    }
}

impl fmt::Display for Error {
    /// One line, because the spec asks for a clear single-line error on stderr and a
    /// multi-line report is harder to spot in a shell prompt's output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HarnessMissing(harness) => {
                write!(
                    f,
                    "harness binary `{}` not found on PATH",
                    harness.binary_name()
                )
            }
            Self::ReadFile { flag, path, source } => {
                write!(f, "cannot read {flag} `{}`: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HarnessMissing(_) => None,
            Self::ReadFile { source, .. } => Some(source),
        }
    }
}

/// Resolves the harness binary and reads the startup files, in the order spec §3 requires.
///
/// `lookup` and `clipboard` are injected so this function has no hidden dependency on the
/// machine: tests simulate a missing binary and count clipboard reads without spawning.
pub fn assemble(
    args: Args,
    lookup: impl Fn(&str) -> bool,
    clipboard: &mut impl ClipboardRead,
) -> Result<Config, Error> {
    // Before the file reads: a missing harness is the more fundamental problem, and the
    // spec forbids opening the UI in that case, so its error must win.
    if !lookup(args.harness.binary_name()) {
        return Err(Error::HarnessMissing(args.harness));
    }

    // Read exactly once, here. `Config` holds the text, not the path, so nothing later
    // can re-read the file — there is no reload hotkey and no lazy source.
    let initial_instructions = match (&args.prompt, &args.prompt_file) {
        (Some(text), _) => text.clone(),
        (None, Some(path)) => read_text_file("--prompt-file", path)?,
        (None, None) => String::new(),
    };

    // Spec §3 precedence: --text > --text-file > clipboard > empty.
    let initial_source = match (&args.text, &args.text_file) {
        (Some(text), _) => text.clone(),
        (None, Some(path)) => read_text_file("--text-file", path)?,
        (None, None) if args.no_clipboard => String::new(),
        (None, None) => clipboard.read_text().unwrap_or_default(),
    };

    Ok(Config {
        harness: args.harness,
        model: args.model,
        // Created lazily on the first `Ctrl+R`; startup never starts a session.
        session_id: None,
        initial_source,
        initial_instructions,
    })
}

fn read_text_file(flag: &'static str, path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|source| Error::ReadFile {
        flag,
        path: path.to_path_buf(),
        source,
    })
}

/// Production binary lookup: the same answer the shell would give for a bare name.
///
/// A name containing a separator is taken literally, matching how the harness is later
/// spawned; only a bare name is searched on `PATH`.
pub fn binary_on_path(binary: &str) -> bool {
    lookup_in(std::env::var_os("PATH").as_deref(), binary)
}

/// The `PATH` search with the path value passed in, so the search is testable without
/// mutating the process environment (which would race with parallel tests).
fn lookup_in(path_var: Option<&std::ffi::OsStr>, binary: &str) -> bool {
    let candidate = Path::new(binary);
    // A name with a separator is already a path; searching `PATH` for it would be wrong.
    if candidate.components().count() > 1 {
        return is_executable_file(candidate);
    }
    let Some(path_var) = path_var else {
        return false;
    };
    std::env::split_paths(path_var).any(|dir| {
        executable_names(binary)
            .iter()
            .any(|name| is_executable_file(&dir.join(name)))
    })
}

/// A directory is not a program, and on Unix the execute bit is what makes a file
/// runnable, so a readable-but-not-executable file must not pass the startup check.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Windows finds an extensionless name through `PATHEXT`, so the check has to try the
/// same names or a real `claude.exe` install would look missing.
fn executable_names(binary: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        let mut names = Vec::with_capacity(5);
        names.push(OsString::from(binary));
        for ext in pathext.split(';').filter(|ext| !ext.is_empty()) {
            names.push(OsString::from(format!("{binary}{ext}")));
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![OsString::from(binary)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use clap::error::ErrorKind;

    fn parse(argv: &[&str]) -> Result<Args, clap::Error> {
        try_parse_from(argv)
    }

    fn valid(argv: &[&str]) -> Args {
        parse(argv).expect("expected a valid command line")
    }

    /// A filesystem sandbox under the OS temp dir, removed on drop. Unique per test so
    /// the suite stays parallel-safe.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "prosecraft-cli-{}-{}-{tag}",
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

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).expect("write temp file");
            path
        }

        /// Writes a stand-in binary. On Unix it needs the execute bit to count as one;
        /// elsewhere the extension rules from `PATHEXT` decide, so no mode is set.
        fn write_binary(&self, name: &str) -> PathBuf {
            let path = self.write(name, "not a real harness; never executed");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .expect("mark executable");
            }
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A harness lookup that always succeeds, used where the binary is not under test.
    fn found(_: &str) -> bool {
        true
    }

    fn not_found(_: &str) -> bool {
        false
    }

    #[derive(Default)]
    struct RecordingClipboard {
        text: Option<String>,
        reads: usize,
    }

    impl RecordingClipboard {
        fn with(text: &str) -> Self {
            Self {
                text: Some(text.to_owned()),
                reads: 0,
            }
        }
    }

    impl ClipboardRead for RecordingClipboard {
        fn read_text(&mut self) -> Option<String> {
            self.reads += 1;
            self.text.clone()
        }
    }

    fn assemble_args(
        argv: &[&str],
        lookup: impl Fn(&str) -> bool,
    ) -> (Result<Config, Error>, usize) {
        let mut clipboard = RecordingClipboard::with("clipboard text");
        let result = assemble(valid(argv), lookup, &mut clipboard);
        (result, clipboard.reads)
    }

    #[test]
    fn help_is_a_clap_exit_zero_before_any_harness_lookup() {
        // clap renders help as an "error"; its exit code 0 is what makes -h work on a
        // machine with no harness installed.
        let err = parse(&["prosecraft", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        assert_eq!(err.exit_code(), 0);
        assert!(err.to_string().contains("--harness"));
    }

    #[test]
    fn version_is_a_clap_exit_zero_before_any_harness_lookup() {
        let err = parse(&["prosecraft", "-V"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(err.exit_code(), 0);
        assert!(err.to_string().contains("prosecraft"));
    }

    #[test]
    fn unknown_harness_is_clap_misuse_exit_two_with_usage() {
        let err = parse(&["prosecraft", "--harness", "nope"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        assert_eq!(err.exit_code(), 2);
        let rendered = err.to_string();
        assert!(rendered.contains("nope"), "value not named: {rendered}");
        assert!(rendered.contains("Usage"), "no usage line: {rendered}");
    }

    #[test]
    fn harness_is_required() {
        let err = parse(&["prosecraft"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn both_harness_values_parse() {
        assert_eq!(
            valid(&["prosecraft", "--harness", "claude"]).harness,
            Harness::Claude
        );
        assert_eq!(
            valid(&["prosecraft", "--harness", "pi"]).harness,
            Harness::Pi
        );
    }

    #[test]
    fn harness_binary_names_match_the_cli_values() {
        assert_eq!(Harness::Claude.binary_name(), "claude");
        assert_eq!(Harness::Pi.binary_name(), "pi");
    }

    #[test]
    fn prompt_and_prompt_file_conflict_is_exit_two() {
        let err = parse(&[
            "prosecraft",
            "--harness",
            "claude",
            "--prompt",
            "x",
            "--prompt-file",
            "y",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn missing_harness_names_the_expected_binary_and_skips_file_reads() {
        // The text file does not exist either; the harness error must win so the user
        // is told about the thing that stops the app from running at all.
        let (result, _) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--text-file",
                "definitely-not-here.txt",
            ],
            not_found,
        );
        match result.unwrap_err() {
            Error::HarnessMissing(harness) => assert_eq!(harness, Harness::Claude),
            other => panic!("expected HarnessMissing, got {other:?}"),
        }
    }

    #[test]
    fn harness_lookup_receives_the_binary_name_of_the_selected_harness() {
        let seen = std::cell::Cell::new(None);
        let args = valid(&["prosecraft", "--harness", "pi"]);
        let mut clipboard = RecordingClipboard::default();
        let config = assemble(
            args,
            |binary| {
                seen.set(Some(binary.to_owned()));
                true
            },
            &mut clipboard,
        )
        .unwrap();

        assert_eq!(seen.take().as_deref(), Some("pi"));
        assert_eq!(config.harness, Harness::Pi);
    }

    #[test]
    fn missing_harness_error_is_one_line_and_exits_one() {
        let (result, _) = assemble_args(&["prosecraft", "--harness", "claude"], not_found);
        let err = result.unwrap_err();
        let message = err.to_string();

        assert_eq!(message.lines().count(), 1, "not one line: {message:?}");
        assert!(message.contains("claude"), "binary not named: {message}");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn missing_text_file_is_reported_with_flag_and_path() {
        let (result, _) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--text-file",
                "missing.txt",
            ],
            found,
        );
        let err = result.unwrap_err();
        let message = err.to_string();

        assert!(message.contains("--text-file"), "flag missing: {message}");
        assert!(message.contains("missing.txt"), "path missing: {message}");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn missing_prompt_file_is_reported_with_flag_and_path() {
        let (result, _) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--prompt-file",
                "missing.txt",
            ],
            found,
        );
        let err = result.unwrap_err();
        let message = err.to_string();

        assert!(message.contains("--prompt-file"), "flag missing: {message}");
        assert!(message.contains("missing.txt"), "path missing: {message}");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn prompt_file_is_read_once_and_never_re_read() {
        let dir = TempDir::new("read-once");
        let prompt = dir.write("prompt.txt", "first version");
        let path = prompt.to_str().unwrap();

        let (result, _) = assemble_args(
            &["prosecraft", "--harness", "claude", "--prompt-file", path],
            found,
        );
        let config = result.unwrap();

        // A second read would pick up the new contents; the spec forbids any reload.
        fs::write(&prompt, "second version").unwrap();
        assert_eq!(config.initial_instructions, "first version");
    }

    #[test]
    fn prompt_flag_fills_instructions() {
        let (result, _) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--prompt",
                "make it formal",
            ],
            found,
        );
        assert_eq!(result.unwrap().initial_instructions, "make it formal");
    }

    #[test]
    fn prompt_file_fills_instructions() {
        let dir = TempDir::new("prompt-file");
        let prompt = dir.write("prompt.txt", "from the file");
        let (result, _) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--prompt-file",
                prompt.to_str().unwrap(),
            ],
            found,
        );
        assert_eq!(result.unwrap().initial_instructions, "from the file");
    }

    #[test]
    fn instructions_start_empty_without_prompt_flags() {
        let (result, _) = assemble_args(&["prosecraft", "--harness", "claude"], found);
        assert_eq!(result.unwrap().initial_instructions, "");
    }

    #[test]
    fn text_beats_text_file_and_the_clipboard() {
        let dir = TempDir::new("text-wins");
        let file = dir.write("source.txt", "from the file");
        let (result, reads) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--text",
                "from the flag",
                "--text-file",
                file.to_str().unwrap(),
            ],
            found,
        );

        assert_eq!(result.unwrap().initial_source, "from the flag");
        // Nothing below --text in the precedence order may even be consulted.
        assert_eq!(reads, 0, "clipboard read despite --text");
    }

    #[test]
    fn text_file_beats_the_clipboard() {
        let dir = TempDir::new("file-wins");
        let file = dir.write("source.txt", "from the file");
        let (result, reads) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--text-file",
                file.to_str().unwrap(),
            ],
            found,
        );

        assert_eq!(result.unwrap().initial_source, "from the file");
        assert_eq!(reads, 0, "clipboard read despite --text-file");
    }

    #[test]
    fn empty_text_value_still_beats_the_clipboard() {
        // An explicit empty --text is a choice, not an absent flag; falling back to the
        // clipboard here would be surprising.
        let (result, reads) =
            assemble_args(&["prosecraft", "--harness", "claude", "--text", ""], found);
        assert_eq!(result.unwrap().initial_source, "");
        assert_eq!(reads, 0);
    }

    #[test]
    fn clipboard_fills_source_when_no_text_flag_is_given() {
        let (result, reads) = assemble_args(&["prosecraft", "--harness", "claude"], found);
        assert_eq!(result.unwrap().initial_source, "clipboard text");
        assert_eq!(reads, 1);
    }

    #[test]
    fn empty_clipboard_starts_with_an_empty_source() {
        let mut clipboard = RecordingClipboard::default();
        let config = assemble(
            valid(&["prosecraft", "--harness", "claude"]),
            found,
            &mut clipboard,
        )
        .unwrap();
        assert_eq!(config.initial_source, "");
    }

    #[test]
    fn no_clipboard_skips_the_read_and_still_starts() {
        let (result, reads) = assemble_args(
            &["prosecraft", "--harness", "claude", "--no-clipboard"],
            found,
        );

        let config = result.expect("app must still start with --no-clipboard");
        assert_eq!(reads, 0, "clipboard read despite --no-clipboard");
        assert_eq!(config.initial_source, "");
    }

    #[test]
    fn no_clipboard_does_not_disable_text_or_file_flags() {
        let (result, reads) = assemble_args(
            &[
                "prosecraft",
                "--harness",
                "claude",
                "--no-clipboard",
                "--text",
                "explicit",
            ],
            found,
        );

        assert_eq!(result.unwrap().initial_source, "explicit");
        assert_eq!(reads, 0);
    }

    #[test]
    fn model_is_stored_for_pass_through_and_changes_nothing_else() {
        let args = valid(&[
            "prosecraft",
            "--harness",
            "claude",
            "--model",
            "opus",
            "--prompt",
            "instructions",
            "--text",
            "source",
        ]);
        let config = assemble(args, found, &mut RecordingClipboard::default()).unwrap();

        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.initial_instructions, "instructions");
        assert_eq!(config.initial_source, "source");
    }

    #[test]
    fn model_defaults_to_none() {
        let (result, _) = assemble_args(&["prosecraft", "--harness", "pi"], found);
        assert_eq!(result.unwrap().model, None);
    }

    #[test]
    fn binary_on_path_accepts_an_executable_file_given_by_path() {
        let dir = TempDir::new("exec");
        let path = dir.write_binary("claude");

        assert!(binary_on_path(path.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn binary_on_path_rejects_a_file_without_the_execute_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("noexec");
        let path = dir.write("claude", "");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(!binary_on_path(path.to_str().unwrap()));
    }

    #[test]
    fn binary_on_path_rejects_a_path_that_does_not_exist() {
        let dir = TempDir::new("absent");
        let path = dir.path("claude");

        assert!(!binary_on_path(path.to_str().unwrap()));
    }

    /// `PATH` is passed in rather than set, so this test cannot race the others.
    fn path_var(dirs: &[&TempDir]) -> std::ffi::OsString {
        std::env::join_paths(dirs.iter().map(|dir| dir.root())).expect("join PATH entries")
    }

    #[test]
    fn lookup_finds_a_binary_in_a_later_path_entry() {
        let empty = TempDir::new("shadowed");
        let dir = TempDir::new("second-entry");
        dir.write_binary("claude");

        assert!(lookup_in(Some(&path_var(&[&empty, &dir])), "claude"));
    }

    #[test]
    fn lookup_reports_not_found_when_no_entry_holds_the_binary() {
        let first = TempDir::new("a");
        let second = TempDir::new("b");

        assert!(!lookup_in(Some(&path_var(&[&first, &second])), "claude"));
    }

    #[test]
    fn lookup_without_a_path_variable_is_not_found() {
        assert!(!lookup_in(None, "claude"));
    }

    #[test]
    fn lookup_does_not_match_a_different_binary_name() {
        let dir = TempDir::new("wrong-name");
        dir.write_binary("pi");

        assert!(!lookup_in(Some(&path_var(&[&dir])), "claude"));
    }

    #[test]
    fn lookup_does_not_match_a_directory_named_like_the_binary() {
        let dir = TempDir::new("dir-named");
        fs::create_dir(dir.path("claude")).unwrap();

        assert!(!lookup_in(Some(&path_var(&[&dir])), "claude"));
    }
}
