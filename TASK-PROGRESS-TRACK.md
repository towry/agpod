# Task Progress Track

- Track: agpod mcp hive tool contraction
- Done-when: agpod-mcp exposes one hive tool with repo-scoped process-backed Claude workers, max-5 agents, caller-controlled resume, and mode config introspection
- Updated: 2026-04-08

## Tasks
- [x] plan: stabilize hive tool contract
- [x] test: verify with tests and review
- [x] smoke-case: smoke core case tool flows on the current repo case
- [x] handoff: write restart handoff summary for current hive smoke work
- [x] config-mode: define hive mode introspection and required mode config
- [x] proc: replace tmux hive runtime with local child-process manager
- [x] resume: add optional Claude session resume controlled by caller
- [x] docs: document process-mode lifecycle and edge-case handling
- [x] impl: implement process-backed hive state and tool
- [x] hooks: persist worker output files and session metadata
- [x] smoke-hive-env: finish hive end-to-end smoke for process-backed runtime
- [x] output-wrap: define provider output envelope and parsing boundary
- [x] probe: add minimal hive probe-style tests for provider output

## Document References
- docs/hive-claude-modes.md - Claude-only hive mode config and mode_info contract
