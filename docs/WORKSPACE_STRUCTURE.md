# Workspace Structure

`agpod` is a Rust workspace with focused crates. Agent memory lives in a separate Go module.

## Crates

### `agpod-core`

- Location: `crates/agpod-core/`
- Purpose: shared configuration helpers

### `agpod-diff`

- Location: `crates/agpod-diff/`
- Purpose: git diff minimization and saved review chunks

### `agpod-vcs-path`

- Location: `crates/agpod-vcs-path/`
- Purpose: annotate paths with Git/Jujutsu branch or bookmark metadata

### `agpod-mcp`

- Location: `internal/agpod-mcp/`
- Purpose: Go MCP server for agent memory (`note` / `ask_note` / `forget`), stdio or Streamable HTTP

### `agpod`

- Location: `crates/agpod/`
- Purpose: CLI entrypoint wiring `diff` and `vcs-path-info`

## Build

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

## Notes

- The root `Cargo.toml` defines workspace members and shared dependencies.
- CI and release workflows operate on the workspace directly.
