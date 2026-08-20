# agpod

[![CI](https://img.shields.io/github/actions/workflow/status/towry/agpod/ci.yml?branch=main&label=CI&logo=github)](https://github.com/towry/agpod/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org)

`agpod` is a Rust CLI for two concrete workflows:

- minimize git diffs for LLM context
- print repository paths with Git or Jujutsu branch metadata

Agent memory (`note` / `ask_note` / `forget`) lives in the Go MCP server at `internal/agpod-mcp`.

## Features

### Diff

- summarize oversized file diffs
- reduce empty-line noise while preserving patch structure
- optionally save review chunks with a `REVIEW.md` checklist

### VCS Path Info

- append branch or bookmark metadata to paths
- filter to repository paths only
- fit shell pipelines such as `zoxide`, `fzf`, and custom prompts

## Installation

### From source

```bash
git clone https://github.com/towry/agpod.git
cd agpod
cargo build --release
```

Built binaries:

- `target/release/agpod`

## Usage

### Diff

```bash
git diff | agpod diff
git diff | agpod diff --save
git diff | agpod diff --save --save-path custom/path
```

### VCS Path Info

```bash
echo "/path/to/repo" | agpod vcs-path-info
echo "/path/to/repo" | agpod vcs-path-info -f "{path} [{branch}]"
zoxide query --list | agpod vcs-path-info --filter -f "{path} [{branch}]" | fzf
```

## Configuration

Global config:

- `$XDG_CONFIG_HOME/agpod/config.toml`
- `~/.config/agpod/config.toml`

Repo-local override:

- `.agpod.toml`

Example:

```toml
version = "1"

[log]
level = "warning"

[diff]
output_dir = "llm/diff"
large_file_changes_threshold = 100
large_file_lines_threshold = 500
max_consecutive_empty_lines = 2
```

See [examples/config.toml](examples/config.toml).

Logs default to `warning` and are written under the platform data directory in `agpod/logs/`, for example `~/Library/Application Support/agpod/logs/agpod.log` and `agpod-mcp.log`.

## Workspace

- `crates/agpod` - CLI entrypoint
- `crates/agpod-core` - shared configuration helpers
- `crates/agpod-diff` - diff minimization
- `crates/agpod-vcs-path` - VCS path formatting
- `internal/agpod-mcp` - Go MCP server for agent memory (`note` / `ask_note` / `forget`)

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

## License

MIT. See [LICENSE](LICENSE).
