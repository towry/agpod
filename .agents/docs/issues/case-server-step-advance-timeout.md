# Issue: case-server-step-advance-timeout

- Status: verified
- Updated: 2026-04-17

## Problem

- Report: `agpod-mcp.case_step_advance` 常报 `database connection failed: timed out waiting for case-server response after 5000 ms`。
- Where: `agpod-mcp -> agpod-case::server_client -> agpod-case::server -> agpod-case::commands::cmd_step_advance`。
- When: 尤见于已有较多 step/entry 之活跃 case，近例为 `C-20cde22b-1e16-46dd-8708-f93b42853297` 于 `S-005/S-006/S-007` 推进时。
- Expected: `case_step_advance` 于 5 秒内完成，且并发 `case_current` / `case_finish` 不被无端饿死。
- Actual: 某单条 step advance 实耗 8-12 秒；其间其他请求卡在 server 全局 execution slot 前并超时。
- Trigger: MCP 或 CLI 发起 `Step::Advance`，附 `record` 与 `next_step_id` 时尤常见。

## Reproduction

- Steps:
  1. 以现有 case 数据运行 `case_step_advance`，推进活跃 step 并附 `record`。
  2. 在其执行期内并发调用 `case_current` 或 `case_finish`。
  3. 观察客户端 5 秒超时与 server 慢日志。
- Frequency: intermittent
- Scope: `agpod-case` server 串行执行路径；凡经 case-server 之请求皆受牵连。

## Context

- Files:
  - `crates/agpod-case/src/server.rs` — server 全局 gate 与请求包裹边界
  - `crates/agpod-case/src/commands.rs` — `cmd_step_advance` 之读写与返回组装
  - `crates/agpod-case/src/client.rs` — SurrealDB 查询、schema、事务 SQL
  - `crates/agpod-case/src/server_client.rs` — 外层 5 秒客户端超时
- Environment: 本地嵌入式 `surrealdb = "3"` + `kv-rocksdb`，local server 模式

## Evidence

- Static analysis:
  - `server.rs` 以单一 `db_gate: Mutex<()>` 串行整条请求，持锁范围包住 `execute_command_json(...).await` 全程。
  - `cmd_step_advance` 先后执行：`get_steps`、`get_entries`、`next_entry_seq(get_entry_count -> get_entries)`、`advance_step`、`get_case`、`get_steps`、`get_entries`、event dispatch。
  - `entry` 仅有 `idx_entry_case`，`step` 仅有 `idx_step_case`；而查询常为 `WHERE case_id ... ORDER BY seq/order_index`，缺复合索引。
  - `next_entry_seq` 现以全量 `get_entries(case_id)` 计数，非取 latest seq。
- Errors/logs:
  - `~/Library/Application Support/agpod/logs/agpod-case-server.log`：
    - `2026-04-17T02:31:59Z` `Step::Advance(S-006)` `elapsed_ms=12392`
    - `2026-04-17T02:37:08Z` `Step::Advance(S-007)` `elapsed_ms=10006`
    - `2026-04-17T02:37:07Z` `Current { state: true }` `timed out waiting on db gate`
    - `2026-04-17T03:22:51Z` `open_dispatch_opened elapsed_ms=5002`
    - `2026-04-17T03:24:03Z` `step_advance_dispatch_record elapsed_ms=5002`
    - `2026-04-17T03:24:08Z` `step_advance_dispatch_done elapsed_ms=5002`
  - `~/Library/Application Support/agpod/logs/agpod-mcp.log`：
    - 多次 `database connection failed: timed out waiting for case-server response after 5000 ms`
- Web research:
  - SurrealDB 官方 performance best practices 指出，带 `ORDER BY` 之查询若无可用索引，常退化为全扫描/排序；大事务与广泛扫描亦会拖慢本地引擎路径。

## Hypotheses

1. server 全局 gate 持锁过宽，将“数据库读写 + reload + 输出组装 + hooks”整段串行，致一条慢 advance 饿死其他请求。
2. 真正把单请求顶满 5s/10s 者，乃同步 Honcho hooks：`open` 一次 dispatch 约 5s，`step_advance` 两次 dispatch 叠至约 10s。
3. `cmd_step_advance` 自身原有多次全量 `entry/step` 读取，虽非当前主矛盾，仍放大写命令热路径成本。

## Attempts

- tried 外层 5 秒 timeout + 慢日志 + gate wait timeout → result: 仅阻止无穷挂起，未除 server 内慢路径。
- tried 观察客户端日志 → result: 可知“谁超时”，不足知“何处慢”；须结合 case-server 日志与代码路径。
- tried 读写分流 + `step_advance`/`open` 减扫 → result: 并发读已脱写 gate，热路径亦收窄；然若仍同步等待 hooks，`open`/`advance` 仍可被 Honcho 拖过 5s。
- tried hooks 改为同 case 有序后台队列 → result: 主响应不再候外部 sink，且同 case 事件不乱序。

## Fix Plan

- Next: none
- Why: 根因已证并已改。

## Verification

- Checks:
  - 增加 server 并发测试：慢 `Step::Advance` 执行时，`Current { state: true }` 不应再因 gate 饿死而超时。
  - 针对 `next_entry_seq` / `step_advance` 增测试，确保改用 latest seq 或更窄读取后语义不变。
  - 增 hooks 队列测试，确保后台投递不阻主链，且同 case 保序。
  - 运行定向 `cargo test -p agpod-case ...` 与 `cargo clippy -p agpod-case -- -D warnings`。
- Pass:
  - 并发只读请求可在慢写期间返回。
  - `Step::Advance` 行为、`record_entry`、next-step 语义不变。
  - hooks 回执仅示 `queued` / 初始化失败；慢 Honcho 不再致 MCP 5s 超时。
  - 无新增 clippy / fmt / 定向 test 失败。

## Resolution

- Root cause: `agpod-case` 将外部 Honcho hook 同步置于 mutation 主请求链上；`open` 与 `step_advance` 分别等待约 1 次 / 2 次 5 秒 dispatch，叠加 server 写 gate 串行，遂现 5s/10s 超时。DB 热路径偏重虽为次因，非当下主矛盾。
- Change: 已做三层收束：`server.rs` 读写分流；`client.rs`/`commands.rs` 收窄 `open` 与 `step_advance` 之 reload/全扫；`hooks.rs` 改为同 case 有序后台队列，mutation 响应仅回报 hook 已排队或初始化失败，不再等待外部送达。
- Verified: yes
