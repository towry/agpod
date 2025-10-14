# Agpod Kilo Examples

This directory contains example configurations and templates for the `agpod kilo` workflow.

## Directory Structure

```
examples/
├── config.toml                      # Example configuration file
├── templates/                       # Template directories
│   ├── default/                    # Default template
│   │   ├── DESIGN.md.j2
│   │   └── TASK.md.j2
│   ├── vue/                        # Vue-specific template
│   │   ├── DESIGN.md.j2
│   │   ├── TASK.md.j2
│   │   └── COMPONENT.md.j2
│   └── rust/                       # Rust-specific template
│       ├── DESIGN.md.j2
│       ├── TASK.md.j2
│       └── IMPL.md.j2
├── plugins/                        # Example plugins
│   └── branch_name.sh             # Custom branch name generator
└── README.md                       # This file
```

## Setup

1. **Copy configuration to your config directory:**
   ```bash
   mkdir -p ~/.config/agpod
   cp examples/config.toml ~/.config/agpod/config.toml
   ```

2. **Copy templates:**
   ```bash
   mkdir -p ~/.config/agpod/templates
   cp -r examples/templates/* ~/.config/agpod/templates/
   ```

3. **Copy and enable plugins (optional):**
   ```bash
   mkdir -p ~/.config/agpod/plugins
   cp examples/plugins/branch_name.sh ~/.config/agpod/plugins/
   chmod +x ~/.config/agpod/plugins/branch_name.sh
   ```

## Usage

### Create a new PR draft

```bash
# Using default template
agpod kilo pr-new --desc "实现用户登录功能"

# Using Vue template
agpod kilo pr-new --desc "添加登录表单组件" --template vue

# Using Rust template
agpod kilo pr-new --desc "实现JWT认证模块" --template rust
```

### List PR drafts

```bash
# Table format
agpod kilo pr-list

# JSON format
agpod kilo --json pr-list
```

### Interactive selection

```bash
# Using built-in selector
agpod kilo pr

# Using fzf (if available)
agpod kilo pr --fzf

# Get absolute path
agpod kilo pr --output abs

# Use in shell scripts
selected=$(agpod kilo pr)
cd llm/kilo/$selected
```

### Shortcut flags

```bash
# Equivalent to pr-new
agpod kilo --pr-new "添加新功能"

# Equivalent to pr-list
agpod kilo --pr-list

# Equivalent to pr
agpod kilo --pr
```

## Template Variables

Templates have access to the following variables:

- `branch_name`: Generated branch name
- `desc`: Description provided by user
- `template`: Template name being used
- `now`: Current timestamp (ISO 8601)
- `date`: Current date (YYYY-MM-DD)
- `user`: Current system user
- `base_dir`: Base directory path
- `pr_dir_abs`: Absolute path to PR directory
- `pr_dir_rel`: Relative path to PR directory
- `git.repo_root`: Git repository root (if in a git repo)
- `git.current_branch`: Current git branch
- `git.short_sha`: Short commit SHA
- `config`: Full configuration object

## Template Filters

- `slugify`: Convert text to URL-safe slug
- `truncate(n)`: Truncate string to n characters

## Custom Plugins

The example `branch_name.sh` plugin demonstrates how to:
- Access environment variables passed by agpod
- Generate custom branch names
- Handle errors gracefully

Environment variables available to plugins:
- `AGPOD_DESC`: User-provided description
- `AGPOD_TEMPLATE`: Template name
- `AGPOD_BRANCH_PREFIX`: Suggested prefix
- `AGPOD_TIME_ISO`: Current timestamp
- `AGPOD_BASE_DIR`: Base directory
- `AGPOD_REPO_ROOT`: Git repository root
- `AGPOD_USER`: Current user
- Plus any variables matching patterns in `pass_env`

## Configuration Priority

Configuration is loaded in this order (later sources override earlier ones):

1. Default values
2. Global config: `~/.config/agpod/config.toml`
3. Repository config: `.agpod.toml` (in project root)
4. Environment variables: `AGPOD_*`
5. CLI arguments

## Tips

- Use `--dry-run` to preview actions without creating files
- Use `--log-level debug` for verbose output
- Templates ending in `.j2` will have that extension removed in output
- Plugin failures automatically fall back to default branch name generation
- Use JSON output for scripting: `agpod kilo --json pr-list`
