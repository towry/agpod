# Workspace Structure

`agpod` is a Rust workspace with focused crates.

## Crates

### `agpod-core`

- Location: `crates/agpod-core/`
- Purpose: shared configuration helpers

### `agpod-diff`

- Location: `crates/agpod-diff/`
- Purpose: git diff minimization and saved review chunks

### `agpod-case`

- Location: `crates/agpod-case/`
- Purpose: structured exploration case tracking with steps and event logs

### `agpod-vcs-path`

- Location: `crates/agpod-vcs-path/`
- Purpose: annotate paths with Git/Jujutsu branch or bookmark metadata

### `agpod-mcp`

- Location: `crates/agpod-mcp/`
- Purpose: MCP server exposing `agpod-case` workflows

### `agpod`

- Location: `crates/agpod/`
- Purpose: CLI entrypoint wiring `diff`, `case`, and `vcs-path-info`

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
