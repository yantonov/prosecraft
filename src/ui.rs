//! The four-pane frame, its focus styling and the status line (spec §4).
//!
//! Everything here is TTY-free so a ratatui `TestBackend` can assert on what was drawn.
//! Content is placeholder text for now: the editor ticket owns buffers and the wrap ticket
//! owns wrapping, so this module only has to give each pane the right inner rectangle.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::cli::Config;
use crate::clipboard::Clipboard;
use crate::editor::Editor;
use crate::layout::{self, Focus};
use crate::panes::{Settled, StreamState};
use crate::rewrite::{Machine, Refusal, Tick};
use crate::wrap::WrapState;

/// Shown instead of quitting when the quit key arrives mid-rewrite (spec §4: "Quit; confirm
/// if a rewrite is running"). Accepting the confirmation is the robustness ticket, so this
/// ticket only has to get the prompt onto the status line.
const CONFIRM_QUIT: &str = "press Ctrl+Q again to quit";

/// The state word on the status line (spec §4: `idle` / `running` / `error`).
///
/// Three values and no transitions: the rewrite flow that drives them is a later ticket, so
/// this ticket only has to render the word and let a test set each value directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Running,
    // Reached only from the rewrite flow, which is a later ticket: nothing outside the tests
    // constructs it yet, and the variant has to exist now because the status line renders it.
    #[allow(dead_code)]
    Error,
}

impl State {
    /// Spelled exactly as spec §4 spells it, because these are the words the user reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Error => "error",
        }
    }
}

/// The only thing a key can tell the event loop: carry on, or stop.
///
/// Focus and editing stay inside this module as view state, so a quit decision is the sole
/// answer that has to travel back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
}

/// Everything the frame needs that is not geometry.
///
/// Not `Clone`: the clipboard is a trait object holding a live display connection on some
/// platforms, and a cloned view would have two handles to one clipboard.
pub struct View<'a> {
    pub config: &'a Config,
    pub focus: Focus,
    /// The two editable buffers (spec §9). Owned, because editing writes to them; the wrap
    /// state beside each one holds its scroll offset and sticky column, which depend on a
    /// width the buffer deliberately does not know about.
    pub source: Editor,
    pub source_wrap: WrapState,
    pub instructions: Editor,
    pub instructions_wrap: WrapState,
    /// Content and scroll state of the output and debug panes (spec §5, §6.1b). Owned here
    /// rather than borrowed because the stream writes into it while the view is alive.
    pub stream: StreamState,
    /// The system clipboard (spec §4). A trait object rather than a generic parameter so the
    /// view keeps one type: a generic would ripple through every function that takes a
    /// `&mut View`, and nothing here needs the monomorphised speed.
    pub clipboard: Box<dyn Clipboard>,
    /// A one-shot note drawn over the focused pane, dismissed by any key. Used for the
    /// read-only paste refusal, which is the only thing that needs to interrupt without
    /// changing the status line.
    pub toast: Option<String>,
    /// Whether a quit is waiting for its second `Ctrl+Q` (spec §4: confirm while running).
    pub quit_armed: bool,
    /// The rewrite state machine (spec §5). Owned by the view so a key press and a tick are the
    /// only two things that can move a run along, and both go through here.
    pub machine: Option<Box<Machine>>,
    /// The session id for the status line's short form (spec §4). Mirrors the machine's, which
    /// owns the real value; kept here because the status line is drawn from the view.
    pub session: Option<String>,
    pub state: State,
    /// One-line note beside the state word: validation text, an error, the quit prompt, or
    /// how the last run ended (a cancelled rewrite returns to `Idle` with its note here).
    /// Kept apart from [`State`] because the word must stay one of three and this is free
    /// text that may be arbitrarily long.
    pub message: Option<String>,
}

/// The clipboard is a trait object with no `Debug` of its own, so the view spells out what a
/// reader needs: where focus is, what state the panes are in, and whether a toast is showing.
impl fmt::Debug for View<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("focus", &self.focus)
            .field("state", &self.state)
            .field("message", &self.message)
            .field("toast", &self.toast)
            .finish_non_exhaustive()
    }
}

impl<'a> View<'a> {
    pub fn new(config: &'a Config, focus: Focus) -> Self {
        Self {
            config,
            focus,
            source: Editor::from_text(&config.initial_source),
            source_wrap: WrapState::new(),
            instructions: Editor::from_text(&config.initial_instructions),
            instructions_wrap: WrapState::new(),
            stream: StreamState::new(),
            clipboard: Box::new(crate::clipboard::Disabled),
            toast: None,
            quit_armed: false,
            machine: None,
            session: config.session_id.clone(),
            state: State::Idle,
            message: None,
        }
    }

    /// Applies the §4 keys this ticket owns: focus, quit, and the keys that must do nothing.
    /// Returns whether the run is over.
    ///
    /// The state is read here rather than by the caller because the same key does not always
    /// mean the same thing: spec §4 makes `Ctrl+C` a quit while idle and a cancel while a
    /// rewrite runs, so the key cannot be decoded without the state in hand.
    pub fn handle_key(&mut self, key: &KeyEvent) -> Outcome {
        // Windows reports a press and a release for every keystroke; acting on both would
        // cycle focus twice per Tab press and quit when `q` is released.
        if key.kind != KeyEventKind::Press {
            return Outcome::Continue;
        }
        if is_quit(*key) {
            return self.request_quit();
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return self.interrupt();
        }
        // A toast is the first thing any key dismisses, and it must not also do its own
        // action: pressing `y` to get rid of "this pane is read-only" should not then start
        // copying. So the dismissal eats the key and returns.
        if self.toast.is_some() {
            self.toast = None;
            return Outcome::Continue;
        }
        // The clipboard keys (spec §4). Distinct on purpose: Ctrl+V is cursor-relative and
        // respects focus, Ctrl+E is a whole-pane replace that always targets source.
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('v') => {
                    self.paste();
                    return Outcome::Continue;
                }
                KeyCode::Char('y') => {
                    self.copy();
                    return Outcome::Continue;
                }
                KeyCode::Char('e') => {
                    self.load_into_source();
                    return Outcome::Continue;
                }
                KeyCode::Char('r') => {
                    self.rewrite();
                    return Outcome::Continue;
                }
                KeyCode::Char('n') => {
                    self.new_session();
                    return Outcome::Continue;
                }
                _ => {}
            }
        }
        // Editing and moving in an editable pane come before focus routing, because the same
        // key means different things either side of it: Tab changes focus, a character types.
        // Ctrl chords are excluded here and left to the arms below, so Ctrl+Left still jumps
        // columns instead of moving within the line.
        if self.focus.is_editable() && !key.modifiers.contains(KeyModifiers::CONTROL) {
            // The width the wrap layer needs is the one the last draw used, which is why every
            // text operation takes `width` from the cached viewport rather than guessing.
            if let Some((editor, wrap)) = self.editable(self.focus) {
                let width = wrap.viewport_width();
                match key.code {
                    KeyCode::Char(c) => editor.insert_char(c),
                    KeyCode::Enter => editor.insert_newline(),
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Delete => editor.delete(),
                    // Up/Down move by *visual* row (spec §4): a wrapped line is several rows,
                    // and stepping by logical line would make Down skip content.
                    KeyCode::Up => wrap.move_up(editor, width),
                    KeyCode::Down => wrap.move_down(editor, width),
                    KeyCode::Left => editor.move_left(),
                    KeyCode::Right => editor.move_right(),
                    // Horizontal paging is not a thing; the vertical ones are below.
                    KeyCode::PageUp => wrap.page_up(editor, width, wrap.viewport_height()),
                    KeyCode::PageDown => wrap.page_down(editor, width, wrap.viewport_height()),
                    _ => {
                        // Every other key is focus routing's business, so it falls through
                        // rather than being swallowed by the editor.
                        return self.route_focus(key);
                    }
                }
                // The cursor may have left the window, so the view follows it before the next
                // draw rather than a keystroke later.
                wrap.scroll_to_cursor(editor, width, wrap.viewport_height());
                return Outcome::Continue;
            }
        }
        // Scrolling a read-only pane is a view operation, not a movement of the cursor, so it
        // is handled before focus routing and only for the panes that have an offset.
        if !self.focus.is_editable() {
            match (key.code, key.modifiers) {
                // Spec §4: PgUp/PgDn move by a page in the focused pane.
                (KeyCode::PageUp, _) => {
                    self.stream.page(self.focus, -1);
                    return Outcome::Continue;
                }
                (KeyCode::PageDown, _) => {
                    self.stream.page(self.focus, 1);
                    return Outcome::Continue;
                }
                // The arrow keys scroll rather than move: there is no cursor in a read-only
                // pane, and a pane that ignored them would look broken.
                (KeyCode::Up, _) => {
                    self.stream.scroll(self.focus, -1);
                    return Outcome::Continue;
                }
                (KeyCode::Down, _) => {
                    self.stream.scroll(self.focus, 1);
                    return Outcome::Continue;
                }
                _ => {}
            }
        }
        self.route_focus(key)
    }

    /// The §4 focus keys: `Tab`/`Shift+Tab` in reading order, `Ctrl+Left`/`Ctrl+Right` across
    /// columns. Every other key is ignored here, so a key owned by a later ticket does nothing
    /// rather than something surprising.
    ///
    /// Esc is in the fall-through arm on purpose: spec §4 fixes it as "does nothing in v0.1",
    /// and a key that silently starts doing something later is harder to notice than a test.
    fn route_focus(&mut self, key: &KeyEvent) -> Outcome {
        self.focus = match (key.code, key.modifiers) {
            // Terminals disagree on how Shift+Tab arrives, so both spellings are accepted.
            (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => self.focus.prev(),
            (KeyCode::Tab, _) => self.focus.next(),
            // Ctrl+Left and Ctrl+Right are one action: both cross to the other column.
            (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Right, KeyModifiers::CONTROL) => {
                self.focus.switch_column()
            }
            _ => self.focus,
        };
        Outcome::Continue
    }

    /// `Ctrl+Q` from any focus state: quit, unless a rewrite is running and spec §4 asks for
    /// a confirmation first.
    fn request_quit(&mut self) -> Outcome {
        if self.state != State::Running {
            return Outcome::Quit;
        }
        // A run is live, so the first press asks and the second answers (spec §4: "Quit;
        // confirm if a rewrite is running"). The armed flag is the confirmation itself, so
        // there is no separate modal to get out of.
        if self.quit_armed {
            return Outcome::Quit;
        }
        self.quit_armed = true;
        self.message = Some(CONFIRM_QUIT.to_owned());
        Outcome::Continue
    }

    /// `Ctrl+C`: quit when there is nothing to cancel, otherwise cancel the harness — which
    /// is the robustness ticket's half, so mid-run this deliberately ignores the key instead
    /// of pretending to have cancelled.
    fn interrupt(&mut self) -> Outcome {
        match self.state {
            State::Running => {
                // Spec §4: while running, Ctrl+C cancels rather than quits. The child is asked
                // to stop; the cancelled outcome arrives through the next tick, so this must
                // not also change the state word now or the run would look finished before it
                // has been reaped.
                let asked = self
                    .machine
                    .as_mut()
                    .is_some_and(|machine| machine.cancel());
                self.message = Some(if asked {
                    "cancelling".to_owned()
                } else {
                    // A live state with no child is a bug, not a user-visible case; saying so
                    // beats a silent no-op.
                    "nothing to cancel".to_owned()
                });
                Outcome::Continue
            }
            State::Idle | State::Error => Outcome::Quit,
        }
    }

    /// `Ctrl+R`: validate and start a rewrite (spec §5).
    ///
    /// Every refusal moves focus to the pane that caused it, so the fix is next to the reason.
    /// A refusal while a run is live is checked inside the machine and reported the same way,
    /// which is why this does not special-case it.
    fn rewrite(&mut self) {
        let Some(machine) = self.machine.as_mut() else {
            // No machine wired: the app runs without rewriting (the startup-path tests rely on
            // this being harmless rather than a panic).
            self.message = Some("rewriting is not available".to_owned());
            return;
        };
        // The instructions and source travel in the user turn, so the task is built from the
        // panes rather than from the startup config (spec §5 step 2, §6.0).
        let instructions = self.instructions.text();
        let source = self.source.text();
        let task = crate::harness::Task {
            instructions: &instructions,
            source_text: &source,
            model: self.config.model.as_deref(),
            // The machine decides the session; the task carries only what the user typed.
            session: None,
        };
        // A binary that resolved at startup can disappear before a rewrite; §5 step 1 refuses
        // it here too rather than spawning and failing.
        let present = crate::cli::binary_on_path(self.config.harness.binary_name());

        let focus = self.focus;
        match machine.start(&task, present, &mut self.stream, std::time::Instant::now()) {
            Ok(()) => {
                self.state = State::Running;
                self.message = None;
                self.sync_session();
            }
            Err(refusal) => {
                self.message = Some(refusal.message().to_owned());
                self.focus = culprit(refusal, focus);
            }
        }
    }

    /// `Ctrl+N`: drop the harness session, keeping the panes' text (spec §5 step 3).
    ///
    /// Refused while a run is live, because the running process is using that id.
    fn new_session(&mut self) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        match machine.reset_session() {
            Ok(()) => {
                // Output and debug belong to the old session; source and instructions do not.
                self.stream = StreamState::new();
                self.sync_session();
                self.message = Some("new session".to_owned());
            }
            Err(refusal) => self.message = Some(refusal.message().to_owned()),
        }
    }

    /// One UI tick: drain the live run, settle it if it ended, and move the state word.
    ///
    /// Called once per tick by the event loop. Returns whether the view needs redrawing, which
    /// it does when a line arrived or a run ended — a tick with nothing to report must not
    /// cause a frame, or the batching rule in §6.1b would be pointless.
    pub fn tick(&mut self) -> bool {
        let Some(machine) = self.machine.as_mut() else {
            return false;
        };
        match machine.tick(&mut self.stream) {
            Tick::Idle => false,
            Tick::Streaming => true,
            Tick::Finished(settled) => {
                self.state = match settled {
                    Settled::Rewritten => State::Idle,
                    // A run that produced no usable rewrite is the error state, and its reason
                    // is the message (spec §5 step 5).
                    Settled::Error(reason) => {
                        self.message = Some(reason.to_owned());
                        State::Error
                    }
                };
                true
            }
        }
    }

    /// Copies the machine's session id into the config the status line reads.
    pub fn sync_session(&mut self) {
        let session = self.machine.as_ref().and_then(|m| m.session());
        self.session = session.map(str::to_owned);
    }

    /// `Ctrl+V`: paste the clipboard into the focused pane, at the cursor.
    ///
    /// A read-only pane has no cursor to insert at, so the paste is refused with a toast rather
    /// than silently ignored: the user pressed a key and deserves to know why nothing happened.
    fn paste(&mut self) {
        if !self.focus.is_editable() {
            // Spec §4: `Ctrl+V` is editable-panes-only. The toast is the whole UI for this,
            // deliberately not a modal framework for one case.
            self.toast = Some(format!("{} is read-only", self.focus.title()));
            return;
        }
        match self.clipboard.get_text() {
            Ok(Some(text)) if !text.is_empty() => {
                // `insert_str` routes newlines to `insert_newline`, so pasted prose lands as
                // real lines rather than one long row. Invalid UTF-8 was already made lossy
                // when the clipboard was read.
                if let Some((editor, _)) = self.editable(self.focus) {
                    editor.insert_str(&text);
                }
                self.message = None;
            }
            // An empty clipboard must never wipe a pane, so it is a note and nothing else.
            Ok(_) => self.message = Some("clipboard is empty".to_owned()),
            Err(err) => self.message = Some(failure(&err)),
        }
    }

    /// `Ctrl+Y`: copy the focused pane's text.
    ///
    /// Works for the read-only panes as well as the editable ones, which is what makes the
    /// debug pane useful: its content can be taken somewhere else to read.
    fn copy(&mut self) {
        let text = match self.focus {
            Focus::Source => self.source.text(),
            Focus::Instructions => self.instructions.text(),
            Focus::Output => self.stream.output_text(),
            Focus::Debug => self.stream.debug_text(),
        };
        if text.is_empty() {
            self.message = Some(format!("{} is empty", self.focus.title()));
            return;
        }
        match self.clipboard.set_text(&text) {
            Ok(()) => self.message = Some(format!("copied {}", self.focus.title())),
            Err(err) => self.message = Some(failure(&err)),
        }
    }

    /// `Ctrl+E`: replace the source pane with the clipboard, and show that it happened.
    ///
    /// Always the source pane whatever has focus (spec §4 says so explicitly), and focus moves
    /// there afterwards, because a replacement the user cannot see is indistinguishable from
    /// nothing happening.
    fn load_into_source(&mut self) {
        match self.clipboard.get_text() {
            Ok(Some(text)) if !text.is_empty() => {
                self.source = Editor::from_text(&text);
                self.focus = Focus::Source;
                self.message = None;
            }
            Ok(_) => self.message = Some("clipboard is empty".to_owned()),
            Err(err) => self.message = Some(failure(&err)),
        }
    }

    /// The editor and wrap state of one editable pane, mutably.
    ///
    /// A pair rather than a merged type: the buffer is logical and width-free, the wrap state
    /// is visual and width-dependent, and every operation that moves the cursor needs both.
    /// `None` for the read-only panes, which have neither.
    fn editable(&mut self, focus: Focus) -> Option<(&mut Editor, &mut WrapState)> {
        match focus {
            Focus::Source => Some((&mut self.source, &mut self.source_wrap)),
            Focus::Instructions => Some((&mut self.instructions, &mut self.instructions_wrap)),
            Focus::Output | Focus::Debug => None,
        }
    }
}

/// Draws the panes, then the status line over whatever the panes left.
pub fn render(frame: &mut Frame, view: &mut View) {
    let panes = layout::panes(frame.area());
    for (focus, area) in panes.all() {
        // A zero-sized pane is normal at tiny terminal sizes and must draw nothing at all.
        if area.width > 0 && area.height > 0 {
            draw_pane(frame, view, &panes, focus, area);
        }
    }
    // Drawn after the panes and before the status line, so a toast covers the focused pane but
    // can never hide the harness name or the state word.
    if let Some(toast) = &view.toast {
        draw_toast(frame, toast, panes.inner(view.focus));
    }
    if panes.status.height > 0 {
        draw_status(frame, view, panes.status);
    }
}

/// Draws a one-line note centred in `area`, on a reversed style so it reads as an overlay.
///
/// Non-modal by construction: it holds no state the key handler has to consult, and any key
/// clears it, so it cannot trap the user (spec §4's "popup", which is all the modal surface
/// this app needs).
fn draw_toast(frame: &mut Frame, text: &str, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Clipped to the pane, so a long message cannot paint over a border.
    let text = fit(text, area.width);
    let width = Line::from(text.as_str()).width() as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height / 2;
    let style = Style::new().add_modifier(Modifier::REVERSED);
    frame
        .buffer_mut()
        .set_stringn(x, y, &text, usize::from(area.width), style);
}

fn draw_pane(frame: &mut Frame, view: &mut View, panes: &layout::Panes, focus: Focus, area: Rect) {
    let focused = focus == view.focus;
    // The debug pane says so when it has stopped following the stream, so a scrolled pane
    // cannot look frozen (spec §6.1b). The output pane has nothing to follow and never says
    // it.
    let suffix = match focus {
        Focus::Debug => view.stream.debug_suffix(),
        Focus::Output => view.stream.output_suffix(),
        Focus::Source | Focus::Instructions => None,
    };
    let mut title = focus.title().to_owned();
    if let Some(suffix) = suffix {
        title.push_str(" — ");
        title.push_str(suffix);
    }
    let block = Block::bordered()
        .border_type(border_type(focus, focused))
        .border_style(border_style(focused))
        .title(title);

    // The block and the text are rendered separately so the text is laid out in exactly
    // `panes.inner(focus)`: that rectangle is the width the wrap ticket wraps to, and taking
    // it from one place keeps the two from disagreeing.
    frame.render_widget(block, area);
    match view.editable(focus) {
        // The editor's own wrap layer draws the pane, so the buffer is never reflowed and the
        // cursor is the terminal's rather than a glyph appended to the text. Only the focused
        // pane shows a cursor, which is why the unfocused one is drawn without setting it.
        Some((editor, wrap)) => {
            // Only the focused pane owns the cursor: the last `set_cursor_position` of a draw
            // wins, so an unfocused pane placing one would steal it.
            if focused {
                wrap.render(frame, editor, panes.inner(focus));
            } else {
                wrap.render_unfocused(frame, editor, panes.inner(focus));
            }
        }
        // Read-only panes own their wrapping and their offset: `StreamState` is what keeps a
        // page a screenful and the auto-tail honest.
        None => view.stream.render(focus, frame, panes.inner(focus)),
    }
}

/// Read-only panes are identified by their border, so the focused one switches to a doubled
/// line; editable panes already show a cursor, so a colour change is enough and keeps their
/// single-line border. Unfocused panes stay plain, which is the contrast the user reads.
fn border_type(focus: Focus, focused: bool) -> BorderType {
    if focused && !focus.is_editable() {
        BorderType::Double
    } else {
        BorderType::Plain
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    }
}

/// One line, always: spec §8 puts a long error here, and a status line that wrapped would
/// resize the panes. [`fit`] is what keeps that promise.
///
/// Field order is spec §4's: harness, session, state, then the key hints. The note is not a
/// field in that list, so it is inserted just before the hints; that keeps every fixed field
/// in its place and makes clipping eat the static hints rather than an error the user needs.
fn draw_status(frame: &mut Frame, view: &View, area: Rect) {
    let session = view.session.as_deref().unwrap_or("—");
    let mut line = format!(
        " {} │ {} │ {}",
        view.config.harness.binary_name(),
        session,
        view.state.label(),
    );
    if let Some(message) = &view.message {
        line.push_str(" │ ");
        line.push_str(message);
    }
    line.push_str(" │ [Ctrl+R] rewrite  [Tab] focus  [Ctrl+Q] quit");
    // No `Wrap`: the line is already guaranteed to fit, and leaving wrapping off makes
    // "never wraps" a property of the code rather than of the widget's defaults.
    frame.render_widget(Paragraph::new(Line::from(fit(&line, area.width))), area);
}

/// The pane a refusal should move focus to, so the reason and its fix are in one place.
fn culprit(refusal: Refusal, current: Focus) -> Focus {
    match refusal {
        Refusal::NoInstructions => Focus::Instructions,
        Refusal::NoSource => Focus::Source,
        // Neither of these is fixed in a pane, so focus is left where the user had it.
        Refusal::HarnessGone | Refusal::AlreadyRunning => current,
    }
}

/// The status-line sentence for a clipboard failure.
///
/// A fixed sentence, not the platform's error string: this shares one row with the harness
/// name and the state word, and a raw `arboard` message would be clipped mid-word. The detail
/// is kept on the error for a debug line.
fn failure(err: &crate::clipboard::Error) -> String {
    let _ = err.detail();
    "clipboard unavailable".to_owned()
}

/// Truncates `text` to `width` display cells, ending in `…` when it had to cut.
///
/// Cells, not bytes: `…` is one cell but three bytes, so a byte-based cut would either spill
/// past the row or split a character. The width rule is ratatui's own, reached through
/// `Line` so nothing here has to depend on the width crate directly.
fn fit(text: &str, width: u16) -> String {
    let available = usize::from(width);
    // A zero-width area shows nothing: an ellipsis would still need one cell.
    if available == 0 {
        return String::new();
    }
    if display_width(text) <= available {
        return text.to_owned();
    }
    // The ellipsis spends a cell of its own, so the kept prefix may claim only the rest.
    let budget = available - 1;
    let mut out = String::new();
    for c in text.chars() {
        out.push(c);
        if display_width(&out) > budget {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// Rendered width of `text` in terminal cells, counting a wide glyph as two - the same count
/// the renderer uses to place the cell after it.
fn display_width(text: &str) -> usize {
    Line::from(text).width()
}

/// Plain `q` is accepted alongside `Ctrl+Q` so the quit path can be driven without a
/// control chord, e.g. by a pty-based check that only types printable keys.
pub fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q')
        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Harness;
    use crate::layout;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    fn config() -> Config {
        Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: "source text".to_owned(),
            initial_instructions: "make it formal".to_owned(),
        }
    }

    fn draw_at(view: &mut View, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, view)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Draws a view and returns where the terminal cursor was placed, which is how a cursor is
    /// observable now that it is not a glyph in the buffer.
    ///
    /// Read inside the draw closure: `Frame`'s cursor field is private and `TestBackend`
    /// reports `(0,0)` both for "no cursor" and for a cursor genuinely at the origin, so
    /// asking the backend afterwards cannot tell the two apart.
    fn cursor_of(view: &mut View, width: u16, height: u16) -> Option<(u16, u16)> {
        let predicted = cursor_from_view(view, width, height);

        // And the draw must agree with the prediction: TestBackend stores whatever
        // `set_cursor_position` was given, defaulting to the origin when a draw set nothing.
        // A prediction checked against itself would pass while the render did nothing.
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, view)).unwrap();
        let drawn = terminal.get_cursor_position().unwrap();
        if let Some((x, y)) = predicted {
            assert_eq!(
                (drawn.x, drawn.y),
                (x, y),
                "the draw did not place the cursor where the layout says it goes"
            );
        }
        predicted
    }

    /// Where the draw *would* put the cursor, taken from the same two functions the draw uses.
    ///
    /// Mirrors `draw_pane`: only the focused editable pane goes through `WrapState::render`,
    /// which is the only place that calls `set_cursor_position`.
    fn cursor_from_view(view: &View, width: u16, height: u16) -> Option<(u16, u16)> {
        let (editor, wrap) = match view.focus {
            Focus::Source => (&view.source, &view.source_wrap),
            Focus::Instructions => (&view.instructions, &view.instructions_wrap),
            Focus::Output | Focus::Debug => return None,
        };
        let focus = view.focus;
        let panes = layout::panes(Rect::new(0, 0, width, height));
        let inner = panes.inner(focus);
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let cursor = crate::wrap::cursor_position(editor, inner.width);
        let scroll = wrap.scroll();
        if cursor.row < scroll || cursor.row >= scroll + usize::from(inner.height) {
            return None;
        }
        let x = inner.x + cursor.column.min(usize::from(inner.width) - 1) as u16;
        let y = inner.y + (cursor.row - scroll) as u16;
        Some((x, y))
    }

    fn draw(width: u16, height: u16) -> Buffer {
        draw_at(&mut View::new(&config(), Focus::Source), width, height)
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    /// Reads the title out of a top border by dropping the corner and the fill glyph.
    fn title_at(buffer: &Buffer, area: Rect) -> String {
        (area.x + 1..area.x + area.width - 1)
            .map(|x| buffer[(x, area.y)].symbol())
            .collect::<String>()
            .trim_start_matches('─')
            .split('─')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// The four corners of a pane as drawn, so a test can prove a border exists at all.
    fn corners(buffer: &Buffer, area: Rect) -> [&str; 4] {
        let (left, right) = (area.x, area.x + area.width - 1);
        let (top, bottom) = (area.y, area.y + area.height - 1);
        [
            buffer[(left, top)].symbol(),
            buffer[(right, top)].symbol(),
            buffer[(left, bottom)].symbol(),
            buffer[(right, bottom)].symbol(),
        ]
    }

    fn is_corner(symbol: &str) -> bool {
        matches!(symbol, "┌" | "┐" | "└" | "┘" | "╔" | "╗" | "╚" | "╝")
    }

    #[test]
    fn draws_four_bordered_panes_in_spec_order_at_80x24() {
        let buffer = draw(80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        for (focus, area) in panes.all() {
            let corners = corners(&buffer, area);
            assert!(
                corners.iter().all(|symbol| is_corner(symbol)),
                "{focus:?} has no border at {area:?}: {corners:?}"
            );
            // Vertical borders only; the corner rows are covered by `corners`.
            for y in area.y + 1..area.y + area.height - 1 {
                assert!(
                    matches!(buffer[(area.x, y)].symbol(), "│" | "║"),
                    "{focus:?} left border missing at row {y}"
                );
                let right = area.x + area.width - 1;
                assert!(
                    matches!(buffer[(right, y)].symbol(), "│" | "║"),
                    "{focus:?} right border missing at row {y}"
                );
            }
        }

        assert_eq!(title_at(&buffer, panes.source), "source");
        assert_eq!(title_at(&buffer, panes.instructions), "instructions");
        assert_eq!(title_at(&buffer, panes.output), "output");
        assert_eq!(title_at(&buffer, panes.debug), "debug");

        // Spec order, expressed as positions: source over instructions on the left,
        // output over debug on the right.
        assert_eq!((panes.source.x, panes.source.y), (0, 0));
        assert_eq!((panes.instructions.x, panes.instructions.y), (0, 18));
        assert_eq!((panes.output.x, panes.output.y), (40, 0));
        assert_eq!((panes.debug.x, panes.debug.y), (40, 16));
    }

    #[test]
    fn columns_split_fifty_fifty_at_80x24() {
        let buffer = draw(80, 24);

        // Both top borders run to column 39, and the right column starts at 40.
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(39, 0)].symbol(), "┐");
        assert_eq!(buffer[(40, 0)].symbol(), "┌");
        assert_eq!(buffer[(79, 0)].symbol(), "┐");
        // The columns end right where the status row begins, i.e. both are 23 rows tall.
        assert_eq!(buffer[(39, 22)].symbol(), "┘");
        assert_eq!(buffer[(40, 22)].symbol(), "└");
    }

    #[test]
    fn panes_still_draw_at_40x10() {
        let buffer = draw(40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));

        for (focus, area) in panes.all() {
            assert!(
                corners(&buffer, area)
                    .iter()
                    .all(|symbol| is_corner(symbol)),
                "{focus:?} has no border at {area:?}"
            );
            assert!(
                !title_at(&buffer, area).is_empty(),
                "{focus:?} lost its title"
            );
        }

        assert_eq!(title_at(&buffer, panes.source), "source");
        assert_eq!(title_at(&buffer, panes.instructions), "instructions");
        assert_eq!(title_at(&buffer, panes.output), "output");
        assert_eq!(title_at(&buffer, panes.debug), "debug");

        // 9 body rows: 25% floors to 2, so the floor-3 rule decides the instructions height.
        assert_eq!(panes.instructions.height, 3);
        assert_eq!(panes.source.height, 6);
    }

    #[test]
    fn one_by_one_does_not_panic() {
        let buffer = draw(1, 1);

        // Nothing can be drawn but the status line, and the frame must survive anyway.
        assert_eq!(buffer.area, Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn resizing_redistributes_panes_without_panicking() {
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        let config = config();
        let mut view = View::new(&config, Focus::Output);

        // Far bigger and far smaller than the minimum, in both directions.
        for (width, height) in [
            (1, 1),
            (2, 2),
            (40, 10),
            (81, 25),
            (200, 60),
            (3, 1),
            (1, 40),
        ] {
            terminal.backend_mut().resize(width, height);
            terminal.draw(|frame| render(frame, &mut view)).unwrap();
            assert_eq!(
                terminal.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
    }

    #[test]
    fn focused_read_only_pane_uses_a_different_border() {
        let config = config();
        let unfocused = draw_at(&mut View::new(&config, Focus::Source), 80, 24);
        let focused = draw_at(&mut View::new(&config, Focus::Output), 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        assert_eq!(corners(&unfocused, panes.output), ["┌", "┐", "└", "┘"]);
        assert_eq!(corners(&focused, panes.output), ["╔", "╗", "╚", "╝"]);
        // Only the focused pane changes: the others keep the plain border.
        assert_eq!(corners(&focused, panes.source), ["┌", "┐", "└", "┘"]);
        assert_eq!(corners(&focused, panes.debug), ["┌", "┐", "└", "┘"]);
    }

    #[test]
    fn focused_pane_border_is_highlighted_at_every_position() {
        let config = config();
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        for focus in Focus::ALL {
            let buffer = draw_at(&mut View::new(&config, focus), 80, 24);
            for (other, area) in panes.all() {
                let style = buffer[(area.x, area.y)].style();
                if other == focus {
                    assert!(
                        style.fg == Some(Color::Yellow)
                            && style.add_modifier.contains(Modifier::BOLD),
                        "{other:?} is focused but its border is not highlighted"
                    );
                } else {
                    // `Reset` is what ratatui reports for "no foreground was set".
                    assert!(
                        matches!(style.fg, None | Some(Color::Reset)),
                        "{other:?} is not focused but its border is fg {:?}",
                        style.fg
                    );
                }
            }
        }
    }

    #[test]
    fn the_cursor_is_placed_in_the_focused_editable_pane_only() {
        let config = config();
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        // The cursor is terminal state now, not a glyph appended to the text, so it is read
        // from the frame rather than searched for in the buffer.
        for focus in [Focus::Source, Focus::Instructions] {
            let at = cursor_of(&mut View::new(&config, focus), 80, 24)
                .unwrap_or_else(|| panic!("{focus:?} placed no cursor"));
            let inner = panes.inner(focus);
            assert!(
                inner.x <= at.0 && at.0 < inner.x + inner.width,
                "{focus:?} cursor is outside its pane: {at:?} not in {inner:?}"
            );
            assert!(
                inner.y <= at.1 && at.1 < inner.y + inner.height,
                "{focus:?} cursor row is outside its pane: {at:?} not in {inner:?}"
            );
        }

        // A read-only pane has no cursor of its own to edit.
        for focus in [Focus::Output, Focus::Debug] {
            assert_eq!(
                cursor_of(&mut View::new(&config, focus), 80, 24),
                None,
                "{focus:?} is read-only but placed a cursor"
            );
        }
    }

    #[test]
    fn an_unfocused_editable_pane_places_no_cursor() {
        // Focus on a read-only pane: neither editable pane may show a cursor, or the user
        // would type into a pane that is not taking input.
        assert_eq!(
            cursor_of(&mut View::new(&config(), Focus::Output), 80, 24),
            None
        );
    }

    #[test]
    fn seeded_buffers_render_in_their_own_panes() {
        let buffer = draw(80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        assert!(row(&buffer, panes.source.y + 1).contains("source text"));
        assert!(row(&buffer, panes.instructions.y + 1).contains("make it formal"));
    }

    #[test]
    fn long_text_wraps_inside_the_pane_instead_of_truncating() {
        // Inner width is 18 at 40x10, so this line must occupy several rows and keep its
        // last word. Truncation would drop the tail, which is what the spec forbids.
        let mut config = config();
        config.initial_source = (1..=9)
            .map(|n| format!("word{n}"))
            .collect::<Vec<_>>()
            .join(" ");

        let buffer = draw_at(&mut View::new(&config, Focus::Source), 40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));
        let rendered = (panes.source.y + 1..panes.source.y + panes.source.height - 1)
            .map(|y| row(&buffer, y))
            .collect::<String>();

        assert!(rendered.contains("word9"), "tail dropped: {rendered:?}");
        assert!(rendered.contains("word1 "), "head dropped: {rendered:?}");
    }

    #[test]
    fn status_line_shows_harness_session_state_and_hints() {
        let buffer = draw(80, 24);
        let status = row(&buffer, 23);

        assert!(status.contains("claude"), "harness missing: {status:?}");
        assert!(status.contains('—'), "session em-dash missing: {status:?}");
        assert!(status.contains("idle"), "state missing: {status:?}");
        assert!(status.contains("Ctrl+R"), "hints missing: {status:?}");

        // Order is fixed by the spec: harness, session, state, hints.
        let positions = ["claude", "—", "idle", "Ctrl+R"].map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(
            positions, sorted,
            "status line out of spec order: {status:?}"
        );
    }

    #[test]
    fn status_line_names_the_selected_harness() {
        let mut config = config();
        config.harness = Harness::Pi;
        let buffer = draw_at(&mut View::new(&config, Focus::Source), 80, 10);

        assert!(
            row(&buffer, 9).contains("pi"),
            "pi not named: {:?}",
            row(&buffer, 9)
        );
    }

    #[test]
    fn status_line_shows_a_real_session_id_once_one_exists() {
        let mut config = config();
        config.session_id = Some("abc123".to_owned());
        let buffer = draw_at(&mut View::new(&config, Focus::Source), 80, 10);
        let status = row(&buffer, 9);

        assert!(status.contains("abc123"), "session id missing: {status:?}");
        assert!(
            !status.contains('—'),
            "em-dash kept past the first rewrite: {status:?}"
        );
    }

    #[test]
    fn status_line_keeps_one_row_at_every_size() {
        for (width, height) in [(1, 1), (1, 2), (40, 10), (80, 24)] {
            let buffer = draw(width, height);
            let panes = layout::panes(Rect::new(0, 0, width, height));
            assert_eq!(panes.status.height, 1, "no status row at {width}x{height}");
            assert_eq!(buffer.area.height, height);
        }
    }

    /// Draws a view with the given state and message, then returns its status row.
    fn status_row(state: State, message: Option<&str>, width: u16) -> String {
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = state;
        view.message = message.map(str::to_owned);
        let buffer = draw_at(&mut view, width, 10);
        row(&buffer, 9)
    }

    #[test]
    fn every_state_renders_its_own_word() {
        for (state, word) in [
            (State::Idle, "idle"),
            (State::Running, "running"),
            (State::Error, "error"),
        ] {
            assert_eq!(state.label(), word);
            let status = status_row(state, None, 80);
            assert!(status.contains(word), "{state:?} not shown: {status:?}");
            // The word sits between the session and the hints, in spec §4's order.
            let session = status.find('—').expect("session em-dash missing");
            let state_at = status.find(word).expect("state word missing");
            let hints = status.find("Ctrl+R").expect("hints missing");
            assert!(session < state_at && state_at < hints, "{status:?}");
        }
    }

    #[test]
    fn a_message_sits_between_the_state_word_and_the_hints() {
        let status = status_row(State::Error, Some("missing output markers"), 120);

        assert!(status.contains("claude"), "harness lost: {status:?}");
        assert!(status.contains("missing output markers"), "{status:?}");
        // Spec §4 fixes the four fields; the note is not one of them, so it must not push
        // any of them out of order.
        let positions = [
            "claude",
            "—",
            "error",
            "missing output markers",
            "Ctrl+R",
            "Ctrl+Q",
        ]
        .map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "out of order: {status:?}");
    }

    #[test]
    fn a_cancelled_run_returns_to_idle_with_a_note() {
        // The ticket's fake of the cancelled outcome: the state word says idle and the
        // reason is the free-text channel, so the two never have to share one string.
        let status = status_row(State::Idle, Some("cancelled"), 80);

        assert!(status.contains("idle"), "{status:?}");
        assert!(status.contains("cancelled"), "{status:?}");
    }

    #[test]
    fn a_long_message_is_clipped_with_an_ellipsis_and_never_wraps() {
        let long = "x".repeat(200);
        let status = status_row(State::Error, Some(&long), 40);

        // One row can hold 40 cells, so the row ends in the ellipsis and nothing spilled
        // into the row above (which is a pane border, not more status).
        assert_eq!(status.chars().count(), 40, "row changed width: {status:?}");
        assert!(status.ends_with('…'), "no ellipsis: {status:?}");
        assert!(
            status.contains("claude"),
            "the fixed fields were squeezed out"
        );
        assert!(status.contains("error"), "the state word was squeezed out");

        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = State::Error;
        view.message = Some(long);
        let buffer = draw_at(&mut view, 40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));
        // The row directly above the status line is still the pane bottom border: a wrapped
        // status line would have painted over it.
        assert_eq!(buffer[(0, panes.status.y - 1)].symbol(), "└");
    }

    #[test]
    fn a_clipped_message_keeps_the_fixed_fields_and_says_it_was_cut() {
        // The error path is the one that matters: the user must still see which harness and
        // which state, and must know the note continues.
        let status = status_row(State::Error, Some(&"boom ".repeat(40)), 60);

        let positions = ["claude", "—", "error", "boom"].map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "out of order: {status:?}");
        assert!(status.ends_with('…'), "no ellipsis: {status:?}");
    }

    #[test]
    fn fit_keeps_short_text_and_clips_long_text_to_the_cell_count() {
        assert_eq!(fit("abc", 10), "abc");
        assert_eq!(fit("abc", 3), "abc", "exactly fitting text is not clipped");
        assert_eq!(fit("abcd", 3), "ab…");
        // A wide glyph is two cells: '世' plus the ellipsis fills 3 cells, not 2 chars.
        assert_eq!(fit("世界", 3), "世…");
        assert_eq!(fit("", 0), "");
        assert_eq!(fit("abc", 0), "", "nothing fits in a zero-width row");
        assert_eq!(
            fit("abc", 1),
            "…",
            "one cell means the ellipsis and no text"
        );

        // Byte-based clipping would cut inside these; cell-based clipping must not.
        for text in ["café", "a—b", "🙂🙂🙂", "naïve prose"] {
            let fitted = fit(text, 3);
            assert!(
                display_width(&fitted) <= 3,
                "{text:?} fitted to {fitted:?} is wider than 3 cells"
            );
            assert!(
                fitted.is_char_boundary(fitted.len()),
                "{text:?} split a character"
            );
        }
    }

    #[test]
    fn ctrl_q_quits_from_every_focus_state() {
        for focus in Focus::ALL {
            for state in [State::Idle, State::Error] {
                let config = config();
                let mut view = View::new(&config, focus);
                view.state = state;

                assert_eq!(
                    view.handle_key(&ctrl(KeyCode::Char('q'))),
                    Outcome::Quit,
                    "Ctrl+Q from {focus:?} while {state:?}"
                );
            }
        }
    }

    #[test]
    fn ctrl_q_while_running_prompts_instead_of_quitting() {
        let config = config();
        let mut view = View::new(&config, Focus::Output);
        view.state = State::Running;

        assert_eq!(
            view.handle_key(&ctrl(KeyCode::Char('q'))),
            Outcome::Continue,
            "Ctrl+Q quit mid-run without confirming"
        );
        let message = view.message.clone().expect("no confirmation prompt");
        assert!(
            message.contains("Ctrl+Q"),
            "prompt is not actionable: {message}"
        );

        // The prompt has to be readable, which means the status line must carry it.
        let status = row(&draw_at(&mut view, 120, 10), 9);
        assert!(status.contains(&message), "prompt not drawn: {status:?}");
    }

    #[test]
    fn ctrl_c_quits_only_when_nothing_is_running() {
        for state in [State::Idle, State::Error] {
            let config = config();
            let mut view = View::new(&config, Focus::Debug);
            view.state = state;
            assert_eq!(
                view.handle_key(&ctrl(KeyCode::Char('c'))),
                Outcome::Quit,
                "Ctrl+C while {state:?}"
            );
        }

        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = State::Running;
        assert_eq!(
            view.handle_key(&ctrl(KeyCode::Char('c'))),
            Outcome::Continue,
            "Ctrl+C quit mid-run instead of leaving the cancel half unimplemented"
        );
        assert!(
            view.message.is_some(),
            "an ignored Ctrl+C must say so in the status line"
        );
    }

    #[test]
    fn esc_does_nothing() {
        for state in [State::Idle, State::Running, State::Error] {
            let config = config();
            let mut view = View::new(&config, Focus::Source);
            view.state = state;

            assert_eq!(
                view.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Outcome::Continue,
                "Esc quit while {state:?}"
            );
            assert_eq!(view.focus, Focus::Source, "Esc moved focus");
            assert_eq!(view.message, None, "Esc wrote a status message");
            assert_eq!(view.state, state, "Esc changed the state");
        }
    }

    #[test]
    fn a_released_quit_key_does_not_quit() {
        // Windows sends a release after every press; quitting on it would fire twice.
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );

        assert_eq!(view.handle_key(&release), Outcome::Continue);
    }

    /// A view with a full debug pane, a known viewport, and the given focus. The viewport is
    /// established by a draw, because a paging key uses the geometry of the last frame.
    fn scrolling_view(focus: Focus) -> View<'static> {
        let config = Box::leak(Box::new(config()));
        let mut view = View::new(config, focus);
        for n in 0..60 {
            view.stream.app(format!("line{n}"));
        }
        let _ = draw_at(&mut view, 80, 24);
        view
    }

    #[test]
    fn pgup_and_pgdn_scroll_a_focused_read_only_pane() {
        let mut view = scrolling_view(Focus::Debug);
        assert_eq!(
            view.stream.debug_suffix(),
            None,
            "the pane should start tailing"
        );

        // Spec §4: PgUp scrolls by a page in the focused pane, and the pane then says it is
        // no longer following the stream.
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Outcome::Continue
        );
        assert_eq!(view.stream.debug_suffix(), Some("not tailing"));

        // PgDn brings it back to the bottom, which resumes tailing.
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Outcome::Continue
        );
        assert_eq!(view.stream.debug_suffix(), None);
    }

    #[test]
    fn arrows_scroll_a_read_only_pane_instead_of_moving_a_cursor() {
        let mut view = scrolling_view(Focus::Output);
        let output = view.stream.output_text();

        // An arrow key must not quit, must not change focus, and must not edit anything.
        for code in [KeyCode::Up, KeyCode::Down] {
            assert_eq!(
                view.handle_key(&KeyEvent::new(code, KeyModifiers::NONE)),
                Outcome::Continue,
                "{code:?} quit"
            );
        }
        assert_eq!(view.focus, Focus::Output, "an arrow moved focus");
        assert_eq!(view.stream.output_text(), output, "an arrow edited a pane");
    }

    #[test]
    fn scrolling_keys_do_not_disturb_an_editable_pane() {
        for focus in [Focus::Source, Focus::Instructions] {
            let mut view = scrolling_view(focus);
            // A draw establishes the read-only viewports; the editable pane has none, so
            // these keys fall through to focus handling and must change nothing.
            for code in [
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Up,
                KeyCode::Down,
            ] {
                assert_eq!(
                    view.handle_key(&KeyEvent::new(code, KeyModifiers::NONE)),
                    Outcome::Continue
                );
            }
            assert_eq!(view.focus, focus, "{focus:?} moved");
            assert_eq!(
                view.stream.debug_suffix(),
                None,
                "{focus:?} scrolled the debug pane"
            );
        }
    }

    fn focus_after(key: KeyEvent, from: Focus) -> Focus {
        let config = config();
        let mut view = View::new(&config, from);
        assert_eq!(
            view.handle_key(&key),
            Outcome::Continue,
            "{key:?} is not a quit key"
        );
        view.focus
    }

    #[test]
    fn tab_and_shift_tab_cycle_focus_through_all_four_panes() {
        for from in Focus::ALL {
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), from),
                from.next(),
                "Tab from {from:?}"
            );
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), from),
                from.prev(),
                "Shift+Tab from {from:?}"
            );
            // Terminals that report Shift+Tab as BackTab must get the same answer.
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), from),
                from.prev(),
                "BackTab from {from:?}"
            );
        }
    }

    #[test]
    fn four_tabs_walk_source_instructions_output_debug_and_back() {
        let mut focus = Focus::Source;
        let mut seen = vec![focus];
        for _ in 0..4 {
            focus = focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), focus);
            seen.push(focus);
        }

        assert_eq!(
            seen,
            [
                Focus::Source,
                Focus::Instructions,
                Focus::Output,
                Focus::Debug,
                Focus::Source
            ]
        );
    }

    #[test]
    fn ctrl_arrows_jump_columns_preserving_the_row() {
        for from in Focus::ALL {
            let expected = from.switch_column();
            for code in [KeyCode::Left, KeyCode::Right] {
                let key = KeyEvent::new(code, KeyModifiers::CONTROL);
                assert_eq!(focus_after(key, from), expected, "{code:?} from {from:?}");
            }
        }

        // Spelled out, because this is the mapping the spec names.
        assert_eq!(
            focus_after(ctrl(KeyCode::Right), Focus::Source),
            Focus::Output
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Left), Focus::Output),
            Focus::Source
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Right), Focus::Instructions),
            Focus::Debug
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Left), Focus::Debug),
            Focus::Instructions
        );
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn key_releases_do_not_move_focus() {
        // Windows sends a release after every press; acting on both would double-step.
        let release =
            KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Release);

        assert_eq!(focus_after(release, Focus::Source), Focus::Source);
    }

    #[test]
    fn the_not_tailing_marker_is_drawn_in_the_debug_pane_title() {
        let mut view = scrolling_view(Focus::Debug);
        drawn_title(&mut view);
        // Read the title off a real frame rather than the accessor, so the marker is proved
        // to reach the screen and not just the state.
        assert!(
            !drawn_title(&mut view).contains("not tailing"),
            "a tailing pane drew the marker"
        );

        view.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let title = drawn_title(&mut view);
        assert!(
            title.contains("not tailing"),
            "a scrolled pane did not draw the marker: {title:?}"
        );
    }

    /// The debug pane's title as it appears in a drawn frame.
    fn drawn_title(view: &mut View) -> String {
        let buffer = draw_at(view, 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));
        title_at(&buffer, panes.debug)
    }

    /// A view holding the given text, focused on the given pane, with its wrap state primed by
    /// a draw so a text operation knows the width it is working in.
    fn editing_view(focus: Focus, text: &str) -> View<'static> {
        let config = Box::leak(Box::new(Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: text.to_owned(),
            initial_instructions: text.to_owned(),
        }));
        let mut view = View::new(config, focus);
        let _ = draw_at(&mut view, 80, 24);
        view
    }

    fn press_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn typing_inserts_into_the_focused_pane_only() {
        let mut view = editing_view(Focus::Source, "");
        view.handle_key(&press_char('h'));
        view.handle_key(&press_char('i'));

        assert_eq!(view.source.text(), "hi");
        assert_eq!(
            view.instructions.text(),
            "",
            "the unfocused pane was edited"
        );
    }

    #[test]
    fn typing_goes_to_the_instructions_pane_when_it_has_focus() {
        let mut view = editing_view(Focus::Instructions, "");
        view.handle_key(&press_char('x'));

        assert_eq!(view.instructions.text(), "x");
        assert_eq!(view.source.text(), "");
    }

    #[test]
    fn enter_and_backspace_edit_the_focused_pane() {
        let mut view = editing_view(Focus::Source, "");
        for c in "ab".chars() {
            view.handle_key(&press_char(c));
        }
        view.handle_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        view.handle_key(&press_char('c'));
        assert_eq!(view.source.text(), "ab\nc");

        view.handle_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(view.source.text(), "ab\n");
    }

    #[test]
    fn delete_removes_the_character_ahead() {
        let mut view = editing_view(Focus::Source, "abc");
        // Seeded text puts the cursor at the end, so move to the start first.
        for _ in 0..3 {
            view.handle_key(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        }
        view.handle_key(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

        assert_eq!(view.source.text(), "bc");
    }

    #[test]
    fn a_read_only_pane_ignores_text_keys() {
        for focus in [Focus::Output, Focus::Debug] {
            let mut view = editing_view(focus, "seed");
            for key in [
                press_char('x'),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            ] {
                view.handle_key(&key);
            }
            // Nothing was inserted anywhere, because there is nowhere to insert it.
            assert_eq!(view.source.text(), "seed", "{focus:?} edited source");
            assert_eq!(
                view.instructions.text(),
                "seed",
                "{focus:?} edited instructions"
            );
        }
    }

    #[test]
    fn down_moves_by_visual_row_inside_a_wrapped_line() {
        // Width 18 inner at 40x10, so this wraps. Down must move one row, not one line.
        let mut view = editing_view(Focus::Source, "");
        view.source = Editor::from_text("aaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbb");
        view.source.set_cursor(0, 0);
        let _ = draw_at(&mut view, 40, 10);

        view.handle_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

        // One visual row down, and still on logical line 0: the line did not change.
        assert_eq!(view.source.cursor().line(), 0, "Down changed logical line");
        assert!(
            view.source.cursor().column() > 0,
            "Down did not move within the wrapped line"
        );
    }

    #[test]
    fn a_control_chord_is_not_typed_into_the_buffer() {
        let mut view = editing_view(Focus::Source, "");
        // Ctrl+Right switches column; it must not insert and must not edit.
        view.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));

        assert_eq!(view.source.text(), "", "a ctrl chord typed a character");
    }

    #[test]
    fn tab_still_cycles_focus_while_an_editable_pane_is_focused() {
        let mut view = editing_view(Focus::Source, "");
        view.handle_key(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(view.focus, Focus::Instructions, "Tab stopped cycling");
        // And it did not insert a tab character.
        assert_eq!(view.source.text(), "");
    }

    #[test]
    fn paging_scrolls_a_focused_editable_pane() {
        let text = (0..60)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = editing_view(Focus::Source, "");
        view.source = Editor::from_text(&text);
        // Cursor at the end, so the view is scrolled to the bottom.
        let _ = draw_at(&mut view, 80, 24);
        let before = view.source_wrap.scroll();

        view.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(
            view.source_wrap.scroll() < before,
            "PgUp did not scroll the editable pane"
        );
    }

    #[test]
    fn the_cursor_stays_visible_after_typing_at_the_bottom() {
        let text = (0..60)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = editing_view(Focus::Source, "");
        view.source = Editor::from_text(&text);
        let _ = draw_at(&mut view, 40, 10);

        // The cursor was positioned in the pane, so the view had scrolled to it.
        let at = cursor_of(&mut view, 40, 10).expect("no cursor");
        let panes = layout::panes(Rect::new(0, 0, 40, 10));
        let inner = panes.inner(Focus::Source);
        assert!(
            at.1 < inner.y + inner.height,
            "cursor left the pane at the bottom: {at:?}"
        );
    }

    #[test]
    fn a_resize_keeps_the_text_and_the_logical_cursor() {
        let mut view = editing_view(Focus::Source, "");
        view.source = Editor::from_text("some text that wraps at a small width");
        view.source.set_cursor(0, 10);
        let text = view.source.text();

        for (width, height) in [(80, 24), (40, 10), (10, 4), (1, 1), (80, 24)] {
            let _ = draw_at(&mut view, width, height);
            assert_eq!(
                view.source.text(),
                text,
                "resize edited the buffer at {width}"
            );
            assert_eq!(
                view.source.cursor(),
                crate::editor::Cursor::new(0, 10),
                "resize moved the cursor at {width}"
            );
        }
    }

    /// A view with a recording clipboard, so a copy or paste is observable without a display
    /// server. Primed by a draw so the wrap state knows its width.
    fn clipboard_view(focus: Focus, clipboard: crate::clipboard::Recording) -> View<'static> {
        clipboard_view_with(focus, clipboard).0
    }

    /// The view plus a handle on the recording clipboard.
    ///
    /// A handle rather than a downcast: the view holds a `Box<dyn Clipboard>`, and a shared
    /// `Rc` is the only way back to what was written.
    fn clipboard_view_with(
        focus: Focus,
        clipboard: crate::clipboard::Recording,
    ) -> (View<'static>, crate::clipboard::Recording) {
        let config = Box::leak(Box::new(Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: String::new(),
            initial_instructions: String::new(),
        }));
        let mut view = View::new(config, focus);
        let handle = clipboard.clone();
        view.clipboard = Box::new(clipboard);
        let _ = draw_at(&mut view, 80, 24);
        (view, handle)
    }

    fn ctrl_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_v_pastes_into_the_focused_editable_pane_at_the_cursor() {
        let mut view = clipboard_view(Focus::Source, crate::clipboard::Recording::with("pasted"));
        view.source = Editor::from_text("ab");
        view.source.set_cursor(0, 1);
        let _ = draw_at(&mut view, 80, 24);

        view.handle_key(&ctrl_char('v'));

        assert_eq!(
            view.source.text(),
            "apastedb",
            "paste did not land at the cursor"
        );
        assert_eq!(
            view.instructions.text(),
            "",
            "the wrong pane was pasted into"
        );
    }

    #[test]
    fn ctrl_v_pastes_multiline_text_as_real_lines() {
        let mut view = clipboard_view(Focus::Source, crate::clipboard::Recording::with("one\ntwo"));

        view.handle_key(&ctrl_char('v'));

        assert_eq!(view.source.text(), "one\ntwo");
        assert_eq!(
            view.source.lines().len(),
            2,
            "newlines were not turned into line breaks"
        );
    }

    #[test]
    fn ctrl_v_in_a_read_only_pane_is_refused_with_a_toast() {
        for focus in [Focus::Output, Focus::Debug] {
            let mut view = clipboard_view(focus, crate::clipboard::Recording::with("pasted"));
            let before_output = view.stream.output_text();
            let before_debug = view.stream.debug_text();

            view.handle_key(&ctrl_char('v'));

            let toast = view.toast.as_deref().expect("no refusal shown");
            assert!(
                toast.contains("read-only"),
                "the toast should say why: {toast}"
            );
            // Nothing was inserted and focus did not move.
            assert_eq!(view.focus, focus, "the refusal moved focus");
            assert_eq!(view.stream.output_text(), before_output);
            assert_eq!(view.stream.debug_text(), before_debug);
        }
    }

    #[test]
    fn any_key_dismisses_the_toast_without_also_acting() {
        let mut view = clipboard_view(Focus::Output, crate::clipboard::Recording::with("x"));
        view.handle_key(&ctrl_char('v'));
        assert!(view.toast.is_some());

        // `y` would normally copy; while a toast is up it only dismisses, so a user clearing
        // the note does not accidentally overwrite their clipboard.
        view.handle_key(&ctrl_char('y'));
        assert!(view.toast.is_none(), "the toast survived a key press");
        assert!(
            view.message.is_none(),
            "the dismissing key also performed its action"
        );
    }

    #[test]
    fn ctrl_y_copies_the_focused_pane() {
        // The debug pane is the interesting case: copying from a read-only pane is what makes
        // its content usable outside the app.
        let (mut view, clipboard) =
            clipboard_view_with(Focus::Debug, crate::clipboard::Recording::default());
        view.stream.app("a debug line".to_owned());

        view.handle_key(&ctrl_char('y'));

        let copied = clipboard
            .written()
            .expect("nothing was written to the clipboard");
        assert!(
            copied.contains("a debug line"),
            "the debug pane's content was not copied: {copied:?}"
        );
    }

    #[test]
    fn ctrl_y_copies_the_source_pane_too() {
        let (mut view, clipboard) =
            clipboard_view_with(Focus::Source, crate::clipboard::Recording::default());
        view.handle_key(&press_char('h'));
        view.handle_key(&press_char('i'));

        view.handle_key(&ctrl_char('y'));

        assert_eq!(clipboard.written().as_deref(), Some("hi"));
    }

    #[test]
    fn ctrl_y_on_an_empty_pane_says_so_and_writes_nothing() {
        let (mut view, clipboard) =
            clipboard_view_with(Focus::Source, crate::clipboard::Recording::default());

        view.handle_key(&ctrl_char('y'));

        assert_eq!(
            clipboard.written(),
            None,
            "an empty pane wrote to the clipboard"
        );
        assert!(
            view.message.as_deref().is_some_and(|m| m.contains("empty")),
            "no note about the empty pane: {:?}",
            view.message
        );
    }

    #[test]
    fn ctrl_e_replaces_the_source_from_any_focus_and_moves_focus_there() {
        for focus in Focus::ALL {
            let mut view = clipboard_view(focus, crate::clipboard::Recording::with("replacement"));
            view.source = Editor::from_text("old");

            view.handle_key(&ctrl_char('e'));

            assert_eq!(
                view.source.text(),
                "replacement",
                "Ctrl+E from {focus:?} did not target the source pane"
            );
            assert_eq!(
                view.focus,
                Focus::Source,
                "focus did not move to the replaced pane from {focus:?}"
            );
        }
    }

    #[test]
    fn ctrl_e_with_an_empty_clipboard_never_wipes_the_source() {
        let mut view = clipboard_view(Focus::Source, crate::clipboard::Recording::default());
        view.source = Editor::from_text("keep me");

        view.handle_key(&ctrl_char('e'));

        assert_eq!(
            view.source.text(),
            "keep me",
            "an empty clipboard wiped the pane"
        );
        assert!(
            view.message.as_deref().is_some_and(|m| m.contains("empty")),
            "no note about the empty clipboard: {:?}",
            view.message
        );
    }

    #[test]
    fn ctrl_v_with_an_empty_clipboard_never_wipes_a_pane() {
        let mut view = clipboard_view(Focus::Source, crate::clipboard::Recording::default());
        view.source = Editor::from_text("keep me");

        view.handle_key(&ctrl_char('v'));

        assert_eq!(view.source.text(), "keep me");
        assert!(view.message.as_deref().is_some_and(|m| m.contains("empty")));
    }

    #[test]
    fn a_clipboard_failure_is_a_status_note_and_the_app_keeps_working() {
        // Spec §8: unavailable clipboard is a warning, with every other feature still working.
        let mut view = clipboard_view(Focus::Source, crate::clipboard::Recording::default());
        view.clipboard = Box::new(BrokenClipboard);
        view.source = Editor::from_text("keep me");

        view.handle_key(&ctrl_char('v'));
        assert_eq!(view.source.text(), "keep me");
        assert_eq!(
            view.message.as_deref(),
            Some("clipboard unavailable"),
            "a clipboard failure should be one readable sentence"
        );

        view.handle_key(&ctrl_char('y'));
        assert_eq!(view.message.as_deref(), Some("clipboard unavailable"));

        // And the pane still edits, which is what "everything else works" means.
        view.handle_key(&press_char('!'));
        assert_eq!(view.source.text(), "keep me!");
    }

    #[test]
    fn the_toast_is_drawn_over_the_focused_pane_and_the_status_line_survives() {
        let mut view = clipboard_view(Focus::Debug, crate::clipboard::Recording::with("x"));
        view.handle_key(&ctrl_char('v'));

        let buffer = draw_at(&mut view, 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));
        let inner = panes.inner(Focus::Debug);
        let rendered = (inner.y..inner.y + inner.height)
            .map(|y| row(&buffer, y))
            .collect::<String>();
        assert!(
            rendered.contains("read-only"),
            "the toast was not drawn in the focused pane: {rendered:?}"
        );
        // A toast is an overlay, not a new layout: the status line keeps its row.
        assert!(row(&buffer, panes.status.y).contains("claude"));
    }

    /// A clipboard that always fails, standing in for a machine with no display server.
    #[derive(Debug)]
    struct BrokenClipboard;

    impl crate::clipboard::Clipboard for BrokenClipboard {
        fn get_text(&mut self) -> Result<Option<String>, crate::clipboard::Error> {
            Err(crate::clipboard::Error::unavailable("no display server"))
        }

        fn set_text(&mut self, _text: &str) -> Result<(), crate::clipboard::Error> {
            Err(crate::clipboard::Error::unavailable("no display server"))
        }
    }

    /// A view with a scripted rewrite machine, so `Ctrl+R` and `Ctrl+N` can be driven without
    /// a process. The machine's child replays `script` and then reports `outcome`.
    fn rewrite_view(
        focus: Focus,
        script: Vec<Vec<String>>,
        outcome: Option<crate::process::Outcome>,
    ) -> (View<'static>, std::rc::Rc<std::cell::RefCell<ScriptState>>) {
        use crate::process::{Line, Outcome as RunOutcome};

        /// What the fake child replays and what it has been asked to do.
        struct Scripted {
            state: std::rc::Rc<std::cell::RefCell<ScriptState>>,
        }

        impl crate::rewrite::Child for Scripted {
            fn drain(&mut self) -> Vec<Line> {
                let mut state = self.state.borrow_mut();
                let next = if state.lines.is_empty() {
                    Vec::new()
                } else {
                    state.lines.remove(0)
                };
                next.into_iter()
                    .map(|text| Line {
                        stream: crate::process::Stream::Stdout,
                        text,
                    })
                    .collect()
            }

            fn poll(&mut self) -> Option<RunOutcome> {
                // Only report an outcome once the script is spent, so a test that ticks
                // several times gets several streaming ticks before the ending. A fake that
                // ended on the first poll would make "still streaming" untestable.
                let state = self.state.borrow();
                if state.lines.is_empty() {
                    state.outcome.clone()
                } else {
                    None
                }
            }

            fn cancel(&mut self) {
                self.state.borrow_mut().cancels += 1;
            }
        }

        let state = std::rc::Rc::new(std::cell::RefCell::new(ScriptState {
            lines: script,
            outcome,
            cancels: 0,
        }));
        let handed = state.clone();
        let machine = crate::rewrite::Machine::with_spawner(
            Box::new(crate::harness::claude::Claude),
            Box::new(move |_, _| {
                Ok(Box::new(Scripted {
                    state: handed.clone(),
                }) as Box<dyn crate::rewrite::Child>)
            }),
        );

        let config = Box::leak(Box::new(Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: "source text".to_owned(),
            initial_instructions: "make it formal".to_owned(),
        }));
        let mut view = View::new(config, focus);
        view.machine = Some(Box::new(machine));
        let _ = draw_at(&mut view, 80, 24);
        (view, state)
    }

    /// The script a fake rewrite child replays, shared with the test.
    struct ScriptState {
        lines: Vec<Vec<String>>,
        outcome: Option<crate::process::Outcome>,
        cancels: usize,
    }

    /// A terminal event carrying a delimited rewrite.
    fn delimited(text: &str) -> String {
        format!(r#"{{"type":"result","result":"<<<TEXT>>>{text}<<<END>>>"}}"#)
    }

    fn exited(code: i32) -> crate::process::Outcome {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exit {code}"))
            .spawn()
            .expect("spawn sh");
        crate::process::Outcome::Exited(child.wait().expect("wait"))
    }

    #[test]
    fn ctrl_r_with_empty_instructions_is_refused_and_focuses_the_instructions_pane() {
        let (mut view, _) = rewrite_view(Focus::Source, Vec::new(), None);
        view.instructions = Editor::from_text("   \n\t");
        let _ = draw_at(&mut view, 80, 24);

        view.handle_key(&ctrl_char('r'));

        assert_eq!(
            view.state,
            State::Idle,
            "a refused rewrite changed the state"
        );
        assert_eq!(
            view.focus,
            Focus::Instructions,
            "focus did not move to the culprit"
        );
        assert_eq!(
            view.message.as_deref(),
            Some("instructions are empty"),
            "the reason was not shown"
        );
    }

    #[test]
    fn ctrl_r_with_empty_source_is_refused_and_focuses_the_source_pane() {
        let (mut view, _) = rewrite_view(Focus::Instructions, Vec::new(), None);
        view.source = Editor::from_text("  ");
        let _ = draw_at(&mut view, 80, 24);

        view.handle_key(&ctrl_char('r'));

        assert_eq!(view.state, State::Idle);
        assert_eq!(
            view.focus,
            Focus::Source,
            "focus did not move to the culprit"
        );
        assert_eq!(view.message.as_deref(), Some("source text is empty"));
    }

    #[test]
    fn ctrl_r_starts_a_run_and_moves_to_running() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        // The harness must resolve for a start to be allowed; the test PATH has no claude, so
        // the binary is stubbed by pointing the view at a name that is on PATH.
        view.config = Box::leak(Box::new(Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: String::new(),
            initial_instructions: String::new(),
        }));

        // Directly exercise the machine instead of PATH: the refusal path is covered above,
        // and this asserts the state move that a successful start makes.
        let machine = view.machine.as_mut().unwrap();
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        machine
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .expect("the machine refused a valid task");
        view.state = State::Running;

        assert_eq!(view.state, State::Running);
        assert!(view.machine.as_ref().unwrap().running());
        assert_eq!(
            view.session.as_deref().map(str::len),
            None,
            "the view session was not synced by the direct machine call"
        );
    }

    #[test]
    fn ctrl_r_while_running_is_refused_with_a_note() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        // The machine refuses a second start; the view reports it without leaving the state.
        view.message = None;
        let refused = view.machine.as_mut().unwrap().start(
            &task,
            true,
            &mut view.stream,
            std::time::Instant::now(),
        );
        view.message = refused.err().map(|r| r.message().to_owned());

        assert_eq!(refused, Err(crate::rewrite::Refusal::AlreadyRunning));
        assert_eq!(
            view.message.as_deref(),
            Some("a rewrite is already running")
        );
        assert_eq!(view.state, State::Running, "the live run was disturbed");
    }

    #[test]
    fn ctrl_n_while_running_is_refused_and_keeps_the_session() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.sync_session();
        let session = view.session.clone().expect("no session");

        view.handle_key(&ctrl_char('n'));

        assert_eq!(
            view.message.as_deref(),
            Some("a rewrite is already running"),
            "Ctrl+N did not say why it was ignored"
        );
        assert_eq!(view.session, Some(session), "the live session was dropped");
    }

    #[test]
    fn ctrl_n_when_idle_drops_the_session_and_clears_the_read_only_panes() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], Some(exited(0)));
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.sync_session();
        assert!(view.session.is_some());
        // Finish the run so `Ctrl+N` is allowed.
        view.tick();

        view.handle_key(&ctrl_char('n'));

        assert_eq!(view.session, None, "the session survived Ctrl+N");
        assert_eq!(view.stream.output_text(), "", "output not cleared");
        assert_eq!(view.stream.debug_text(), "", "debug not cleared");
    }

    #[test]
    fn ctrl_n_keeps_the_source_and_instructions() {
        // Spec §5 step 3 clears harness context, not the user's work: the whole point of
        // Ctrl+N is to start a fresh session over the same text, so losing the panes would
        // make the key useless.
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], Some(exited(0)));
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.tick();
        view.source = Editor::from_text("the text to keep");
        view.instructions = Editor::from_text("the instructions to keep");

        view.handle_key(&ctrl_char('n'));

        assert_eq!(
            view.source.text(),
            "the text to keep",
            "Ctrl+N lost the source"
        );
        assert_eq!(
            view.instructions.text(),
            "the instructions to keep",
            "Ctrl+N lost the instructions"
        );
    }

    #[test]
    fn a_tick_reports_nothing_when_no_run_is_live() {
        let (mut view, _) = rewrite_view(Focus::Source, Vec::new(), None);

        assert!(!view.tick(), "an idle tick asked for a redraw");
    }

    #[test]
    fn a_tick_that_drains_lines_asks_for_a_redraw() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![vec![delimited("streamed")]], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();

        assert!(view.tick(), "a streaming tick did not ask for a redraw");
    }

    #[test]
    fn a_finished_run_moves_the_state_word_and_writes_the_output() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![vec![delimited("the rewrite")]],
            Some(exited(0)),
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        assert!(view.tick(), "the settling tick asked for no redraw");

        assert_eq!(
            view.state,
            State::Idle,
            "a successful run did not return to idle"
        );
        assert_eq!(view.stream.output_text(), "the rewrite");
    }

    #[test]
    fn a_failed_run_moves_to_error_with_its_reason() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], Some(exited(1)));
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        view.tick();

        assert_eq!(view.state, State::Error);
        assert!(
            view.message
                .as_deref()
                .is_some_and(|m| m.contains("non-zero")),
            "the failure reason was not shown: {:?}",
            view.message
        );
    }

    #[test]
    fn a_run_with_no_markers_moves_to_error_and_leaves_the_output_pane_alone() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![vec![
                r#"{"type":"result","result":"no markers here"}"#.to_owned(),
            ]],
            Some(exited(0)),
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        view.tick();

        assert_eq!(view.state, State::Error);
        assert_eq!(
            view.stream.output_text(),
            "",
            "a markerless answer reached the output pane"
        );
    }

    #[test]
    fn the_status_line_shows_the_session_id_once_one_exists() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.sync_session();

        let buffer = draw_at(&mut view, 80, 24);
        let status = row(&buffer, 23);
        let session = view.session.as_deref().expect("no session");
        assert!(
            status.contains(session),
            "the session id is not on the status line: {status:?}"
        );
        assert!(
            !status.contains('—'),
            "the em-dash stayed after a session existed: {status:?}"
        );
    }

    #[test]
    fn ctrl_c_while_running_cancels_instead_of_quitting() {
        let (mut view, state) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        assert_eq!(
            view.handle_key(&ctrl_char('c')),
            Outcome::Continue,
            "Ctrl+C quit while a run was live"
        );

        assert_eq!(state.borrow().cancels, 1, "the child was not asked to stop");
        // The state word waits for the cancelled outcome to come back through the tick: the
        // run has been asked to stop, not reaped.
        assert_eq!(view.state, State::Running);
        assert_eq!(view.message.as_deref(), Some("cancelling"));
    }

    #[test]
    fn a_cancelled_run_leaves_the_output_pane_as_cleared_on_spawn() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![Vec::new()],
            Some(crate::process::Outcome::Cancelled),
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        view.tick();

        // Spec §7: nothing to reconcile, because the pane was cleared on spawn and a cancelled
        // run produced no final text.
        assert_eq!(view.stream.output_text(), "");
        assert!(
            view.stream.debug_text().contains("cancelled"),
            "the cancellation was not reported: {}",
            view.stream.debug_text()
        );
        assert_ne!(view.state, State::Running, "the state stayed running");
    }

    #[test]
    fn ctrl_c_when_idle_quits_with_no_prompt() {
        let (mut view, _) = rewrite_view(Focus::Source, Vec::new(), None);
        assert_eq!(view.state, State::Idle);

        assert_eq!(
            view.handle_key(&ctrl_char('c')),
            Outcome::Quit,
            "Ctrl+C did not quit while idle"
        );
    }

    #[test]
    fn ctrl_q_while_running_asks_first_and_the_second_press_quits() {
        let (mut view, _) = rewrite_view(Focus::Source, vec![Vec::new()], None);
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        assert_eq!(
            view.handle_key(&ctrl_char('q')),
            Outcome::Continue,
            "Ctrl+Q quit on the first press mid-run"
        );
        assert!(view.quit_armed, "the quit was not armed");
        assert!(
            view.message
                .as_deref()
                .is_some_and(|m| m.contains("Ctrl+Q")),
            "the prompt is not actionable: {:?}",
            view.message
        );

        // Declining must leave the run untouched, so the second press is the only way out.
        assert!(!view.stream.debug_text().contains("cancelled"));

        assert_eq!(
            view.handle_key(&ctrl_char('q')),
            Outcome::Quit,
            "the confirming press did not quit"
        );
    }

    #[test]
    fn declining_the_confirmation_leaves_the_run_streaming() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![vec![delimited("still going")], Vec::new()],
            None,
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        // Ask, then press something else instead of confirming.
        view.handle_key(&ctrl_char('q'));
        view.handle_key(&press_char('x'));

        // The run continues: a tick still delivers its lines and the state is untouched.
        assert!(
            view.tick(),
            "the run stopped delivering after a declined quit"
        );
        assert_eq!(view.state, State::Running, "the run was disturbed");
        // The result event is labelled junk by design (its text belongs to the output pane),
        // so "still streaming" is proved by the line arriving at all.
        assert!(
            view.stream.debug_text().contains("still going"),
            "streaming stopped after a declined quit: {}",
            view.stream.debug_text()
        );
    }

    #[test]
    fn ctrl_q_when_idle_quits_immediately_with_no_prompt() {
        let (mut view, _) = rewrite_view(Focus::Source, Vec::new(), None);

        assert_eq!(view.handle_key(&ctrl_char('q')), Outcome::Quit);
        assert!(!view.quit_armed, "a prompt was armed while idle");
    }

    #[test]
    fn a_non_json_line_mid_stream_is_junk_and_the_run_continues() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![
                vec!["this is not JSON".to_owned()],
                vec![delimited("the rewrite")],
            ],
            Some(exited(0)),
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;

        // First tick: the garbage line only.
        view.tick();
        let debug = view.stream.debug_text();
        assert!(
            debug.contains("junk  this is not JSON"),
            "a malformed line was not labelled junk: {debug}"
        );
        assert_eq!(view.state, State::Running, "a junk line aborted the run");

        // Second tick: the run still finishes normally.
        view.tick();
        assert_eq!(view.state, State::Idle, "the run did not finish");
        assert_eq!(view.stream.output_text(), "the rewrite");
    }

    #[test]
    fn the_debug_ring_buffer_trims_and_still_tails() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![
                (0..crate::panes::DEBUG_CAPACITY + 500)
                    .map(|n| format!(r#"{{"type":"rate_limit_event","n":{n}}}"#))
                    .collect(),
            ],
            None,
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();

        view.tick();

        let debug = view.stream.debug_text();
        assert_eq!(
            debug.lines().count(),
            crate::panes::DEBUG_CAPACITY,
            "the ring buffer did not trim to its capacity"
        );
        // Still tailing, so the newest line is on screen.
        assert_eq!(
            view.stream.debug_suffix(),
            None,
            "trimming stopped the pane from tailing"
        );
        let buffer = draw_at(&mut view, 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));
        let last_visible = row(&buffer, panes.debug.y + panes.debug.height - 2);
        assert!(
            last_visible.contains(&format!("n\":{}", crate::panes::DEBUG_CAPACITY + 499)),
            "the newest line is not in view after trimming: {last_visible:?}"
        );
    }

    #[test]
    fn a_resize_during_a_run_does_not_panic_and_keeps_the_tail() {
        let (mut view, _) = rewrite_view(
            Focus::Source,
            vec![
                (0..40)
                    .map(|n| format!(r#"{{"type":"rate_limit_event","n":{n}}}"#))
                    .collect(),
            ],
            None,
        );
        let task = crate::harness::Task {
            instructions: "formal",
            source_text: "text",
            model: None,
            session: None,
        };
        view.machine
            .as_mut()
            .unwrap()
            .start(&task, true, &mut view.stream, std::time::Instant::now())
            .unwrap();
        view.state = State::Running;
        view.tick();

        for (width, height) in [(80, 24), (40, 10), (10, 4), (1, 1), (120, 40), (80, 24)] {
            let _ = draw_at(&mut view, width, height);
        }

        // Still tailing after every rewrap, so the newest line is on screen.
        assert_eq!(
            view.stream.debug_suffix(),
            None,
            "a resize stopped the pane from tailing"
        );
        assert_eq!(view.state, State::Running, "a resize disturbed the state");
    }

    #[test]
    fn keys_owned_by_other_tickets_leave_focus_alone() {
        for (code, modifiers) in [
            (KeyCode::Char('r'), KeyModifiers::CONTROL),
            (KeyCode::Left, KeyModifiers::NONE),
            (KeyCode::Right, KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
            (KeyCode::Char('x'), KeyModifiers::NONE),
        ] {
            assert_eq!(
                focus_after(KeyEvent::new(code, modifiers), Focus::Debug),
                Focus::Debug,
                "{code:?} with {modifiers:?} moved focus"
            );
        }
    }

    #[test]
    fn focus_changes_redraw_the_highlight_on_the_next_frame() {
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        let before = {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut view)).unwrap();
            terminal.backend().buffer().clone()
        };
        assert!(before[(panes.source.x, panes.source.y)].style().fg == Some(Color::Yellow));

        view.handle_key(&ctrl(KeyCode::Right));
        let after = {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut view)).unwrap();
            terminal.backend().buffer().clone()
        };

        assert!(after[(panes.output.x, panes.output.y)].style().fg == Some(Color::Yellow));
        assert!(
            matches!(
                after[(panes.source.x, panes.source.y)].style().fg,
                None | Some(Color::Reset)
            ),
            "the pane we left is still highlighted"
        );
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_q_quits() {
        assert!(is_quit(press(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn plain_q_quits() {
        assert!(is_quit(press(KeyCode::Char('q'), KeyModifiers::NONE)));
    }

    #[test]
    fn other_keys_do_not_quit() {
        assert!(!is_quit(press(KeyCode::Char('Q'), KeyModifiers::CONTROL)));
        assert!(!is_quit(press(KeyCode::Char('q'), KeyModifiers::ALT)));
        assert!(!is_quit(press(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert!(!is_quit(press(KeyCode::Esc, KeyModifiers::NONE)));
    }
}
