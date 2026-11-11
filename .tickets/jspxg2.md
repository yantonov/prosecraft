---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T12:34:43Z
updated: 2026-11-11T12:34:45Z
closed-at: 2026-11-11T12:34:45Z
---
# Burst collector: apply the grace window to the first char too

Retroactive ticket for commit dbfb029. Filed after the fact: the work was committed without a ticket, which breaks the WIP=1 rule in AGENTS.md.

Bug: after 1884c9c the pasted fragment came back in one Ctrl+Z but its first symbol needed another. Cause: the collector waited Duration::ZERO for the first char and BURST_GRACE for the rest, assuming a paste is queued whole. It is not: the terminal injects the chars, so at the instant the first arrives the rest are not there yet. The run therefore ended at count == 1, that char became its own edit, and the remaining fragment a second.

Done when:
- The wait is the same for the first char as for the rest, so a paste is never split into a head and a tail.
- A test pins the equality, so the asymmetry cannot creep back unnoticed.
- Typing keeps one step per key: the window is far below the interval between two human keypresses.
- make check is green.

Note: this was the second split of the same paste, with 1884c9c fixing the tail and this fixing the head. Both were the same class of mistake: an assumption about how the terminal delivers a paste, encoded without measuring it.
