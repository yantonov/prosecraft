---
title: "Ctrl+K clears the focused editable pane"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:49:20Z
updated: 2026-11-08T12:50:30Z
closed-at: 2026-11-08T12:50:30Z
---
# Ctrl+K clears the focused editable pane

There is no undo (spec §2), so today the only way to start a pane over is Backspace. Add Ctrl+K as the clear key for the focused editable pane; refuse it in the read-only panes with the same toast Ctrl+V uses. Spec §4 row already written.
