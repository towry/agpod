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
api_key_env = "HONCHO_API_KEY"
peer_id = "agpod-system"
```

## Honcho Fields

- `enabled` — enable Honcho integration
- `sync_enabled` — send qualifying case events to Honcho
- `base_url` — Honcho API base URL
- `workspace_id` — target Honcho workspace
- `api_key_env` — env var name that stores the API key
- `peer_id` — peer identifier used when writing messages

## Environment Overrides

Environment variables still override file config:

- `AGPOD_CASE_HONCHO_ENABLED`
- `AGPOD_CASE_HONCHO_SYNC_ENABLED`
- `AGPOD_CASE_SEMANTIC_RECALL`
- `AGPOD_CASE_VECTOR_DIGEST_JOB`
- `HONCHO_BASE_URL`
- `HONCHO_WORKSPACE_ID`
- `AGPOD_CASE_HONCHO_API_KEY_ENV`
- `AGPOD_CASE_HONCHO_PEER_ID`
- the env var named by `api_key_env` (default `HONCHO_API_KEY`)

## Notes

- If `case.plugins.honcho.enabled = false`, Honcho config is ignored.
- If Honcho is enabled, missing `base_url`, `workspace_id`, or API key env will fail fast.
- Keep secrets in environment variables, not in `.agpod.toml`.
