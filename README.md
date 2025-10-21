# agpod

[![CI](https://img.shields.io/github/actions/workflow/status/towry/agpod/ci.yml?branch=main&label=CI&logo=github)](https://github.com/towry/agpod/actions/workflows/ci.yml)
[![Rust Tests](https://img.shields.io/github/actions/workflow/status/towry/agpod/rust.yml?branch=main&label=tests&logo=rust)](https://github.com/towry/agpod/actions/workflows/rust.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org)
![GitHub Copilot](https://img.shields.io/badge/Github-Copilot-blue?logo=githubcopilot)
<a href="https://github.com/sponsors/towry">![GitHub Sponsors](https://img.shields.io/github/sponsors/towry)</a>
<a href="https://github.com/towry/agpod/pulls">![GitHub Issues or Pull Requests](https://img.shields.io/github/issues-pr/towry/agpod)</a>
![GitHub Repo stars](https://img.shields.io/github/stars/towry/agpod)

A powerful agent helper tool for optimizing git diffs for LLM context and managing PR drafts locally.

## Features

### Diff Minimization
- **Smart summarization**: Large files (>100 changes) show metadata only
- **Token optimization**: Removes excessive empty lines while preserving structure
- **Save mode**: Split diffs into reviewable chunks with status tracking

### Kiro Workflow
- **PR draft management**: Organize design docs, tasks, and implementation notes locally
- **Template system**: Customizable templates for different project types
- **Git integration**: Auto-create branches and manage workflow state

## Installation

### Quick install (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/towry/agpod/main/install.sh | bash
```

**Requirements:** macOS with Apple Silicon (M1/M2/M3)

### From source

```bash
git clone https://github.com/towry/agpod.git
cd agpod
cargo build --release
# Binary available at target/release/agpod
```

## Usage

### Diff Minimization

```bash
# Minimize a git diff
git diff | agpod diff

# Save chunks for review workflow
git diff | agpod diff --save

# Custom output path
git diff | agpod diff --save --save-path custom/path

# Integration examples
git diff | agpod diff | pbcopy              # Copy to clipboard (macOS)
git diff | agpod diff > minimized_diff.txt  # Save to file
```

**Save mode** creates separate chunk files with `REVIEW.md` for tracking review status. See [SAVE_OPTION_SUMMARY.md](docs/SAVE_OPTION_SUMMARY.md) for details.

### Kiro Workflow

```bash
# Initialize configuration and templates
agpod kiro init

# Create a new PR draft
agpod kiro pr-new --desc "implement user login"

# Create with template and git branch
agpod kiro pr-new --desc "add JWT module" --template rust --git-branch

# List PR drafts
agpod kiro pr-list

# Interactive PR selection
agpod kiro pr
```

See [KIRO_GUIDE.md](docs/KIRO_GUIDE.md) for comprehensive workflow documentation.

## Configuration

agpod supports feature-specific configuration through `config.toml` files. Configuration can be placed at:

- **Global**: `$XDG_CONFIG_HOME/agpod/config.toml` or `~/.config/agpod/config.toml` - Applies to all projects
- **Project**: `.agpod.toml` in project root - Project-specific overrides

The global configuration location respects the `XDG_CONFIG_HOME` environment variable, making it easy to test different configurations without affecting your default setup.

> **⚠️ Breaking Change in v0.5.0**: The configuration format has changed to use structured sections. The old flat format is no longer supported. You must update your config files to use `[kiro]` and `[diff]` sections.

### Configuration Structure

The configuration file uses a versioned schema to track changes and ensure compatibility:

```toml
# Configuration version (current: "1")
version = "1"

# Kiro workflow settings
[kiro]
base_dir = "llm/kiro"
templates_dir = "~/.config/agpod/templates"
template = "default"

# Diff minimization settings
[diff]
output_dir = "llm/diff"
large_file_changes_threshold = 100
large_file_lines_threshold = 500
max_consecutive_empty_lines = 2
```

The `version` field helps track configuration schema changes over time, allowing agpod to:
- Detect deprecated configuration options
- Provide migration warnings when needed
- Maintain compatibility across versions

See [examples/config.toml](examples/config.toml) for a complete configuration example.

## Architecture

agpod is built as a modular Rust library with clean separation of concerns:

- **`agpod::diff`** - Git diff minimization and processing logic
- **`agpod::kiro`** - PR draft workflow management  
- **`agpod::config`** - Unified configuration system

This modular design allows:
- Easy addition of new features
- Publishing individual modules as libraries
- Clear separation between features
- Independent testing of each component

## How It Works

### Diff Minimization Strategy

1. **Large files** (>100 changes or >500 lines): Shows metadata only (filename, change type, line count)
2. **Regular files**: Preserves full diff with reduced empty lines (max 2 consecutive)
3. **All file types**: Handles added, deleted, modified, and renamed files

## Development

agpod is structured as a Rust workspace with multiple crates:

- **agpod-core**: Core configuration and utilities
- **agpod-diff**: Git diff minimization functionality
- **agpod-kiro**: PR draft workflow management
- **agpod**: CLI binary that integrates all features

### Building from Source

```bash
# Clone the repository
git clone https://github.com/towry/agpod.git
cd agpod

# Build all workspace crates
cargo build --release

# Run tests
cargo test

# Run linting
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt --all
```

### Workspace Structure

```
agpod/
├── Cargo.toml          # Workspace root
├── crates/
│   ├── agpod-core/     # Core configuration library
│   ├── agpod-diff/     # Diff processing library
│   ├── agpod-kiro/     # Kiro workflow library
│   └── agpod/          # Binary crate (CLI)
├── examples/           # Example templates and configs
└── test_data/          # Test fixtures
```

## Support This Project

This project is maintained using AI coding agents, which incur operational costs. If you find agpod useful, please consider supporting its development:

- ⭐ **Star this repository** - Show your appreciation and help others discover agpod
- ☕ **[Sponsor on GitHub](https://github.com/sponsors/towry)** - Support ongoing development and maintenance (if available)
- 🐦 **Share with others** - Help grow the community by sharing agpod with developers who might benefit

Your support helps cover the costs of running AI agents and keeps this project actively maintained and improved. Every contribution, big or small, makes a difference!

## License

MIT License - see [LICENSE](LICENSE) file for details.
