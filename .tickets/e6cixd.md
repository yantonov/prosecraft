---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T11:57:19Z
updated: 2026-11-11T11:59:04Z
closed-at: 2026-11-11T11:59:04Z
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
