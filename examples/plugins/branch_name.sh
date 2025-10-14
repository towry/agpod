#!/usr/bin/env bash
# Example plugin for custom branch name generation
set -euo pipefail

desc="${AGPOD_DESC:-}"
prefix="${AGPOD_BRANCH_PREFIX:-feature-impl}"
template="${AGPOD_TEMPLATE:-default}"

if [[ -z "$desc" ]]; then
  echo "Error: desc is empty" >&2
  exit 2
fi

# Simple slugify (preserves Chinese, replaces spaces with hyphens)
slug="$(echo "$desc" | tr '[:space:]' '-' | sed -E 's/[^[:alnum:]._\\-一-龥]+/-/g' | sed -E 's/-+/-/g' | sed -E 's/^-|-$//g')"

# Generate random suffix
rand="$(LC_ALL=C tr -dc 'a-z0-9' </dev/urandom | head -c 6)"

# Output branch name
echo "${prefix}-${slug}-${rand}"
