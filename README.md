# agpod

[![CI](https://img.shields.io/github/actions/workflow/status/towry/agpod/ci.yml?branch=main&label=CI&logo=github)](https://github.com/towry/agpod/actions/workflows/ci.yml)
[![Rust Tests](https://img.shields.io/github/actions/workflow/status/towry/agpod/rust.yml?branch=main&label=tests&logo=rust)](https://github.com/towry/agpod/actions/workflows/rust.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org)
![GitHub Copilot](https://img.shields.io/badge/github_copilot-8957E5?style=for-the-badge&logo=github-copilot&logoColor=white)
![GitHub Sponsors](https://img.shields.io/github/sponsors/towry)
![Gitea Pull Requests](https://img.shields.io/gitea/pull-requests/open/towry/agpod)
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

**Save mode** creates separate chunk files with `REVIEW.md` for tracking review status. See [SAVE_OPTION_SUMMARY.md](SAVE_OPTION_SUMMARY.md) for details.

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

See [KIRO_GUIDE.md](KIRO_GUIDE.md) for comprehensive workflow documentation.

## How It Works

### Diff Minimization Strategy

1. **Large files** (>100 changes or >500 lines): Shows metadata only (filename, change type, line count)
2. **Regular files**: Preserves full diff with reduced empty lines (max 2 consecutive)
3. **All file types**: Handles added, deleted, modified, and renamed files

## Support This Project

This project is maintained using AI coding agents, which incur operational costs. If you find agpod useful, please consider supporting its development:

- ⭐ **Star this repository** - Show your appreciation and help others discover agpod
- ☕ **[Sponsor on GitHub](https://github.com/sponsors/towry)** - Support ongoing development and maintenance (if available)
- 🐦 **Share with others** - Help grow the community by sharing agpod with developers who might benefit

Your support helps cover the costs of running AI agents and keeps this project actively maintained and improved. Every contribution, big or small, makes a difference!

## License

MIT License - see [LICENSE](LICENSE) file for details.
