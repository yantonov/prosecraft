---
status: closed
priority: 1
deps: []
attachments: []
created: 2026-11-08T18:05:32Z
updated: 2026-11-08T18:16:08Z
closed-at: 2026-11-08T18:16:08Z
---
# Amend spec for codex: measured argv, session-from-stream, no delta stream

## Why this ticket exists

Adding a `codex` harness needs measurements before code, exactly as `pi` did. All of them
are done (codex-cli 0.159.0 on this machine) and recorded here, because several of them
contradict the assumptions §6.1a, §6.1b and §6.4 make about *any* harness.

**This ticket is the approval event for editing `docs/spec.md`.** AGENTS.md constraint 1
freezes the spec without explicit human approval; the human said the spec can be updated
for codex support, so this ticket carries the amendment and nothing else. No code changes.

## Measured: codex has no delta stream

| observation | value |
|---|---|
| `codex exec --json` on a 300-word answer | one `item.completed` at t≈21s, nothing in between |
| events in a whole run | `thread.started`, `item.started/completed`, `turn.started`, `turn.completed` |
| `agent_message` | arrives **complete** in `item.completed`, never as a delta |
| reasoning | `reasoning_output_tokens` > 0 but **no reasoning item is emitted at all** |
| delta events in the binary | `AgentMessageDeltaNotification` exists, but only in the app-server protocol, not in `exec --json` |

So §6.1b's premise ("JSON streaming is the only way to avoid an inert window") is
unavailable: codex's JSON mode is incremental *per item*, not per token. The debug pane
shows the run's first lines at t≈0.7s and then nothing until the whole answer lands. The
status line's `running` and the `app` lines are the only progress. New §6.5 must say this
plainly rather than implying a stream that does not exist.

## Measured: tools cannot be removed, only sandboxed

`claude` has `--tools ""`, `pi` has `--no-tools`. codex has **no equivalent**: tools are
built into the harness, and config keys that look like they would help are rejected
(`--strict-config` says `unknown configuration field`) or do nothing. Measured tool count
and effect:

| argv | tools offered | writes |
|---|---|---|
| nothing | 147 | file written |
| `-s read-only` | 147 | **file written 3/3** |
| `-s read-only -c approval_policy="never"` | 147 | blocked 3/3 |
| `-c sandbox_mode="read-only" -c approval_policy="never"` | 147 | blocked 9/9 |
| `… -c features.apps=false` | 29 | blocked |
| `… -c web_search="disabled"` | web tool gone | blocked |

Two findings that decide the argv:

1. **`-s read-only` alone does not deny anything.** It sets the sandbox policy but leaves
   approvals on-request, and on this machine a write then succeeded in every run. The pair
   `-c sandbox_mode="read-only"` + `-c approval_policy="never"` blocked every attempt
   (9/9 across a git repo and a plain directory). Both are passed, and the sandbox — not a
   missing tool — is what denies.
2. **A JS REPL survives every filter** (`mcp__node_repl__js`, `mcp__cua_repl.js`) and
   reaches the filesystem; measured, it fails with `EPERM` under the same sandbox, and a
   shell fetch fails too. So denial holds, but it is a *sandbox* guarantee, not a
   *no-tools* guarantee, which is weaker than §1's "tool use is disabled, not merely
   discouraged". This is the one place codex support is honestly worse than the other two
   adapters, and the spec must record it rather than imply parity.

## Measured: context injection, and what stops it

| argv | user's `AGENTS.md` in the prompt |
|---|---|
| hardened block only | **leaked**, reproduced verbatim when asked |
| `+ -c project_doc_max_bytes=0` | not present |
| `+ --ignore-user-config` | not present, **but changes sandbox/auth behaviour** |

`project_doc_max_bytes=0` is load-bearing and is codex's `--no-context-files`: without it a
rewrite silently inherits the user's coding guidelines. `--ignore-user-config` is
deliberately **not** used, because it disables the user's own config (including their model
choice) and measurably changed whether writes were blocked.

Residual, recorded and accepted: codex injects its own developer prompt *and* a skills
catalogue (~30 KB of local `SKILL.md` descriptions, ~18 k input tokens) into every call, and
no config key found suppresses it (`skills_instructions`,
`include_skills_usage_instructions`, `features.skill_search=false` are all rejected). It
costs tokens and register, not correctness: the markers still come back.

## Measured: session lifecycle is inverted

claude and pi take a client-chosen session id. **codex generates the id itself** and reports
it as `{"type":"thread.started","thread_id":"01a1bc…"}` on stdout; forcing one via
`-c thread_id=…` is ignored. Continuations use a subcommand:
`codex exec resume <id>`. So:

- the id is learned from the **event stream**, not from the argv, which the current trait
  cannot express (it reports the id only through argv, and `Machine` reads it there);
- `new_session_id()` has no meaning for this adapter, because the client cannot name a
  session.

Also measured: `--skip-git-repo-check` is required or codex refuses to run outside a git
repo, and `exec resume` accepts neither `-s` nor `--ignore-user-config` — those are
parent-command flags, so the sandbox is set through `-c` for both call shapes to keep one
argv shape instead of two.

## Measured: the protocol holds

| check | result |
|---|---|
| markers honoured, first call | `<<<TEXT>>>Arrr, send me the report, matey.<<<END>>>` |
| markers honoured, `exec resume` with changed instructions | `<<<TEXT>>>You are hereby requested…<<<END>>>` |
| changed instructions take effect on the same thread id | yes (pirate → legal register) |
| prompt transport, argv (`--` then one element) | works, newlines intact |
| prompt transport, stdin | works (`Reading prompt from stdin…` on stderr) |
| non-zero exit | `turn.failed` → exit 1 |
| answers when `stdin` is a closed pipe and the prompt is in argv | correct; noise line `Reading additional input from stdin…` on stderr |

## Done when

`docs/spec.md` is amended, and nothing else is touched:

1. §3 CLI: `--harness <claude|pi|codex>`.
2. §6: `new_session_id` returns `Option<String>` and a new `session_from_event` exists,
   with the reason (codex names its own sessions).
3. New §6.5 "codex", carrying the argv for both call shapes, each flag with the
   measurement behind it, the sandbox-not-no-tools caveat, the context-injection result,
   and the inverted session lifecycle.
4. §6.1a: a codex column in the event table — terminal text is the last
   `item.completed` with `item.type == "agent_message"`; there are no deltas.
5. §6.1b: a paragraph stating that codex cannot stream, so the debug pane is inert between
   items by necessity, not by oversight, and the status line is the progress signal.
6. §6.4: the four differences become five, with "who names the session" added.
7. §10 M6 — codex adapter milestone, with its verify script.
8. Decisions: one entry recording that codex denial is sandbox-level, the accepted
   consequence of §1's wording, and that this was a human decision.


## Result, recorded when the amendment landed

Spec is now v0.9. What changed, all of it measured on codex-cli 0.159.0 on this machine:

- §1 and the Decisions: codex denial is sandbox-level, stated in the non-goals rather than
  buried, with the intent ("no run may reach the filesystem") unchanged.
- §3: `--harness <claude|pi|codex>`.
- §6: `new_session_id -> Option<String>` and a new `session_from_event`, with a paragraph
  saying why one question ("who names the session") needs two methods; the prompt-transport
  paragraph now says claude and pi use argv and codex uses stdin, always, per adapter.
- §6.1a: codex column in the event table, plus the rule that only
  `item.type == "agent_message"` is prose — taking "the last item with text" would put a
  shell command's stdout in the output pane.
- §6.1b: a paragraph stating codex cannot stream, citing the 300-word measurement (one
  `item.completed` at t≈21 s), and that the status line is therefore the progress signal.
- §6.4: four differences become five, "who names the session" added.
- §6.5 (new): both argv shapes, every flag with its measurement, the deliberately absent
  `--ignore-user-config`/`-s`, the inverted session lifecycle, the sandbox caveat, the
  residual skills-catalogue cost, and the live end-to-end result.
- §7, §8, §9, §10 (M6), Decisions.

## Three corrections the amendment forced that the ticket did not anticipate

1. **`-c web_search="disabled"` is not sufficient on its own.** Measured: with
   `features.apps=false` alone the model still returned a live headline via `web__run`;
   with `web_search="disabled"` alone it could still search; only the pair produced
   `NO-WEB-ACCESS`. Both flags are therefore load-bearing and neither is redundant.
2. **`-c tools.web_search=false` does not remove web search.** Measured: `web__run` stayed.
   Named in §6.5 as not used, so nobody "simplifies" toward it.
3. **`-s read-only` alone is a no-op for denial.** It leaves approvals on-request and a write
   succeeded 3 of 3. Only `-c sandbox_mode="read-only"` + `-c approval_policy="never"`
   blocked, 9 of 9. §6.5 says the pair is the substitution for `--tools ""`.

## Final verified argv

    codex exec --json       -c sandbox_mode="read-only" -c approval_policy="never"       -c project_doc_max_bytes=0 -c features.apps=false -c web_search="disabled"       --skip-git-repo-check [--model <name>] -- -

and `codex exec resume --json <same -c block> <thread-id> -- -`.

Re-verified end to end after the spec was written: pirate first call, legal-register
continuation on the same thread id, `NO-WEB-ACCESS`, and `no file` for a write attempt.

`make check` green; no code touched, which is what this ticket asked for.
