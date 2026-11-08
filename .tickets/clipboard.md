---
status: closed
priority: 2
deps: [editor-wrap, tui-dummy]
attachments: []
created: 2026-11-07T15:33:44Z
updated: 2026-11-08T08:12:14Z
closed-at: 2026-11-08T08:12:14Z
---
# Clipboard: load, copy, paste

## Description
arboard-backed clipboard integration per docs/spec.md §4. Depends on editor-wrap, so the editor and its wrapping already exist.

Done when:
- Ctrl+V pastes the clipboard into the focused pane. In an editable pane it inserts at the cursor. In a read-only pane (output, debug) the paste is refused and a popup tells the user the pane is read-only; the popup is dismissible with any key and does not change pane content or focus.
- Ctrl+E loads the clipboard into the source pane, replacing its whole content. It acts on the source pane regardless of which pane has focus (spec §4 is explicit that the target is the source pane), and it moves focus to the source pane afterwards so the replace is visible.
- Ctrl+Y copies the focused pane's text to the clipboard; works for read-only panes (output, debug) as well as editable ones.
- With no clipboard available at startup, a single status-line warning appears and every other feature still works (no panic, no retry loop).
- An empty clipboard is a no-op for both Ctrl+V and Ctrl+E, reported in the status line; it must never wipe a pane.
- Non-UTF-8 and multi-line clipboard content is handled without panic; invalid UTF-8 is inserted lossily.

Note: Ctrl+V and Ctrl+E are distinct and do not conflict. Ctrl+V is cursor-relative and respects focus; Ctrl+E is a whole-pane replace that always targets source.

The popup is the first modal surface in the app. Keep it a single non-blocking status toast rendered over the focused pane; do not build a general modal framework for it.
