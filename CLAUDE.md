# agpod

Rust multi-crate CLI tool: diff minimization, VCS path formatting. Agent memory lives in the Go MCP server.

## Build & Test

- `cargo build -p agpod` — build binary
- `cargo test` — run all tests
- `cargo clippy -p <crate> -- -D warnings` — lint (CI enforces `-D warnings`)
- `cargo fmt -p <crate> -- --check` — format check (CI enforces)
- Before committing: run `cargo fmt` and `cargo clippy -- -D warnings` on changed crates

## Workspace Structure

- `crates/agpod` — CLI entrypoint
- `crates/agpod-core` — shared utilities
- `crates/agpod-diff` — diff minimization for LLM context
- `crates/agpod-vcs-path` — VCS branch/bookmark path formatting
- `internal/agpod-mcp` — Go-based agent-memo MCP server (own `go.mod`, not in Cargo workspace)

## Code Conventions

- Conventional commits: `topic(scope): message`
- Before adding dependencies: add to `[workspace.dependencies]` in root `Cargo.toml`, reference via `{ workspace = true }` in crate
- Before creating new files: follow existing crate module structure
- Before adding or removing a crate: update `release-please-config.json` and `.release-please-manifest.json` accordingly
- If using the agent-memo MCP server (`note` / `ask_note` / `forget`): see `docs/agents-md/agent-memo.md`

## CI

- All warnings are errors (`-D warnings` for both rustc and clippy)
- Cross-compile targets: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`
- Always run `cargo fmt` after code changes before committing
