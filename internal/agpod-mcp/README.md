# agpod-mcp

agent-memo MCP server in Go. Three tools backed by [Honcho v3](https://docs.honcho.dev/v3):
`note`, `find`, `forget`.

This module supersedes the legacy Rust crate at `crates/agpod-mcp/`. Both
produce a binary named `agpod-mcp`, but only the Go one is installed under
`~/.local/bin/agpod-mcp` today.

The module lives outside the Cargo workspace with its own `go.mod`.

## Build

```bash
go build ./cmd/agpod-mcp
```

## Run (stdio)

```bash
HONCHO_API_KEY=xxx HONCHO_WORKSPACE_ID=agpod-memo \
  ./agpod-mcp
```

See `docs/agent-memo-mcp.md` for the env reference and agent-side usage.

## Test

```bash
go test ./...
```

Tests use `httptest` to mock the Honcho v3 endpoints — no network needed.
