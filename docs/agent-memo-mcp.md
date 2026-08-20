# agent-memo MCP

Go MCP server that lets agents persist short facts to a Honcho v3 backend
and retrieve them later. Source: `internal/agpod-mcp/`.

Decoupled from the Rust `agpod-case` crate. Recommended workspace:
`agpod-memo` (keep it distinct from any `agpod-case` workspace). Session
namespace: `memo_<repo_id>`.

## Build

```bash
cd internal/agpod-mcp
go build ./cmd/agpod-mcp
go test ./...
```

## Environment

| Variable | Required | Default | Notes |
|---|---|---|---|
| `HONCHO_API_KEY` | yes | — | Bearer token for Honcho. |
| `HONCHO_BASE_URL` | no | `https://api.honcho.dev` | Override for self-hosted Honcho. |
| `HONCHO_WORKSPACE_ID` | yes | — | Recommended: `agpod-memo`. |
| `AGPOD_MEMO_PEER_ID` | no | `agpod-agent` | All notes are written under this peer. |
| `AGPOD_MEMO_REPO_ROOT` | no | cwd | Used to derive `repo_id` from `git remote`. |
| `AGPOD_MEMO_LOG_LEVEL` | no | `info` | Logs go to stderr; stdout is MCP. |
| `AGPOD_MEMO_READONLY` | no | `false` | Truthy values hide `note` and `forget`. |

`repo_id` is `hex(sha256("v1:" + normalized_remote_url))[:16]` — the same
algorithm as `crates/agpod-case/src/repo_id.rs`. Session id is `memo_<repo_id>`.

## Tools

### `note`

Persist a standalone present-tense fact that `rg` cannot recover.

Inputs: `content` (required), `cues[]` (optional short phrases a later
agent will type into `find`). Output: `{id}`.

Cues are not categories. Skip them when `content` already contains the
path, command, or env name.

Each note writes a Honcho **message** (canonical, with metadata and a
`find:` keyword appendix) and a Honcho **conclusion** (clean body, for
semantic search and `peer.chat`).

### `find`

Search stored notes. `query` is required.

- `mode=search` (default): Honcho conclusion query (cosine distance ≤ 0.55)
  plus session hybrid search, plus a local **cue recall** scan of live notes.
  Hybrid hits with no cue/content overlap are dropped (noise). Conclusion
  hits are kept even without shared words (same-language paraphrase).
- `mode=ask`: Honcho `peer.chat` scoped to this repo session. Returns
  `{answer, unknown, degraded, quotes, ids}`. `unknown=true` means nothing
  was stored. `degraded=true` means chat was empty and the answer is the
  top search hit, not a synthesis.

### `forget`

Retire a note by `id`. Deletes the conclusion and marks the message
`retired`. Does not resurrect.

## Agent usage

```text
Before exploring   find({query: "<short phrase>"})
Unsure / need a synthesis   find({query, mode: "ask"})
Learned a fact grep cannot recover   note({content, cues?})
Fact is wrong or obsolete   forget({id})
```
