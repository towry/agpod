# agent-memo MCP usage

When the agent-memo MCP server is registered, three tools are available:
`note`, `ask_note`, `forget`.

## When to note

Call `note` as soon as you learn something `rg` cannot recover: a
convention, a pitfall, a choice with a real tradeoff. Do not note file
paths, signatures, or call graphs.

`content` is one present-tense sentence that stands alone.

`cues` are optional short phrases you would type into `ask_note` later
(2–8 words). Skip them when `content` already contains the path, command,
or env name. Do not invent categories.

## When to ask_note

Before exploring an unfamiliar area, `ask_note` with a short concrete query
that looks like a cue (`"login shell 没有 nix"`), not `"any related memory"`.

If it returns `unknown`, there is no stored note — do not treat that as a
fact. `answer` is a synthesis or the top matching note; `quotes`/`ids`
are the supporting hits.

## When to forget

The stored note is wrong or no longer true. Pass the `id` from `note`.
To correct, `note` a new sentence; do not try to resurrect.
