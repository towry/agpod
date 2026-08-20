# agpod-mcp

agent-memo MCP server in Go. Three tools backed by [Honcho v3](https://docs.honcho.dev/v3):
`note`, `ask_note`, `forget`.

Stdio is the default. Set `AGPOD_MEMO_LISTEN` to serve Streamable HTTP for remote agents.

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

## Run (HTTP)

```bash
HONCHO_API_KEY=xxx HONCHO_WORKSPACE_ID=agpod-memo \
  AGPOD_MEMO_LISTEN=127.0.0.1:8742 AGPOD_MEMO_TOKEN=secret \
  ./agpod-mcp
```

Clients connect to `http://127.0.0.1:8742/mcp` with `Authorization: Bearer secret`.
Health: `GET /healthz`.

See `docs/agent-memo-mcp.md` for the env reference and agent-side usage.

## Test

```bash
go test ./...
```

Tests use `httptest` to mock the Honcho v3 endpoints — no network needed.
