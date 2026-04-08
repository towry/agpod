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
- Resuming known case-aware work

## Core Rules

- All arguments are `--key value` (no positional args except `recall <query>`).
- Most mutating commands may omit `--id` to target the current open case.
- `case current` is the resume entrypoint; it returns current direction, steps, last decision/evidence, health, and next action.
- `case resume` no longer exists.
- `case open --step ...` may seed initial steps at open time; at most one step may set `start=true`.
- CLI `close` / `abandon` are destructive and require a returned `--confirm-token` on the second call.
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
#           --step "Read current code"
#           --step '{"title":"Trace failing path","reason":"reproduce root cause","start":true}'

# Show current case panel / resume entrypoint
agpod case current

# Show full case details
agpod case show --id C-YYYYMMDD-NN

# Close a case (goal met): first call returns confirm_token
agpod case close --summary "Outcome description"
# Then repeat only if intentional
agpod case close --summary "Outcome description" --confirm-token <token>

# Abandon a case (goal not met): same confirmation flow
agpod case abandon --summary "Why we stopped"
agpod case abandon --summary "Why we stopped" --confirm-token <token>
```

### Recording Events

```bash
# Record a finding/note/evidence/blocker
agpod case record --summary "What was found" \
  --kind finding           # note | finding | evidence | blocker
  --files "src/foo.rs"     # optional, comma-separated
  --context "extra detail" # optional

# Record a decision
agpod case decide \
  --summary "Chose approach B" --reason "Because X"
```

### Direction Changes

```bash
agpod case redirect \
  --direction "New approach" \
  --reason "Prior approach failed" \
  --context "What we learned" \
  --success-condition "..." \
  --abort-condition "..."
```

### Execution Steps

```bash
# Add a step
agpod case step add --title "Implement X"

# Start a step
agpod case step start --step-id C-YYYYMMDD-NN/S-001

# Mark step done
agpod case step done --step-id C-YYYYMMDD-NN/S-001

# Mark step blocked
agpod case step block --step-id C-YYYYMMDD-NN/S-001 \
  --reason "Blocked by dependency X"

# Reorder a step
agpod case step move --step-id C-YYYYMMDD-NN/S-001 \
  --before C-YYYYMMDD-NN/S-003

# Complete active step and optionally start the next one
agpod case step advance --record "Captured root cause" --next-step-auto
```

### Search & Handoff

```bash
# Search past cases by goal text (case-insensitive)
agpod case recall "search query"

# List all cases in this repo
agpod case list

# Resume current work
agpod case current
```

## Typical Agent Workflow

```bash
# 1. Open case
agpod case open --goal "Fix auth timeout" --direction "Check token expiry logic"

# 2. Add and start steps
agpod case step add --title "Read auth module"
agpod case step start --step-id C-20260320-01/S-001

# 3. Record findings as you go
agpod case record --summary "Token TTL is hardcoded to 5s" --kind finding

# 4. Make decisions
agpod case decide --summary "Increase TTL to 300s" --reason "5s too short for API calls"

# 5. Complete step
agpod case step done --step-id C-20260320-01/S-001

# 6. Close case (second call uses returned token)
agpod case close --summary "Fixed token TTL from 5s to 300s"
agpod case close --summary "Fixed token TTL from 5s to 300s" --confirm-token <token>
```
