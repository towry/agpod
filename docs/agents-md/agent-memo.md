# agent-memo MCP usage

## When to Use

When the agent-memo MCP server is registered with the host. The seven
`memo_*` tools let an agent preserve work for future sessions and inherit
work from past ones.

Three entry types — each answering a different question:

| Type | Answers | Lifetime |
|---|---|---|
| `finding` | how / where / what | live until explicitly retired |
| `decision` | why this path was chosen | live until superseded by a new decision |
| `handoff` | what state the last session ended in | always live; recalled only on demand |

## When to Write

| Situation | Tool | Why |
|---|---|---|
| Just spent >10 minutes mapping a module / convention / pitfall and the answer is not in code or commit messages | `memo_write_finding` | Next agent will not re-explore. |
| Picked a path with a real tradeoff (rejected alternatives matter) | `memo_write_decision` | `memo_why` later replays the reasoning. |
| Replacing an earlier decision in the same scope | `memo_write_decision` with `supersedes` and `supersede_reason` | Marks old one superseded, preserves the chain. |
| Ending a session with open work, queued questions, or a non-obvious next step | `memo_write_handoff` (once at session end, not per task) | Next session picks up via `memo_pickup_handoff`. |

Do **not** write findings for things derivable from the code (file paths,
function signatures, simple call graphs). Reach for `grep` first; memo is
for what `grep` cannot recover.

## Scope Anchors

`scope[]` on findings and decisions is the lookup key — pick anchors that
the next agent is likely to query by:

- `file:line` for code-local facts: `crates/agpod-case/src/hooks.rs:128`
- module/crate path for cross-cutting facts: `crates/agpod-case`
- concept keys for things without a clear file: `case-hooks`, `repo-id`,
  `honcho-metadata`

Use 1–3 anchors. The first should be the most specific, the last the most
general — `memo_recall scope_prefix=...` walks them as prefixes.

## On Pickup

At the start of a new session for the same repo, call `memo_pickup_handoff`
(no args) before doing anything else. Treat the returned `summary` as the
title of the carry-over and the `content` as the brief.

When investigating an unfamiliar area, call `memo_why scope=<anchor>` first
— it can save you from re-arguing a decision that was already weighed.

## Cross-Repo

All entries are bound to one repo. `cross_repo: true` widens the search to
the entire Honcho workspace. Use this sparingly: it is right for "what did
we learn last time we did X" across related repos, and wrong for routine
recall (which should stay scoped).

## Don'ts

- Don't write `handoff` after every small task. One at session end is the
  right cadence; the queue grows fast otherwise.
- Don't use `memo_set_status live` — the status transition is one-way
  (live → superseded / no_longer_applicable). The tool will reject it.
- Don't put secrets in `content` or `evidence_refs`. The store has no
  redaction layer.
