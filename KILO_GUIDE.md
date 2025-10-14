# Agpod Kilo Workflow Guide

The `kilo` subcommand provides a powerful workflow for managing PR drafts locally. It helps you organize design documents, tasks, and implementation notes in a structured way.

## Quick Start

### Installation

Build from source:
```bash
cargo build --release
sudo cp target/release/agpod /usr/local/bin/
```

### Basic Setup

1. Create configuration directory:
```bash
mkdir -p ~/.config/agpod/templates
```

2. Copy example templates:
```bash
cp -r examples/templates/* ~/.config/agpod/templates/
```

3. (Optional) Copy and configure example config:
```bash
cp examples/config.toml ~/.config/agpod/config.toml
```

## Commands

### Creating a New PR Draft

```bash
# Basic usage
agpod kilo pr-new --desc "实现用户登录功能"

# With specific template
agpod kilo pr-new --desc "添加登录表单" --template vue

# With git branch creation
agpod kilo pr-new --desc "实现JWT模块" --template rust --git-branch

# Open in editor after creation
agpod kilo pr-new --desc "数据库连接池" --open

# Shortcut flag (backward compatible)
agpod kilo --pr-new "实现新功能"
```

**Output:**
- Prints branch name to stdout (machine-readable)
- Logs creation details to stderr
- Creates directory: `llm/kilo/<branch-name>/`
- Renders template files (DESIGN.md, TASK.md, etc.)

### Listing PR Drafts

```bash
# Table format
agpod kilo pr-list

# JSON format (for scripting)
agpod kilo --json pr-list

# Shortcut flag
agpod kilo --pr-list
```

**Output:**
- Shows directory name and summary from DESIGN.md
- Supports both human-readable and JSON formats

### Interactive Selection

```bash
# Built-in selector
agpod kilo pr

# With fzf (if installed)
agpod kilo pr --fzf

# Get absolute path
agpod kilo pr --output abs

# Get name only
agpod kilo pr --output name

# Shortcut flag
agpod kilo --pr
```

**Use in scripts:**
```bash
# Navigate to selected draft
selected=$(agpod kilo pr)
cd llm/kilo/$selected

# Open in editor
code llm/kilo/$(agpod kilo pr)
```

## Configuration

### Configuration Files

Priority (highest to lowest):
1. CLI arguments
2. Environment variables (`AGPOD_*`)
3. Repository config (`.agpod.toml`)
4. Global config (`~/.config/agpod/config.toml`)
5. Default values

### Example Configuration

```toml
# ~/.config/agpod/config.toml
base_dir = "llm/kilo"
templates_dir = "~/.config/agpod/templates"
plugins_dir = "~/.config/agpod/plugins"
template = "default"
summary_lines = 3

[plugins.branch_name]
enabled = true
command = "branch_name.sh"
timeout_secs = 3
pass_env = ["AGPOD_*", "GIT_*", "USER", "HOME"]

[rendering]
files = ["DESIGN.md.j2", "TASK.md.j2"]
missing_policy = "error"

[templates.vue]
files = ["DESIGN.md.j2", "TASK.md.j2", "COMPONENT.md.j2"]
missing_policy = "skip"
```

### Environment Variables

Override any configuration with environment variables:

```bash
export AGPOD_BASE_DIR=~/work/drafts
export AGPOD_TEMPLATES_DIR=~/.agpod/templates
export AGPOD_DEFAULT_TEMPLATE=rust
export AGPOD_LOG_LEVEL=debug
```

### CLI Overrides

```bash
agpod kilo \
  --base-dir ~/custom/path \
  --templates-dir ~/templates \
  --log-level debug \
  pr-new --desc "测试"
```

## Templates

### Template Variables

Available in all templates:

- `branch_name`: Generated branch name
- `desc`: User description
- `template`: Template name
- `now`: ISO 8601 timestamp
- `date`: Date (YYYY-MM-DD)
- `user`: Current system user
- `base_dir`: Base directory path
- `pr_dir_abs`: Absolute PR directory path
- `pr_dir_rel`: Relative PR directory path
- `git.repo_root`: Git repository root (if available)
- `git.current_branch`: Current git branch
- `git.short_sha`: Short commit SHA
- `config`: Full configuration object

### Template Filters

- `slugify`: Convert to URL-safe slug
- `truncate(n)`: Truncate to n characters

### Example Template

```jinja2
# {{ desc }}

Branch: `{{ branch_name }}`
Created: {{ now }}
Author: {{ user }}

{% if git.repo_root %}
Repository: {{ git.repo_root }}
Current branch: {{ git.current_branch | default("none") }}
{% endif %}

## Description
{{ desc }}

## Slug Example
{{ desc | slugify }}
```

### Creating Custom Templates

1. Create template directory:
```bash
mkdir -p ~/.config/agpod/templates/mytemplate
```

2. Add template files (ending in `.j2`):
```bash
touch ~/.config/agpod/templates/mytemplate/DESIGN.md.j2
touch ~/.config/agpod/templates/mytemplate/TASK.md.j2
```

3. Use the template:
```bash
agpod kilo pr-new --desc "test" --template mytemplate
```

## Plugins

### Branch Name Plugin

Create custom branch name generator:

```bash
#!/usr/bin/env bash
# ~/.config/agpod/plugins/branch_name.sh
set -euo pipefail

desc="${AGPOD_DESC:-}"
prefix="${AGPOD_BRANCH_PREFIX:-feature}"

# Custom logic here
slug=$(echo "$desc" | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
rand=$(tr -dc 'a-z0-9' </dev/urandom | head -c 6)

echo "${prefix}-${slug}-${rand}"
```

Make it executable:
```bash
chmod +x ~/.config/agpod/plugins/branch_name.sh
```

Enable in config:
```toml
[plugins.branch_name]
enabled = true
command = "branch_name.sh"
timeout_secs = 3
pass_env = ["AGPOD_*", "GIT_*", "USER", "HOME"]
```

### Plugin Environment Variables

Plugins receive:
- `AGPOD_DESC`: Description
- `AGPOD_TEMPLATE`: Template name
- `AGPOD_BRANCH_PREFIX`: Suggested prefix
- `AGPOD_TIME_ISO`: Current timestamp
- `AGPOD_BASE_DIR`: Base directory
- `AGPOD_REPO_ROOT`: Git repo root
- `AGPOD_USER`: Current user
- Any variables matching `pass_env` patterns

## Advanced Usage

### Dry Run

Preview without creating files:
```bash
agpod kilo --dry-run pr-new --desc "test"
```

### Force Overwrite

Overwrite existing directory:
```bash
agpod kilo pr-new --desc "test" --force
```

### Custom Output Directory

```bash
agpod kilo --base-dir ~/my-drafts pr-new --desc "test"
```

### Scripting

```bash
# Create draft and open in editor
draft=$(agpod kilo pr-new --desc "新功能")
code llm/kilo/$draft

# List all drafts in JSON
agpod kilo --json pr-list | jq -r '.[].name'

# Select and navigate
cd llm/kilo/$(agpod kilo pr --output name)
```

## Troubleshooting

### Plugin Not Found

```
Warning: Plugin not found at /path/to/plugin.sh, using default branch name generation
```

**Solution:** Ensure plugin path is correct and file is executable:
```bash
chmod +x ~/.config/agpod/plugins/branch_name.sh
```

### Template Not Found

```
Error: Template not found: DESIGN.md.j2 in /path/to/template
```

**Solution:** Check template directory exists and contains required files:
```bash
ls ~/.config/agpod/templates/default/
```

### Directory Already Exists

```
Error: Directory already exists: llm/kilo/feature-...
```

**Solution:** Use `--force` flag or choose different description:
```bash
agpod kilo pr-new --desc "new description"
# or
agpod kilo pr-new --desc "same description" --force
```

## Tips

1. **Use Chinese descriptions freely** - The slug generator handles Chinese characters via pinyin conversion

2. **Leverage templates** - Create domain-specific templates (vue, rust, python, etc.)

3. **Plugin fallback** - Plugin failures automatically fall back to default generation

4. **JSON for automation** - Use `--json` flag for scripting and automation

5. **Git integration** - Use `--git-branch` to automatically create and checkout branches

6. **Editor integration** - Use `--open` to immediately start editing after creation

7. **Environment variables** - Use `AGPOD_*` variables for project-specific defaults

8. **Repo-local config** - Place `.agpod.toml` in project root for team-shared settings

## Examples

See the `examples/` directory for:
- Complete configuration file
- Default, Vue, and Rust templates
- Example plugin script
- Detailed README with more examples

## Support

For issues or questions:
- GitHub: https://github.com/towry/agpod
- Documentation: See examples/ directory
