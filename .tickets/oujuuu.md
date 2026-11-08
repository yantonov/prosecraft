---
status: closed
priority: 3
deps: []
attachments: []
created: 2026-11-08T12:04:13Z
updated: 2026-11-08T12:16:47Z
closed-at: 2026-11-08T12:16:47Z
---
# F1 help panel: key bindings overlay

## Why
The app has twelve key bindings and exactly three of them reach the screen: the status line
hints read `[Ctrl+R] rewrite  [Tab] focus  [Ctrl+Q] quit`. Everything else a user can do —
`Ctrl+N`, `Ctrl+Y`, `Ctrl+V`, `Ctrl+E`, `Ctrl+←/→/↑/↓`, `PgUp`/`PgDn` — is discoverable only
by reading `docs/spec.md` §4. The hints row cannot carry the rest: `fit()` already clips it,
and making it longer eats the error message that shares the row (`draw_status`, `src/ui.rs:628`).

## Decided by the human, 2026-11-08
`docs/spec.md` §4 now carries the two rows (edited with explicit approval; constraint 1):

| Key | Action |
|---|---|
| `F1` | Toggle the key-bindings help panel. Any key closes it; the key that closes it does nothing else |
| `Esc` | Closes the help panel when it is open. Otherwise does nothing in v0.1 |

The status-line hint list was also pinned there: `[Ctrl+R] rewrite`, `[Tab] focus`,
`[F1] help`, `[Ctrl+Q] quit`, in that order. **`F1`, any-key dismissal, and showing `F1` in
the status line are settled — do not re-litigate them in the implementation.**

Why `F1` and not the alternatives, measured in crossterm 0.29's own parsing, not guessed:
- `F1` arrives on Unix as `ESC O P`, or `CSI 1 ; mods P` with modifiers (`event/sys/unix/parse.rs`),
  and on Windows as `VK_F1` → `KeyCode::F(1)` with the modifier state intact
  (`event/sys/windows/parse.rs`). It is a distinct key code, so no ambiguity with a printable
  character.
- `Ctrl+H` is **not** usable here: on Unix `\x08` is not in crossterm's control range and falls
  through to `parse_utf8_char` as a bare `Char('\x08')`; on Windows `VK_BACK` maps to
  `Backspace` *before* the `get_char_for_key` branch that would recover the `h`, so
  `Ctrl+H` and Backspace are literally the same event: pressing it in the source pane would
  both open the panel and delete a character. Rejected on evidence.
- `Ctrl+F1` costs two cells more in the status row for nothing: it measured 81 cells against
  `F1`'s 76, and the row already needs to survive being clipped.

The one thing still worth a look while running: VS Code's integrated terminal binds `F1` to
the command palette before the app sees it. If that bites, the fix is a §4 edit naming another
function key, not code.

## Shape of the change (S)
The overlay machinery already exists, so this is not a new mechanism:

- `src/ui.rs:86` — one field beside `toast`: `pub help: bool`. A bool, not an enum: there is
  exactly one panel, and a second overlay type would be speculative.
- `src/ui.rs:149` `handle_key` — `KeyCode::F(1)` toggles it and returns; while it is open the
  next key closes it and is swallowed, mirroring the existing "the dismissal eats the key"
  rule for the toast at `src/ui.rs:167`. Placement matters: after the `is_quit`/`Ctrl+C` checks,
  before the toast arm.
- `src/ui.rs:522` `render` — `draw_help(frame, area)` after the panes, before the status line.
  It must `Clear` a centred rect first (ratatui's own `Clear`, **no new dependency**) or the
  pane borders show through.
- Panel content: a static `[(key, sentence)]` slice holding the course of §4, kept in one const
  next to `draw_help` so it cannot drift from the real bindings invisibly.

## Known traps
- **Every-key dismissal collides with a running rewrite**: with the panel open, the first
  `Ctrl+C` only closes it and the cancel needs a second press. This is spec'd behaviour now
  ("any key closes it; the key that closes it does nothing else"), so implement it and print
  "any key closes" inside the panel so the second press is not a surprise.
- **Sub-80-column terminals and 1×1 frames** must not panic, as with `one_by_one_does_not_panic`.

## Done when
- §4 carries the two rows (done) and the status line shows `[F1] help`.
- TestBackend tests beside `the_toast_is_drawn_over_the_focused_pane_and_the_status_line_survives`
  (`src/ui.rs:2052`): `F1` opens and every listed binding name appears in the frame; a second
  `F1` and `Esc` both close it; a closing key does not also perform its own action (open the
  panel, press `c` with an editable focus, and assert the character was not typed);
  `Ctrl+Q` still quits while it is open; a 1×1 and a 20×6 frame do not panic; the status line
  keeps its row and contains `[F1] help` at 80 columns.
- `make check` green, plus `make run` showing the panel at 80×24 and at 40×12.
- The panel's list and §4's table are compared once by eye, in that same run.

## Out of scope
Contextual help per pane, a scrollable help document, `?` as an alias (an unmodified printable
key cannot be a global binding while the editable panes take every character), and any
on-disk help file — constraint 2 rules that out.
