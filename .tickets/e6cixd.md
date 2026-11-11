---
title: "Bracketed paste: a terminal paste is one undo step"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T11:57:19Z
updated: 2026-11-11T12:34:48Z
closed-at: 2026-11-11T12:34:48Z
---
# Bracketed paste: a terminal paste is one undo step

Reported by a user: after pasting with the terminal (not Ctrl+V), Ctrl+Z removes one character instead of the whole fragment.

Cause: bracketed paste was never enabled. src/main.rs enters raw mode and the alternate screen and nothing else, so the terminal sends the pasted text as ordinary key events: src/ui.rs:284 turns each one into its own Undo::edit call, so a paste of N chars is N steps. Ctrl+V is unaffected because src/ui.rs:529 already wraps the whole insert_str in one Undo::edit.

Done when:
- EnableBracketedPaste is emitted with the alternate screen, and DisableBracketedPaste in write_restore, so the panic hook and the Tui Drop still restore the terminal completely.
- The event loop routes Event::Paste(text) to a new View::handle_paste, which dismisses the help panel and a toast first (same order as handle_key), refuses a read-only pane through require_editable with the same toast Ctrl+V uses, and otherwise does exactly one undo.edit(editor, |e| e.insert_str(text)).
- One terminal paste is one Ctrl+Z, byte-identical to the state before the paste, cursor included.
- A terminal paste into output/debug is refused like Ctrl+V, with no change to pane content and no focus move.
- Unbracketed input keeps undoing one char at a time: a terminal that does not implement the protocol still types char by char, and that must not regress.
- make check is green.

Notes:
- Terminal-dependent by nature: only terminals that emit ESC[200~/ESC[201~ get the single step (Windows Terminal and VT-enabled consoles do; the legacy WinAPI console does not). That is accepted, not a defect.
- docs/spec.md section 4 lists Ctrl+V as the paste surface and does not mention terminal paste. The spec is frozen (constraint 1), so this ticket adds the code path and no spec edit; whether the key table needs a note is a separate, human-owned decision.
- The comment at src/ui.rs:526 claims a paste is one undo step without saying it means the Ctrl+V path only; correct it while here.
- crossterm's bracketed-paste feature is already enabled through its default features, so no Cargo.toml change.



## Correction (measured): the bracketed-paste claim was wrong for Windows

The note above said Windows Terminal gets the single step. That is false, and the first fix
(687d2f2) therefore changed nothing on this machine. Measured in the crossterm 0.29 source:

- Event::Paste is only ever produced by sys/unix/parse.rs. The Windows event source
  (event/source/windows.rs) emits Key, Mouse, Resize and Focus events and never a Paste.
- crossterm never sets ENABLE_VIRTUAL_TERMINAL_INPUT on Windows either, so the console keeps
  delivering KEY_EVENT records: a paste arrives as one KeyCode::Char event per character.

The second cause is larger and platform-independent in effect: on Windows Terminal, Ctrl+V is
bound to the terminal's own paste action (User.paste). The terminal consumes the key, reads the
clipboard itself, and types the text in. The app's Ctrl+V handler never runs, so the one
insert_str call that makes a clipboard paste one step is bypassed entirely.

Fix (this commit): the event loop collects a run of char key events and hands the whole string
to View::handle_text_run, which inserts it in one call. A single typed char still goes through
handle_key, so typing keeps one step per key and nothing is delayed. Event::Paste remains for
terminals that do implement the protocol. The wait is zero until two chars have arrived
together; once a burst is proven a 10 ms grace window drains a paste larger than the console
input buffer instead of splitting it into several steps.



## Closed too early

This ticket was closed with 5aa09ee while the bug was still live: the burst collector it added
was defeated twice more, and each fix needed its own ticket.

- #ewphym - releases ended the run, so every run was one char.
- #jspxg2 - the first char was split off from the rest of the fragment.

The record now matches the commits: 5aa09ee, 1884c9c and dbfb029 each carry a ticket, and
those two were filed after their commits landed rather than before, which is the WIP=1
violation this note records.
