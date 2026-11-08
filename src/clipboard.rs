//! The system clipboard: the one piece of state outside the app (spec §1).
//!
//! The app reads the clipboard at startup for the initial source text and on demand for
//! `Ctrl+V`, and writes it for `Ctrl+Y` (spec §4). That is the whole surface: no
//! config file, no session pointer, nothing else is persisted, which is why `Ctrl+N` is the
//! only reset the app needs.
//!
//! Everything here is behind the [`Clipboard`] trait so the view can be driven in tests
//! without a display server, and so "the clipboard is unavailable" is a case with a test
//! rather than a machine-specific accident. Failure is never fatal: spec §8 makes an
//! unavailable clipboard a status-line warning with every other feature still working, so
//! both operations return `Result` and the caller decides what to say.
//!
//! **Text only.** `arboard` is built with `default-features = false` because its default
//! `image-data` feature pulls the whole `image` crate — PNG, TIFF and JPEG codecs — for a
//! clipboard this app only ever moves prose through.

use std::fmt;

/// Why a clipboard operation failed.
///
/// One variant plus a message, rather than a variant per `arboard` error: every one of them
/// means the same thing to the user ("there is no clipboard here"), and the underlying text is
/// kept for the debug pane, where the detail is useful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    /// A clipboard that could not be reached: no display server, no clipboard daemon.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The detail for a debug line. The status line shows a fixed sentence instead, because a
    /// platform error string is not something a user should have to parse mid-keystroke.
    pub fn detail(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// One clipboard, as the app uses it.
///
/// `get_text` returns `Ok(None)` for an empty clipboard, which is not a failure: a paste from
/// an empty clipboard is a no-op the status line reports, not an error state (spec §8).
pub trait Clipboard {
    /// The clipboard's text, or `None` when it is empty.
    fn get_text(&mut self) -> Result<Option<String>, Error>;

    /// Replaces the clipboard's text.
    fn set_text(&mut self, text: &str) -> Result<(), Error>;
}

/// The real clipboard, backed by `arboard`.
///
/// The handle is created lazily and kept, because on X11 a `Clipboard` holds a connection to
/// the display server and creating one per keystroke would open and close it repeatedly.
pub struct System {
    inner: Option<arboard::Clipboard>,
}

/// A handle is either open or not; the platform object itself carries nothing a reader needs,
/// and `arboard::Clipboard` is not `Debug`, so the state is spelled out by hand.
impl fmt::Debug for System {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("System")
            .field("connected", &self.inner.is_some())
            .finish()
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

impl System {
    pub fn new() -> Self {
        Self { inner: None }
    }

    /// The `arboard` handle, created on first use.
    ///
    /// Created lazily so a machine with no display server only pays for the failure when the
    /// clipboard is actually asked for, and so `--no-clipboard` never opens a connection at
    /// all.
    fn handle(&mut self) -> Result<&mut arboard::Clipboard, Error> {
        if self.inner.is_none() {
            self.inner = match arboard::Clipboard::new() {
                Ok(clipboard) => Some(clipboard),
                Err(err) => {
                    return Err(Error::unavailable(format!("clipboard unavailable: {err}")));
                }
            };
        }
        self.inner
            .as_mut()
            .ok_or_else(|| Error::unavailable("clipboard unavailable"))
    }
}

impl Clipboard for System {
    fn get_text(&mut self) -> Result<Option<String>, Error> {
        match self.handle()?.get_text() {
            Ok(text) => Ok(Some(text)),
            // An empty clipboard surfaces as an error from `arboard`, and it is not one: the
            // user pressed paste with nothing copied, which is a no-op with a note.
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => Err(Error::unavailable(format!("clipboard read failed: {err}"))),
        }
    }

    fn set_text(&mut self, text: &str) -> Result<(), Error> {
        self.handle()?
            .set_text(text)
            .map_err(|err| Error::unavailable(format!("clipboard write failed: {err}")))
    }
}

/// A clipboard that never touches the system, for `--no-clipboard` and for tests.
///
/// Reads always report empty and writes are accepted and dropped, so every caller takes its
/// normal path without a display server in the room.
#[derive(Debug, Default)]
pub struct Disabled;

impl Clipboard for Disabled {
    fn get_text(&mut self) -> Result<Option<String>, Error> {
        Ok(None)
    }

    fn set_text(&mut self, _text: &str) -> Result<(), Error> {
        Ok(())
    }
}

/// Collects what `Ctrl+Y` wrote, so a test can assert on a copy without a clipboard.
///
/// The state is shared, not owned: the view takes a `Box<dyn Clipboard>`, and a trait object
/// cannot be downcast back to this type. Holding an `Rc` handle on the test side is the
/// straightforward way to read what the view wrote.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Recording {
    state: std::rc::Rc<std::cell::RefCell<Recorded>>,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct Recorded {
    /// The last text written, which is what a copy is observable as.
    pub written: Option<String>,
    /// What `get_text` should return, so a paste can be driven.
    pub to_read: Option<String>,
    /// How many times the clipboard was read.
    ///
    /// The startup precedence tests need to prove the clipboard was *not* consulted when
    /// `--text` or `--text-file` supplied the source, and a read count is the only way to
    /// observe "we never asked".
    pub reads: usize,
}

#[cfg(test)]
impl Recording {
    pub fn with(text: &str) -> Self {
        let recording = Self::default();
        recording.state.borrow_mut().to_read = Some(text.to_owned());
        recording
    }

    /// The last text written through this handle.
    pub fn written(&self) -> Option<String> {
        self.state.borrow().written.clone()
    }

    /// How many reads have happened.
    pub fn reads(&self) -> usize {
        self.state.borrow().reads
    }
}

#[cfg(test)]
impl Clipboard for Recording {
    fn get_text(&mut self) -> Result<Option<String>, Error> {
        let mut state = self.state.borrow_mut();
        state.reads += 1;
        Ok(state.to_read.clone())
    }

    fn set_text(&mut self, text: &str) -> Result<(), Error> {
        self.state.borrow_mut().written = Some(text.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_clipboard_reads_empty_and_swallows_writes() {
        let mut clipboard = Disabled;

        assert_eq!(clipboard.get_text().unwrap(), None);
        assert_eq!(clipboard.set_text("ignored"), Ok(()));
        // Still empty: a write to a disabled clipboard is not remembered anywhere.
        assert_eq!(clipboard.get_text().unwrap(), None);
    }

    #[test]
    fn a_recording_clipboard_keeps_the_last_write() {
        let mut clipboard = Recording::default();

        clipboard.set_text("first").unwrap();
        clipboard.set_text("second").unwrap();
        assert_eq!(clipboard.written().as_deref(), Some("second"));
    }

    #[test]
    fn a_recording_clipboard_reads_what_it_was_given() {
        let mut clipboard = Recording::with("pasted");

        assert_eq!(clipboard.get_text().unwrap().as_deref(), Some("pasted"));
    }

    #[test]
    fn an_error_carries_a_detail_apart_from_its_display_form() {
        // The status line shows a fixed sentence and the debug pane shows the detail, so the
        // two must not have to be derived from one another.
        let error = Error::unavailable("clipboard unavailable: no display");

        assert_eq!(error.detail(), "clipboard unavailable: no display");
        assert_eq!(error.to_string(), error.detail());
    }
}
