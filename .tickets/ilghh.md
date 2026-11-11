---
title: "Paste and startup split multi-line text: a line break ends a burst"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T21:47:42Z
updated: 2026-11-11T21:53:53Z
closed-at: 2026-11-11T21:53:53Z
---
# ilghh
Reported by a user: pasting `~/Downloads/oops.txt` (607 chars, 5 paragraphs, CRLF) into the
source pane leaves only the last paragraph. Reproduced by replaying the file through
`collect_text_run` plus the view: 607 chars in, 131 chars stored, 5 separate runs.

Two causes, one mechanism.

1. **A line break ends the burst.** On Windows the console delivers a paste as one
   `KeyCode::Char` event per character, and `Ctrl+V` never reaches the app (Windows Terminal
   binds it to its own paste). `typed_char` in `src/main.rs` accepts only `Char`, so
   `KeyCode::Enter` (a CRLF, a lone CR, or a lone LF) stops the run. Each paragraph becomes its
   own burst.
2. **Each burst replaces the pane.** `handle_paste` -> `replace_text` clears before inserting
   (deliberate, ticket g4nxst), so burst N wipes bursts 1..N-1 and only the last survives.

`KeyCode::Tab` is a third instance of the same edge: a tab ends the run *and* the leftover
event is routed as `Tab`, which moves focus to the next pane, so the remainder of a clipboard
containing a tab lands in the wrong buffer. Measured: `"alpha\tbeta"` gives source `"alpha"`
and instructions `"beta"`.

Startup is not affected (it loads the whole clipboard string at once through
`Editor::from_text`), which is why the pane looks right until the paste.

## Decision (user-approved)
Whitespace is whitespace for this app. A line break and a tab are both content, not commands,
when they appear inside a pasted run, and a tab normalises to a space so the pane shows one
cell per whitespace char.

## Done when
- `Editor` owns the normalisation, in the one primitive every insert goes through
  (`insert_one`): `'\r'` becomes a line break exactly as `'\n'` does, and `'\t'` lands as
  `' '`. No second copy in `cli.rs`, `ui.rs` or the collector: `from_text` (startup,
  `--text`, `--text-file`, the clipboard at startup) and `insert_str` (every paste route)
  already funnel through it.
- `collect_text_run` / `typed_char` in `src/main.rs` treat `KeyCode::Enter` as `'\n'` and
  `KeyCode::Tab` as `' '` **while a run is already open**, so a burst survives the line breaks
  in the middle of a paste. A single typed `Enter` or `Tab` still opens no run and keeps its
  current meaning.
- A paste of `oops.txt` lands 607 chars (4 line breaks + the rest) in one edit, and one
  `Ctrl+Z` restores the pane exactly. This is the regression test and it is the ticket's
  verification.
- Replaying `"alpha\nbeta\ngamma"` and `"alpha\tbeta"` through the collector gives one run,
  the whole text in `source`, and focus still `Source`.

## Notes and consequences to handle in the same commit
- **`tab` is a modifier prefix.** A terminal may report a tab as `Char('\t')` *plus*
  `SHIFT`, which `typed_char` already allows, so the existing modifier rule is sufficient; no
  extra case is needed.
- **Two column-0 tabs give a Tab event.** With bracketed paste off, xterm-style terminals read
  a tab as the completion character and emit `Tab`. Accepting `Tab` inside a run covers that.
  Accepted limitation, not defended: the boundary between "a key that arrived during a burst"
  and "paste content" cannot be recovered from a `KeyEvent`, and 10 ms (`BURST_GRACE`) is
  shorter than any human chord. A held `Tab` pressed inside that window would join the run as
  a space instead of moving focus.
- **`\r` is not stripped anywhere else.** `prompt_passes_text_through_verbatim`
  (`src/harness/mod.rs`) asserts the source reaches the prompt verbatim; with `\r` normalised
  in the editor, that test must keep passing with a source that came from an editor (no CR)
  and its own literal `"a\r\nb"` example stays a statement about the template, not the buffer.
- **The wrap rule stays.** `a_control_char_takes_no_cell_because_the_renderer_drops_it`
  (`src/wrap.rs`) is about a raw control char reaching the renderer. After this ticket the
  editor no longer produces one from a paste or from typing, but the rule is still correct for
  a buffer built directly in a test, so the test and the `char_cells` branch stay.
- **Undo stays one step.** The normalisation is inside the existing single `Undo::edit`
  around `clear` + `insert_str`, so a paste remains one step and no new recording boundary
  appears (spec §9).
- **No spec edit.** `docs/spec.md` is frozen; §4 says nothing about tabs or about how a burst
  ends, so nothing there becomes false. §4's `Ctrl+V` row ("replacing it") stays true.
- No clipboard, terminal-setup or argv change, so no constrained section is touched.

## Tests
- `src/editor.rs`: `from_text` maps `\r\n` and a lone `\r` to one line break and `\t` to a
  space; `insert_str` does the same at the cursor; the existing round-trip test gains the
  cases it can still round-trip (`\t` is documented as the one exception, since it becomes a
  space).
- `src/main.rs`: a run spanning `Enter` is one string; a run spanning `Tab` is one string and
  does not move focus; a single `Enter` and a single `Tab` keep their current behaviour.
- `src/ui.rs`: a paste whose text contains blank lines replaces the pane with all of it, and
  one `Ctrl+Z` restores the previous text and cursor.
- `make check` is green.