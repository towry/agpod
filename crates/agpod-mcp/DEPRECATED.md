# DEPRECATED

This Rust crate is superseded by the Go implementation at
`internal/agpod-mcp/`. Both produce a binary called `agpod-mcp`, but the Go
one is the canonical implementation going forward.

## Why the move

- The Go binary backs an `agent-memo` MCP server with three entry types
  (finding / decision / handoff) for cross-session agent memory.
- The Rust binary wraps `agpod-case` workflows. Those workflows are being
  replaced by the agent-memo model, so the Rust crate has no remaining
  consumers in active development.

## Removal plan

1. Stop publishing Rust binary artifacts in the next release.
2. After one release cycle with the Go binary in production, drop this crate
   from the Cargo workspace and `release-please-config.json`.
3. Do not add new features here — fix only critical bugs.

If you need the old behavior, build from the last tag that included this
crate. The Go implementation does not currently expose `agpod-case` tools.
