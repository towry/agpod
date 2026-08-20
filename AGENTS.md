# agpod

Rust multi-crate CLI tool: diff minimization, VCS path formatting. Agent memory lives in the Go MCP server.

Source local env: `source .env.sh && <cmd>`

## Build & Test

- `cargo build -p agpod` — build CLI binary
- `cargo test` — run all tests
- `cargo clippy -p <crate> -- -D warnings` — lint (CI enforces `-D warnings`)
- `cargo fmt -p <crate> -- --check` — format check (CI enforces)
- Before committing: run `cargo fmt` and `cargo clippy -- -D warnings` on changed crates
- If you change behavior in a crate, run that crate's full test suite before committing, not only narrow tests.
- For agent-memo MCP smoke: `cd internal/agpod-mcp && go build -o /tmp/agpod-mcp ./cmd/agpod-mcp`, then `HONCHO_API_KEY=... HONCHO_WORKSPACE_ID=... /tmp/agpod-mcp` over stdio. Remote: set `AGPOD_MEMO_LISTEN=127.0.0.1:8742` and optional `AGPOD_MEMO_TOKEN`, connect clients to `http://127.0.0.1:8742/mcp`.

## Workspace Structure

- `crates/agpod` — CLI entrypoint
- `crates/agpod-core` — shared utilities
- `crates/agpod-diff` — diff minimization for LLM context
- `crates/agpod-vcs-path` — VCS branch/bookmark path formatting
- `internal/agpod-mcp` — Go MCP server for agent memory (`note` / `ask_note` / `forget`)

## Code Conventions

- Conventional commits: `topic(scope): message`
- Before adding dependencies: add to `[workspace.dependencies]` in root `Cargo.toml`, reference via `{ workspace = true }` in crate
- Before creating new files: follow existing crate module structure
- Before adding or removing a crate: update `release-please-config.json` and `.release-please-manifest.json` accordingly

## CI

- All warnings are errors (`-D warnings` for both rustc and clippy)
- Cross-compile targets: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`
- Always run `cargo fmt` after code changes before committing
