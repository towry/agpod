# agpod

[![CI](https://img.shields.io/github/actions/workflow/status/towry/agpod/ci.yml?branch=main&label=CI&logo=github)](https://github.com/towry/agpod/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org)

`agpod` is a Rust CLI for three concrete workflows:

- minimize git diffs for LLM context
- track exploration work as structured cases
- print repository paths with Git or Jujutsu branch metadata

## Features

### Diff

- summarize oversized file diffs
- reduce empty-line noise while preserving patch structure
- optionally save review chunks with a `REVIEW.md` checklist

### Case

- open, redirect, close, and resume structured exploration cases
- record findings, decisions, blockers, and ordered execution steps
- emit machine-readable JSON and support MCP-based automation

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
- `target/release/agpod-mcp`

## Usage

### Diff

```bash
git diff | agpod diff
git diff | agpod diff --save
git diff | agpod diff --save --save-path custom/path
```

See [docs/SAVE_OPTION_SUMMARY.md](docs/SAVE_OPTION_SUMMARY.md).

### Case

```bash
agpod case open \
  --goal "find the root cause" \
  --direction "inspect the failing path first"

agpod case current

agpod case step add \
  --id C-550e8400-e29b-41d4-a716-446655440000 \
  --title "collect logs" \
  --start

agpod case record \
  --id C-550e8400-e29b-41d4-a716-446655440000 \
  --kind evidence \
  --summary "captured the failing request"
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

[diff]
output_dir = "llm/diff"
large_file_changes_threshold = 100
large_file_lines_threshold = 500
max_consecutive_empty_lines = 2

[case]
server_addr = "127.0.0.1:6142"
auto_start = true
access_mode = "local_server"
semantic_recall_enabled = false
vector_digest_job_enabled = false

[case.plugins.honcho]
enabled = false
sync_enabled = true
# base_url = "https://api.honcho.dev"
# workspace_id = "ws_123"
# api_key_env = "HONCHO_API_KEY"
# peer_id = "agpod-system"
```

See [examples/config.toml](examples/config.toml).

Environment variables still override file config. Common Honcho-related overrides:

- `AGPOD_CASE_HONCHO_ENABLED=true`
- `AGPOD_CASE_HONCHO_SYNC_ENABLED=true`
- `AGPOD_CASE_SEMANTIC_RECALL=true`
- `HONCHO_BASE_URL=https://api.honcho.dev`
- `HONCHO_WORKSPACE_ID=ws_123`
- `AGPOD_CASE_HONCHO_API_KEY_ENV=HONCHO_API_KEY`
- `AGPOD_CASE_HONCHO_PEER_ID=agpod-system`
- `HONCHO_API_KEY=...`

## Workspace

- `crates/agpod` - CLI entrypoint
- `crates/agpod-core` - shared configuration helpers
- `crates/agpod-diff` - diff minimization
- `crates/agpod-case` - exploration case tracker
- `crates/agpod-vcs-path` - VCS path formatting
- `crates/agpod-mcp` - MCP server for case workflows

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

## License

MIT. See [LICENSE](LICENSE).
