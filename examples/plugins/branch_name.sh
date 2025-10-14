#!/usr/bin/env bash
# Example plugin for custom branch name generation
#
# This plugin is called by agpod kiro to generate branch names.
# If it fails or returns empty, agpod will fall back to default generation.
#
# Environment variables available:
#   AGPOD_DESC          - The description provided by --desc flag
#   AGPOD_TEMPLATE      - The template name being used (e.g., "vue", "rust")
#   AGPOD_TIME_ISO      - Current time in ISO8601 format
#   AGPOD_BASE_DIR      - Base directory for PR drafts
#   AGPOD_REPO_ROOT     - Git repository root (if in a git repo)
#   AGPOD_USER          - Current user
#
# Output:
#   stdout: The generated branch name (single line)
#   stderr: Diagnostic messages (logged by agpod)
#   exit 0: Success, use stdout as branch name
#   exit non-zero: Failure, fall back to default generation

# Don't exit on error - we want to return empty string for fallback
set -uo pipefail

# Get environment variables
desc="${AGPOD_DESC:-}"
template="${AGPOD_TEMPLATE:-default}"

# Return empty if no description (triggers fallback)
if [[ -z "$desc" ]]; then
  echo ""
  exit 0
fi

# Simple slugify: convert to lowercase, replace spaces with hyphens
# This is a portable version that works on all systems
slug=$(echo "$desc" | tr '[:upper:]' '[:lower:]' | tr '[:space:]' '-' | tr -cd 'a-z0-9-')

# Remove multiple consecutive hyphens and trim leading/trailing hyphens
slug=$(echo "$slug" | sed -e 's/-\+/-/g' -e 's/^-//' -e 's/-$//')

# If slug is empty after sanitization, return empty for fallback
if [[ -z "$slug" ]]; then
  echo ""
  exit 0
fi

# Output branch name (just the slug, no prefix or random suffix)
echo "$slug"
