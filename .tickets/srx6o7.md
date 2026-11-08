---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:40:24Z
updated: 2026-11-08T12:41:22Z
closed-at: 2026-11-08T12:41:22Z
---
# Ctrl+R puts focus on the output pane

Pressing `Ctrl+R` is the end of typing: the user's next act is to read the result, and today
they have to press `Ctrl+Right` or two `Tab`s to see the pane that will hold it. Nothing else
is worth doing mid-run — the only key with a job in a live run, `Ctrl+C`, is a copy outside
the debug pane — so the focus that fits the moment is the result pane.

Spec §4 (edited with the human's go-ahead, constraint 1) now says a `Ctrl+R` that starts a
rewrite sets focus to the `output` pane, and §5's flow diagram shows `ok: focus → output`.
A refused `Ctrl+R` keeps §5 step 1's existing rule and moves focus to the culprit, so the
reason and its fix stay in one place.

Known consequence, deliberate and worth saying out loud: focus on `output` means halting a
live run is `Ctrl+Up` into `debug` and then `Ctrl+C`, because §4 lets only the debug pane
cancel. Promotion to the end of a run was considered and not taken here — during the run the
debug pane is the only pane that changes, so moving focus away from it would take the user
away from the only thing that is actually moving.

Done when:
- `rewrite` (`src/ui.rs`) sets `self.focus = Focus::Output` on the `Ok(())` arm of
  `machine.start`, and does not touch focus on the `Err` arm beyond the existing `culprit`.
- New test: from `Focus::Source` with a valid task, `Ctrl+R` leaves focus on `Output` and the
  state `Running`; the refusal tests (`ctrl_r_with_empty_instructions_...`,
  `ctrl_r_with_empty_source_...`) still assert their culprit focus unchanged.
- Test that an already-running `Ctrl+R` refusal leaves focus where it was — `culprit` returns
  `current` for `AlreadyRunning`, so this is the assertion that the new assignment cannot run
  on a failed start.
- `make check` green, plus `make run`: `Ctrl+R` shows the output pane highlighted while the
  run streams into debug, `Ctrl+Up` then `Ctrl+C` cancels, and a set of `PgUp`/`Ctrl+C` on the
  landed result copies it.

Out of scope: focus changes when a run *ends* (it stays where the user put it), and any
auto-scroll rule beyond the existing debug tail.
