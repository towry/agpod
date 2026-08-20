# agent-memo MCP usage

When the agent-memo MCP server is registered, three tools are available:
`note`, `find`, `forget`.

## When to note

Call `note` as soon as you learn something `rg` cannot recover: a
convention, a pitfall, a choice with a real tradeoff. Do not note file
paths, signatures, or call graphs.

`content` is one present-tense sentence that stands alone.

`cues` are optional short phrases you would type into `find` later
(2–8 words). Skip them when `content` already contains the path, command,
or env name. Do not invent categories.

## When to find

Before exploring an unfamiliar area, `find` with a short concrete query
that looks like a cue (`"login shell 没有 nix"`), not `"any related memory"`.

Use `mode=ask` only when you need a synthesized answer. If it returns
`unknown`, there is no stored note — do not treat that as a fact.

## When to forget

The stored note is wrong or no longer true. Pass the `id` from `note`.
To correct, `note` a new sentence; do not try to resurrect.
