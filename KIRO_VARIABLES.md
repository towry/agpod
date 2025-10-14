# Kiro Template Variables

This document lists all the variables available in Kiro templates (Jinja2 `.j2` files).

## Template Variables

When creating or customizing templates in `~/.config/agpod/templates/`, you can use the following variables:

### Basic Variables

| Variable | Type | Description | Example |
|----------|------|-------------|---------|
| `name` | string | Generated branch/PR name | `feature-user-login` |
| `desc` | string | User-provided description | `实现用户登录功能` |
| `template` | string | Template name being used | `vue`, `rust`, `default` |
| `now` | string | Current timestamp in ISO 8601 format | `2024-03-15T10:30:00Z` |
| `date` | string | Current date in YYYY-MM-DD format | `2024-03-15` |
| `user` | string | Current system user | `john` |

### Path Variables

| Variable | Type | Description | Example |
|----------|------|-------------|---------|
| `base_dir` | string | Base directory for PR drafts | `llm/kiro` |
| `pr_dir_abs` | string | Absolute path to PR directory | `/home/user/project/llm/kiro/feature-login` |
| `pr_dir_rel` | string | Relative path to PR directory | `feature-user-login` |

### Git Variables

Git variables are only available when the command is run inside a git repository.

| Variable | Type | Description | Example |
|----------|------|-------------|---------|
| `git.repo_root` | string | Git repository root path | `/home/user/project` |
| `git.current_branch` | string | Current git branch name | `main` |
| `git.short_sha` | string | Short commit SHA | `abc1234` |

### Configuration Variable

| Variable | Type | Description |
|----------|------|-------------|
| `config` | object | Full configuration object containing all config values |

## Template Filters

Filters can be applied to variables using the pipe operator `|`.

| Filter | Usage | Description | Example Input | Example Output |
|--------|-------|-------------|---------------|----------------|
| `slugify` | `{{ desc \| slugify }}` | Convert text to URL-safe slug | `Hello World Test` | `hello-world-test` |
| `truncate(n)` | `{{ desc \| truncate(20) }}` | Truncate string to n characters | `This is a very long description` | `This is a very l...` |

## Usage Examples

### Basic Variable Usage

```jinja2
# {{ desc }}

Branch: `{{ name }}`
Created: {{ now }}
Author: {{ user }}
```

### Conditional Git Information

```jinja2
{% if git.repo_root %}
Repository: {{ git.repo_root }}
Current branch: {{ git.current_branch | default("none") }}
{% endif %}
```

### Using Filters

```jinja2
# {{ desc }}

Slug: {{ desc | slugify }}
Short description: {{ desc | truncate(50) }}
```

### Path Information

```jinja2
## Paths

- Relative: `{{ pr_dir_rel }}`
- Absolute: `{{ pr_dir_abs }}`
- Base: `{{ base_dir }}`
```

### Full Example Template

```jinja2
# {{ desc }}

## Metadata

- Branch: `{{ name }}`
- Template: `{{ template }}`
- Created: {{ now }}
- Date: {{ date }}
- Author: {{ user }}

{% if git.repo_root %}
## Git Information

- Repository: {{ git.repo_root }}
- Current branch: {{ git.current_branch | default("N/A") }}
- Commit: {{ git.short_sha | default("N/A") }}
{% endif %}

## Description

{{ desc }}

## Slug

{{ desc | slugify }}

## Paths

- Directory: `{{ pr_dir_rel }}`
- Full path: `{{ pr_dir_abs }}`
```

## Template Inheritance

Templates support Jinja2's `{% extends %}` directive for template inheritance. See `TEMPLATE_EXTENSION.md` in the examples directory for detailed documentation.

## See Also

- [KIRO_GUIDE.md](./KIRO_GUIDE.md) - Complete guide to using the Kiro workflow
- [examples/templates/](./examples/templates/) - Example templates using these variables
- [examples/README.md](./examples/README.md) - Examples and setup instructions
