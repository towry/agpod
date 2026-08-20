# agent-memo MCP

Go MCP server that lets agents persist short facts to a Honcho v3 backend
and retrieve them later. Source: `internal/agpod-mcp/`.

Recommended Honcho workspace: `agpod-memo`. Session namespace: `memo_<repo_id>`.

## Build

```bash
cd internal/agpod-mcp
go build ./cmd/agpod-mcp
go test ./...
```

Remote agents: `AGPOD_MEMO_LISTEN=0.0.0.0:8742 AGPOD_MEMO_TOKEN=... ./agpod-mcp`,
then point the MCP client at `http://<host>:8742/mcp`.

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
| `AGPOD_MEMO_LISTEN` | no | — | If set (e.g. `127.0.0.1:8742`), serve Streamable HTTP instead of stdio. |
| `AGPOD_MEMO_TOKEN` | no | — | If set, HTTP requires `Authorization: Bearer`. |

`repo_id` is `hex(sha256("v1:" + normalized_remote_url))[:16]`. Session id is `memo_<repo_id>`.

## Tools

### `note`

Persist a standalone present-tense fact that `rg` cannot recover.

Inputs: `content` (required), `cues[]` (required: one or more short
phrases a later agent will type into `ask_note`). Output: `{id}`.

Cues are not categories. Auto-extracted paths/commands do not count.

Each note writes a Honcho **message** (canonical, with metadata and a
`find:` keyword appendix) and a Honcho **conclusion** (clean body, for
semantic search).

### `ask_note`

Ask stored notes. `query` is required. There is no search/ask mode switch.

Internally: Honcho conclusion query + session hybrid search + local cue/CJK
recall. Empty retrieval returns `{unknown: true}` without calling chat.
Hits then call Honcho `peer.chat` (low, 8s cap); chat unknown/timeout
falls back to the top hit as `answer` with `quotes`/`ids`.

### `forget`

Retire a note by `id`. Deletes the conclusion and marks the message
`retired`. Does not resurrect.

## Agent usage

```text
Before exploring   ask_note({query: "<short phrase>"})
Learned a fact grep cannot recover   note({content, cues})
Fact is wrong or obsolete   forget({id})
```
