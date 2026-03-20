---
name: agpod-case
description: "Use this skill when working with agpod case exploration tracker. Trigger for requests about opening/closing cases, recording findings/decisions, managing execution steps, redirecting direction, and searching past cases."
---

# Agpod Case — Exploration Case Tracker

## When to Use

Load this skill when the user mentions:
- `agpod case`, `case tracker`, `exploration case`
- Opening/closing/abandoning a case
- Recording findings, evidence, decisions, blockers
- Managing execution steps
- Redirecting investigation direction
- Searching or recalling past cases
- Handoff / resume brief

## Core Rules

- All arguments are `--key value` (no positional args except `recall <query>`).
- `--id` is the case ID (e.g., `C-20260320-01`).
- Step IDs follow the pattern `<case-id>/S-NNN`.
- `--json` flag on any command outputs machine-readable JSON.
- Data is stored locally at `$XDG_DATA_HOME/agpod/case.db` (SurrealDB embedded, RocksDB backend). Override with `--data-dir` or `AGPOD_CASE_DATA_DIR`.

## Lifecycle

```
open → [record / decide / step add/start/done] → close
                    ↓
               redirect (change direction)
                    ↓
               abandon (give up)
```

## Command Reference

### Case Lifecycle

```bash
# Open a new case
agpod case open --goal "Investigate X" --direction "Try approach A"
# Optional: --goal-constraint '{"rule":"...","reason":"..."}'
#           --constraint '{"rule":"...","reason":"..."}'
#           --success-condition "..." --abort-condition "..."

# Show current case panel
agpod case current

# Show full case details
agpod case show --id C-YYYYMMDD-NN

# Close a case (goal met)
agpod case close --id C-YYYYMMDD-NN --summary "Outcome description"

# Abandon a case (goal not met)
agpod case abandon --id C-YYYYMMDD-NN --summary "Why we stopped"
```

### Recording Events

```bash
# Record a finding/note/evidence/blocker
agpod case record --id C-YYYYMMDD-NN --summary "What was found" \
  --kind finding           # note | finding | evidence | blocker
  --files "src/foo.rs"     # optional, comma-separated
  --context "extra detail" # optional

# Record a decision
agpod case decide --id C-YYYYMMDD-NN \
  --summary "Chose approach B" --reason "Because X"
```

### Direction Changes

```bash
agpod case redirect --id C-YYYYMMDD-NN \
  --direction "New approach" \
  --reason "Prior approach failed" \
  --context "What we learned" \
  --success-condition "..." \
  --abort-condition "..."
```

### Execution Steps

```bash
# Add a step
agpod case step add --id C-YYYYMMDD-NN --title "Implement X"

# Start a step
agpod case step start --id C-YYYYMMDD-NN --step-id C-YYYYMMDD-NN/S-001

# Mark step done
agpod case step done --id C-YYYYMMDD-NN --step-id C-YYYYMMDD-NN/S-001

# Mark step blocked
agpod case step block --id C-YYYYMMDD-NN --step-id C-YYYYMMDD-NN/S-001

# Reorder a step
agpod case step move --id C-YYYYMMDD-NN --step-id C-YYYYMMDD-NN/S-001 --position 3
```

### Search & Handoff

```bash
# Search past cases (case-insensitive)
agpod case recall "search query"

# List all cases in this repo
agpod case list

# Resume brief for handoff
agpod case resume              # defaults to open case
agpod case resume --id C-YYYYMMDD-NN
```

## Typical Agent Workflow

```bash
# 1. Open case
agpod case open --goal "Fix auth timeout" --direction "Check token expiry logic"

# 2. Add and start steps
agpod case step add --id C-20260320-01 --title "Read auth module"
agpod case step start --id C-20260320-01 --step-id C-20260320-01/S-001

# 3. Record findings as you go
agpod case record --id C-20260320-01 --summary "Token TTL is hardcoded to 5s" --kind finding

# 4. Make decisions
agpod case decide --id C-20260320-01 --summary "Increase TTL to 300s" --reason "5s too short for API calls"

# 5. Complete step
agpod case step done --id C-20260320-01 --step-id C-20260320-01/S-001

# 6. Close case
agpod case close --id C-20260320-01 --summary "Fixed token TTL from 5s to 300s"
```
