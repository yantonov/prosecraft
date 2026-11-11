---
title: "Burst collector: skip key releases instead of ending the run at them"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T12:34:40Z
updated: 2026-11-11T12:34:45Z
closed-at: 2026-11-11T12:34:45Z
---
# Burst collector: skip key releases instead of ending the run at them

Retroactive ticket for commit 1884c9c. Filed after the fact: the work was committed without a ticket, which breaks the WIP=1 rule in AGENTS.md.

Bug: after 5aa09ee a terminal paste still undid one char per Ctrl+Z. Cause: crossterm's Windows parser emits Press and Release for every keystroke (src/main.rs:224 and :287 of sys/windows/parse.rs), so a pasted run arrives Press, Release, Press, Release, ... The collector ended the run at the first event that was not a typed char, and a release is not one, so every run was cut to count == 1 and every char went through handle_key as its own undo step.

Done when:
- Release events are skipped by collect_text_run rather than ending the run; a release is neither text nor a terminator.
- A genuine non-text key (a chord, a function key) still ends the run and is handed back as leftover so it is handled on the next pass.
- A test feeds the press/release interleaving Windows actually delivers. The previous test fed presses only, a shape this platform never produces, which is why it passed against a broken collector.
- make check is green.

Note: src/main.rs already carried a comment saying Windows reports a press and a release for every keystroke, written for handle_key. The collector was added beside it without applying it.
