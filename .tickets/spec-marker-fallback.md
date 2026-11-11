---
title: "Resolve where raw text goes when a marker is missing"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T10:40:00Z
updated: 2026-11-08T10:12:54Z
closed-at: 2026-11-08T10:12:54Z
---
# Resolve where raw text goes when a marker is missing

## Description
`docs/spec.md` contradicts itself, and `AGENTS.md` contradicts part of it, on one question:
when the model does not use the `<<<TEXT>>>`/`<<<END>>>` markers, where does the raw text go?

| Source | Says |
|---|---|
| `docs/spec.md` §6.1a step 4 | "Show the raw terminal text **in the output pane**" |
| `docs/spec.md` §5 step 5 | "output pane **stays empty** for this run" |
| `docs/spec.md` §8 table | "raw terminal text shown, **debug pane** holds the full output (§6.1a)" |
| `AGENTS.md` constraint 5 | "**Output pane gets final text only**" — a MUST |

Three of four agree; §6.1a step 4 is the outlier.

## What is implemented today
The majority position, chosen during `marker-extract` and recorded in that commit:

- The output pane is left exactly as `spawn` cleared it. A failed run shows no rewrite.
- The raw model text is reproduced in the **debug pane** under a `raw` label, one line per
  line, so it can be read without decoding the JSON that carried it.
- The status line says "model did not use the output markers".

The decision lives in one branch of `StreamState::settle` (`src/panes.rs`), deliberately
isolated so resolving this ticket is a one-line change plus documentation.

## Blocked on: a human decision, because it needs a spec edit
Constraint 1 in `AGENTS.md` says `docs/spec.md` MUST NOT be edited without explicit human
approval. Every resolution below edits it. **Do not start this ticket without that approval.**

Two options, and only these two:

**(a) §6.1a step 4 is wrong.** Change it to say the raw text goes to the debug pane. No code
change: the documentation stops disagreeing with §5, §8 and constraint 5. Cheapest, and it
leaves the hard constraint intact.

**(b) §6.1a step 4 is right.** The user sees the raw text in the output pane on failure. This
loosens constraint 5 from "final text only" to "final text, or the raw output on error", and
§5 step 5 and the §8 row must be updated to match. Also change the `settle` branch and its
tests.

## Done when
- The human has chosen (a) or (b), in writing, on this ticket.
- `docs/spec.md` and `AGENTS.md` say the same thing in all four places. (For (b) that means
  amending constraint 5, which is a deliberate loosening of a hard rule and needs to be
  recorded as such in the Decisions section.)
- The code matches, and the tests assert the chosen behaviour. For (a) that is today's tests
  unchanged; for (b) the two tests in `panes.rs` that assert the output pane is untouched on
  a marker failure.
- `make check` green.

## Not in scope
Changing the marker protocol, falling back to trimming or preamble-stripping, or treating a
missing marker as success. All three are settled by §6.1a and are not in question here — this
ticket is only about *where the raw text is displayed*.

## Resolved 2026-11-08: option (a), approved by the human

§6.1a step 4 now says the raw text goes to the **debug pane** and explicitly that the output
pane stays as cleared on spawn, citing constraint 5 and §5 step 5. Exactly one paragraph
changed; the diff was checked against a saved baseline.

All four statements now agree:

- §6.1a step 4 — debug pane, output pane untouched
- §5 step 5 — output pane stays empty for the run
- §8 table — debug pane holds the full output (no change needed)
- AGENTS.md constraint 5 — output pane gets final text only (no change needed)

No code change was required: `marker-extract` had already implemented the majority position,
and `panes::tests::a_marker_failure_is_an_error_and_never_writes_the_output_pane` already
asserts it, along with `StreamState::raw` putting the text in the debug pane under its own
label.

Constraint 5 was NOT loosened, so it is unchanged and the spec's Decisions section needs no
amendment.
