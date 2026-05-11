# agent-memo MCP

Go-implemented MCP server that lets agents persist findings, decisions, and
session handoffs to a Honcho v3 backend. Source: `internal/agpod-mcp/`.

The server is intentionally decoupled from the Rust `agpod-case` crate: it has
its own Honcho workspace (recommended: `agpod-memo`) and its own session
namespace (`memo_<repo_id>`).

## Build

```bash
cd internal/agpod-mcp
go build ./cmd/agpod-mcp
```

The binary is independent from the Cargo workspace; it has its own `go.mod`
and is not in CI today. Run `go test ./...` from the same directory before
shipping changes.

## Environment

| Variable | Required | Default | Notes |
|---|---|---|---|
| `HONCHO_API_KEY` | yes | — | Bearer token for Honcho. |
| `HONCHO_BASE_URL` | no | `https://api.honcho.dev` | Override for self-hosted Honcho. |
| `HONCHO_WORKSPACE_ID` | yes | — | Recommended value: `agpod-memo`. Keep this distinct from any `agpod-case` workspace. |
| `AGPOD_MEMO_PEER_ID` | no | `agpod-memo` | All entries are written under this peer. |
| `AGPOD_MEMO_REPO_ROOT` | no | current working directory | Used to derive `repo_id` from `git remote`. |
| `AGPOD_MEMO_LOG_LEVEL` | no | `info` | `debug`, `info`, `warn`, `error`. Logs go to stderr; stdout is reserved for MCP traffic. |
| `AGPOD_MEMO_READONLY` | no | `false` | Truthy values (`1`, `true`, `yes`, `on`) skip registering the four mutating tools — only `memo_pickup_handoff`, `memo_recall`, `memo_why` are exposed. Use for low-trust agents that should consume memory without writing to it. |

The `repo_id` is `hex(sha256("v1:" + normalized_remote_url))[:16]` — the same
algorithm as `crates/agpod-case/src/repo_id.rs`, so the IDs match across
tools. The Honcho session ID is `memo_<repo_id>`.

## Wiring into an MCP client

Stdio transport. Example launch line for Claude Code or any other MCP host:

```bash
HONCHO_API_KEY=... HONCHO_WORKSPACE_ID=agpod-memo \
  /path/to/agpod-mcp
```

Run it with the host's working directory pinned to the repo root — that is
how `repo_id` is derived. Alternatively set `AGPOD_MEMO_REPO_ROOT=/abs/path`.

## Tools

All write tools auto-bind to the current repo. Read tools default to the
current repo and accept `cross_repo: true` to escape that boundary.

### `memo_write_finding`
Record an explored fact (how / where / what). Inputs: `content`, `scope[]`, optional `evidence_refs[]`. Output: `{entry_id}`.

### `memo_write_decision`
Record a choice and its rejected alternatives. Pass `supersedes` to mark a
previous decision as replaced — the old entry's status is updated to
`superseded` automatically.

### `memo_write_handoff`
Snapshot session end state. Inputs: `summary` (short title), `content`
(markdown body). Pickup is by repo (latest live) or by `handoff_id`.

### `memo_pickup_handoff`
Returns the latest live handoff for this repo by default. Pass `handoff_id`
for a specific one, or `cross_repo: true` to fetch the latest across repos.

### `memo_recall`
Semantic search when `query` is non-empty; otherwise lists the most recent
entries. Defaults skip `handoff` entries — set `include_handoff: true` to
include them. Other filters: `entry_type`, `scope_prefix`, `cross_repo`,
`limit` (default 20, max 100).

### `memo_why`
Returns every live decision whose `scope` contains the supplied anchor, each
carrying its full supersedes chain (oldest predecessor last). Use this to
audit why a piece of code is the way it is.

### `memo_set_status`
Mark an entry as `superseded` or `no_longer_applicable`. Live → these states
only; cannot resurrect a retired entry by passing `live`.

## Operational notes

- The server treats Honcho failures as fatal for the failing call only — it
  does not retry. The peer/session is ensured at startup; if that fails the
  server still boots and each tool will surface the underlying error.
- Metadata is stored flat (no nested status records); empty arrays and empty
  strings are dropped before sending, matching the policy in
  `crates/agpod-case/src/honcho.rs`.
- Recall results sort by `created_at` descending. Honcho's metadata-filter
  support varies; the server post-filters locally for `status`, `entry_type`,
  and `scope_prefix` so behavior is consistent regardless.

## Future work

These items are deliberately out of scope for the first version and live
here as a punch list:

1. Wire into CI (Linux/macOS build + test).
2. Decide release-please policy if the binary is to be distributed.
3. `memo_handoff_compact` to merge or archive old handoffs.
4. Bridge to `agpod-case`: on case-close write a `decision` automatically.
5. Auto-collect `evidence_refs` from the current git commit.
