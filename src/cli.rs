//! Command line parsing and startup validation (spec §3).
//!
//! Everything here is terminal-free and I/O-injectable: [`assemble`] takes the binary
//! lookup and the clipboard as parameters, so the whole validation order can be tested
//! without a TTY, a real harness, or a real clipboard.
//!
//! Order matters and is fixed by the spec: clap parse, then the harness binary, then the
//! `--*-file` reads. A missing harness must never surface as a file error, and neither
//! must happen after raw mode is entered.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::error::{ContextKind, ContextValue};
use clap::{CommandFactory, Parser, ValueEnum};

use crate::harness::resolve;

/// The harness adapter selected on the command line. It cannot change at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Harness {
    Claude,
    Pi,
    Codex,
}

impl Harness {
    /// The binary name that must resolve on `PATH`, which is also the name users know.
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Pi => "pi",
            Self::Codex => "codex",
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
    /// The program the harness resolved to at startup. Carried rather than looked up again so
    /// the startup check and every later spawn are the same answer by construction (spec §3).
    pub harness_program: PathBuf,
    /// Whether that program is a Windows batch shim, which needs the prompt on stdin
    /// (spec §6). Carried beside the program so the two cannot drift.
    pub harness_is_batch: bool,
    pub model: Option<String>,
    /// `None` until the first rewrite creates a session (spec §5). The status line shows an
    /// em-dash while it is absent, which is why this is not defaulted to an empty string.
    pub session_id: Option<String>,
    pub initial_source: String,
    pub initial_instructions: String,
}

/// A startup failure that is not command-line misuse. Misuse is clap's exit code 2; these
/// are runtime failures of an otherwise valid invocation, so they exit 1.
#[derive(Debug)]
pub enum Error {
    /// The harness program named by `--harness` could not be resolved to something runnable.
    /// Carries the resolution so the message can say *which* way it failed: absent, or present
    /// but a shim this build cannot drive.
    HarnessMissing(Harness, resolve::Resolved),
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
            Self::HarnessMissing(harness, reason) => {
                write!(f, "harness binary `{}` {reason}", harness.binary_name())
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
            Self::HarnessMissing(..) => None,
            Self::ReadFile { source, .. } => Some(source),
        }
    }
}

/// Resolves the harness program and reads the startup files, in the order spec §3 requires.
///
/// `lookup` and `clipboard` are injected so this function has no hidden dependency on the
/// machine: tests simulate a missing binary and count clipboard reads without spawning.
pub fn assemble(
    args: Args,
    lookup: impl Fn(&str) -> resolve::Resolved,
    clipboard: &mut impl crate::clipboard::Clipboard,
) -> Result<Config, Error> {
    // Before the file reads: a missing harness is the more fundamental problem, and the
    // spec forbids opening the UI in that case, so its error must win.
    //
    // `program()` is the whole verdict: a resolution that cannot name a runnable program is a
    // refusal, which is what keeps this check and the spawn from disagreeing.
    let resolved = lookup(args.harness.binary_name());
    let Some(harness_program) = resolved.program() else {
        return Err(Error::HarnessMissing(args.harness, resolved));
    };
    let harness_is_batch = resolved.needs_stdin_prompt();
    let harness_program = harness_program.to_path_buf();

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
        // An unavailable or empty clipboard leaves the pane empty rather than failing
        // startup: spec §8 makes this a status-line warning with everything else working.
        (None, None) => clipboard.get_text().ok().flatten().unwrap_or_default(),
    };

    Ok(Config {
        harness: args.harness,
        harness_program,
        harness_is_batch,
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

/// Production binary lookup: the machine's own answer for a harness name.
///
/// Delegates to [`crate::harness::resolve`], which owns every platform rule. This wrapper
/// exists only so the call reads as "the machine's `PATH`" at the startup site.
pub fn resolve_harness(binary: &str) -> resolve::Resolved {
    resolve::lookup(binary)
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
    fn found(_: &str) -> resolve::Resolved {
        resolve::Resolved::Program {
            program: PathBuf::from("stub-harness"),
            batch: false,
        }
    }

    fn not_found(_: &str) -> resolve::Resolved {
        resolve::Resolved::Missing
    }

    fn assemble_args(
        argv: &[&str],
        lookup: impl Fn(&str) -> resolve::Resolved,
    ) -> (Result<Config, Error>, usize) {
        let mut clipboard = crate::clipboard::Recording::with("clipboard text");
        let result = assemble(valid(argv), lookup, &mut clipboard);
        (result, clipboard.reads())
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

    /// Every harness the CLI accepts, so a table below cannot silently skip one.
    const ALL_HARNESSES: [Harness; 3] = [Harness::Claude, Harness::Pi, Harness::Codex];

    #[test]
    fn every_harness_value_parses() {
        for harness in ALL_HARNESSES {
            assert_eq!(
                valid(&["prosecraft", "--harness", harness.binary_name()]).harness,
                harness,
                "--harness {} did not round-trip",
                harness.binary_name()
            );
        }
    }

    #[test]
    fn harness_binary_names_match_the_cli_values() {
        assert_eq!(Harness::Claude.binary_name(), "claude");
        assert_eq!(Harness::Pi.binary_name(), "pi");
        assert_eq!(Harness::Codex.binary_name(), "codex");
    }

    #[test]
    fn the_startup_lookup_is_asked_for_the_selected_binary() {
        // Constraint 3: the same name must reach the resolver, or the startup check and the
        // spawn disagree about which file to run.
        for harness in ALL_HARNESSES {
            let seen = std::cell::Cell::new(None);
            let mut clipboard = crate::clipboard::Recording::default();
            let config = assemble(
                valid(&["prosecraft", "--harness", harness.binary_name()]),
                |binary| {
                    seen.set(Some(binary.to_owned()));
                    found(binary)
                },
                &mut clipboard,
            )
            .unwrap();

            assert_eq!(
                seen.take().as_deref(),
                Some(harness.binary_name()),
                "the resolver was not asked for {}",
                harness.binary_name()
            );
            assert_eq!(
                config.harness, harness,
                "the selected harness was not carried into the config"
            );
        }
    }

    #[test]
    fn a_missing_codex_binary_is_one_line_naming_codex_and_exits_one() {
        // The third harness must fail startup exactly like the other two rather than opening a
        // window it cannot use.
        let (result, reads) = assemble_args(&["prosecraft", "--harness", "codex"], not_found);
        let err = result.unwrap_err();
        let message = err.to_string();

        assert_eq!(message.lines().count(), 1, "not one line: {message:?}");
        assert!(message.contains("codex"), "binary not named: {message}");
        assert_eq!(err.exit_code(), 1);
        assert_eq!(reads, 0, "a refusal must not read the clipboard");
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
            Error::HarnessMissing(harness, _) => assert_eq!(harness, Harness::Claude),
            other => panic!("expected HarnessMissing, got {other:?}"),
        }
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
        let mut clipboard = crate::clipboard::Recording::default();
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
        let config = assemble(args, found, &mut crate::clipboard::Recording::default()).unwrap();

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
    fn the_resolved_program_is_carried_into_the_config() {
        // The defect this pins down: the check said "found" for a file the spawn could not
        // run, because the two asked different questions. The config now carries the answer
        // the spawn uses, so there is one verdict, not two.
        let dir = TempDir::new("carried");
        let path = dir.write_binary("claude");

        let config = assemble(
            valid(&["prosecraft", "--harness", "claude"]),
            |_| resolve::Resolved::Program {
                program: path.clone(),
                batch: false,
            },
            &mut crate::clipboard::Recording::default(),
        )
        .unwrap();

        assert_eq!(config.harness_program, path);
    }

    #[test]
    fn a_resolution_without_a_program_refuses_startup() {
        let (result, reads) = assemble_args(&["prosecraft", "--harness", "pi"], |_| {
            resolve::Resolved::Missing
        });

        match result.unwrap_err() {
            Error::HarnessMissing(harness, reason) => {
                assert_eq!(harness, Harness::Pi);
                assert_eq!(reason, resolve::Resolved::Missing);
            }
            other => panic!("expected HarnessMissing, got {other:?}"),
        }
        // Refused before the clipboard was touched, exactly like a missing binary.
        assert_eq!(reads, 0, "a refusal must not read the clipboard");
    }

    #[cfg(windows)]
    #[test]
    fn a_batch_shim_starts_instead_of_being_refused() {
        // The shim is a program the OS can start (spec §6), so it must not take the missing
        // path: only the prompt's transport changes, and that is the adapter's business.
        let dir = TempDir::new("shim-accepted");
        let shim = dir.write("pi.cmd", "@echo off");

        let config = assemble(
            valid(&["prosecraft", "--harness", "pi"]),
            |_| resolve::Resolved::Program {
                program: shim.clone(),
                batch: true,
            },
            &mut crate::clipboard::Recording::default(),
        )
        .expect("a batch shim must not be refused at startup");

        assert_eq!(config.harness_program, shim);
    }

    #[test]
    fn a_missing_harness_is_reported_as_not_found() {
        let err = Error::HarnessMissing(Harness::Claude, resolve::Resolved::Missing);
        let rendered = err.to_string();

        assert!(rendered.contains("`claude`"), "not named: {rendered}");
        assert!(
            rendered.contains("not found on PATH"),
            "not explained: {rendered}"
        );
        assert_eq!(rendered.lines().count(), 1, "spec §3 asks for one line");
    }
}
