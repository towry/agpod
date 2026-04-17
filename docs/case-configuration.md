# Case Configuration

`agpod case` reads configuration from:

- `$XDG_CONFIG_HOME/agpod/config.toml`
- `~/.config/agpod/config.toml`
- `.agpod.toml`

Precedence is:

1. defaults
2. config file
3. environment variables
4. explicit CLI overrides

## Example

```toml
version = "1"

[log]
level = "warning"

[case]
server_addr = "127.0.0.1:6142"
auto_start = true
access_mode = "local_server"
semantic_recall_enabled = true
vector_digest_job_enabled = false

[case.plugins.honcho]
enabled = true
sync_enabled = true
base_url = "https://api.honcho.dev"
workspace_id = "ws_123"
api_key = "honcho_secret"
api_key_env = "HONCHO_API_KEY"
```

## Honcho Fields

- `enabled` — enable Honcho integration
- `sync_enabled` — queue qualifying case events for background Honcho sync
- `base_url` — Honcho API base URL
- `workspace_id` — target Honcho workspace
- `api_key` — raw API key stored directly in config
- `api_key_env` — env var name that stores the API key

## Environment Overrides

Environment variables still override file config:

- `AGPOD_CASE_HONCHO_ENABLED`
- `AGPOD_CASE_HONCHO_SYNC_ENABLED`
- `AGPOD_CASE_SEMANTIC_RECALL`
- `AGPOD_CASE_VECTOR_DIGEST_JOB`
- `HONCHO_BASE_URL`
- `HONCHO_WORKSPACE_ID`
- `AGPOD_CASE_HONCHO_API_KEY`
- `AGPOD_CASE_HONCHO_API_KEY_ENV`
- the env var named by `api_key_env` (default `HONCHO_API_KEY`)

## Logging

- Top-level `[log]` applies to CLI, MCP, and case-server
- `level` defaults to `warning`
- Logs are written to the platform data dir under `agpod/logs/`
- Typical files are `agpod.log`, `agpod-mcp.log`, and `agpod-case-server.log`

## Notes

- If `case.plugins.honcho.enabled = false`, Honcho config is ignored.
- If Honcho is enabled, missing `base_url`, `workspace_id`, and both `api_key` / API key env will fail fast.
- Mutation responses report hook enqueue / init failures only; background delivery failures are logged by `agpod-case-server`.
- Background sync is in-process best effort: delivery is queued per case in order while the process stays alive, not durably persisted across process exit.
- `api_key` is supported for convenience, but environment variables remain safer when practical.
- Some implementation-only fields are intentionally omitted from user-facing config docs.
