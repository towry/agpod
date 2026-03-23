# agpod

Rust multi-crate CLI tool: diff minimization, exploration case tracking, VCS path formatting.

## Build & Test

- `cargo build -p agpod` — build binary
- `cargo test` — run all tests
- `cargo clippy -p <crate> -- -D warnings` — lint (CI enforces `-D warnings`)
- `cargo fmt -p <crate> -- --check` — format check (CI enforces)
- Before committing: run `cargo fmt` and `cargo clippy -- -D warnings` on changed crates
- For quick dev smoke on local build artifacts: first run `cargo build -p agpod -p agpod-mcp -p agpod-case-server`
- If you changed `agpod-case` CLI/RPC shapes, rebuild `agpod-case-server` too before MCP smoke, because `agpod-mcp` auto-starts that sibling binary and stale builds can reject new request payloads
- For case CLI smoke with isolated data: use `AGPOD_CASE_DATA_DIR=/tmp/agpod-case-smoke.db AGPOD_CASE_SERVER_ADDR=127.0.0.1:6142 target/debug/agpod case list --json`
- For cross-repo case smoke: use `AGPOD_CASE_DATA_DIR=/tmp/agpod-case-smoke.db target/debug/agpod case --repo-root <abs-repo-path> list --json`
- For explicit server smoke: run `target/debug/agpod-case-server --data-dir /tmp/agpod-case-smoke.db --server-addr 127.0.0.1:6142`, then in another shell run `target/debug/agpod case current --json`
- Before any case CLI/MCP smoke that depends on auto-start or a fresh local server, check whether an older `agpod-case-server` is already listening on the target address (for example `127.0.0.1:6142`). A stale long-running server can shadow newly built binaries, hide `case_finish` confirmation changes, and make `case_open mode=reopen` look broken. Kill the old process or use a different `AGPOD_CASE_SERVER_ADDR` before trusting smoke results.
- For concurrent multi-repo smoke against case-server: create two temp git repos with different remotes, point both to one `AGPOD_CASE_DATA_DIR` and one `AGPOD_CASE_SERVER_ADDR`, then run `target/debug/agpod case --repo-root <repo-a> open ...` and `target/debug/agpod case --repo-root <repo-b> open ...` in parallel; both `open` and follow-up `list --json` should succeed, proving the server reuses one DB client across requests
- For MCP smoke on local build output: run `target/debug/agpod-mcp` and verify `case_current` / `case_open` over stdio; if you need example command patterns, see `.agents/docs/feedback.md`
- For MCP stdio debugging: keep stdin open and send one JSON-RPC per line in order `initialize` -> `notifications/initialized` -> `tools/list` -> `tools/call`; a working local smoke is `cd <repo> && AGPOD_CASE_DATA_DIR=/tmp/agpod-case-smoke-stdio.db python3 - <<'PY'` then spawn `target/debug/agpod-mcp`, write JSON lines, and read line responses
- For MCP stdio call shape: `tools/call` uses `{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"case_current","arguments":{}}}`; local smoke should return structured `isError: true` with message `no open case in this repository` on an empty temp DB

## Workspace Structure

- `crates/agpod` — CLI entrypoint
- `crates/agpod-core` — shared utilities
- `crates/agpod-diff` — diff minimization for LLM context
- `crates/agpod-case` — exploration case tracker (SurrealDB embedded, RocksDB backend)
- `crates/agpod-vcs-path` — VCS branch/bookmark path formatting

## Code Conventions

- Conventional commits: `topic(scope): message`
- Before adding dependencies: add to `[workspace.dependencies]` in root `Cargo.toml`, reference via `{ workspace = true }` in crate
- Before creating new files: follow existing crate module structure (`cli.rs`, `commands.rs`, `client.rs`, `config.rs`, `error.rs`, `types.rs`)
- Before adding or removing a crate: update `release-please-config.json` and `.release-please-manifest.json` accordingly
- If modifying agpod-case client/queries: see `docs/agents-md/case-surrealdb.md`

## CI

- All warnings are errors (`-D warnings` for both rustc and clippy)
- Cross-compile targets: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`
- Always run `cargo fmt` after code changes before committing
