# Case Implementation Plan

本文记本轮 `case` 工具实现之落地次序与 smoke 要求。

当前先落者：

- `case_open.needed_context_query`
- `startup_context`
- `case_step_advance`
- `entry.step_id`

下一批高收益收口：

- 并 `case_resume` 入 `case_current`
- 去诸变更口对 `case_id` 之常态强依赖
- 诸变更口返回稳定 `case.id` / `case_id`
- 扩 `case_open` 以纳初始 `steps`
- 改 `case_finish` 为单次结案

---

## 1. 实施次序

1. 扩 CLI / MCP open 输入
2. 命令层在 `case_open(mode=new)` 组装 startup context
3. 返回 `startup_context` 与 `startup_context_status`
4. 补最小单测
5. 做旧数据兼容 smoke
6. 为 `entry` 补 `step_id`
7. 以存储层事务实现 `case_step_advance`
8. 暴露 CLI / MCP `step advance`
9. 补 `step advance` 单测与全测

后续收口建议次序：

10. 收 `case_current` 与 `case_resume`
11. 令 `case_record` / `case_decide` / `case_redirect` / `case_steps_add` / `case_step_move` / `case_step_advance` / `case_finish` 之 `id` 改为可省
12. 统一诸变更口成功返回之 `case.id` / `case_id`
13. 扩 `case_open(steps=..., needed_context_query=...)`
14. 去 `case_finish.confirm_token`

---

## 2. 旧数据兼容 smoke

### 2.1 目的

避免新功能完成后，对旧 case 数据目录读取时报不兼容错误。

### 2.2 步骤

1. 在功能实现前，保留一份旧 `AGPOD_CASE_DATA_DIR` 备份
2. 完成实现后，以该备份副本启动新二进制
3. 在旧数据副本上运行：
   - `agpod case current --json`
   - `agpod case show --json`
   - `agpod case list --json`
   - 一条读取旧记录之 `agpod case recall --query ... --json`

### 2.3 本轮新增功能之最小 smoke

建议：

1. 用旧数据副本确认：
   - `current` / `show` 可读
   - 旧记录缺新字段时不崩
   - `recall` 可读旧记录
2. 再用一份全新空数据目录执行：
   - `agpod case open --goal ... --direction ... --how-to ... --doc-about ... --json`
   - `agpod case step advance --step-id ... --record-summary ... --next-step-auto --json`
3. 确认返回含：
   - `startup_context`
   - `startup_context_status`
   - `completed_step`
   - 可选 `record_entry`
   - 可选 `started_step`

### 2.4 完成判据

- 旧数据读取不报错
- 新 open 返回结构符合设计
- 空命中时 `case_open.ok = true`
- `startup_context_status` 按设计落入 `ok|empty|degraded`
- `step advance` 成功后无需立刻补调 `case_current`

## 3. 下一批契约完成判据

- `case_current` 一口可替代旧 `case_resume`
- 高频变更口在“当前 open case”场景下不必再手填 `case_id`
- 上述工具返回中稳定可见当前操作 `case_id`
- `case_open` 可一并立初始 step 队列
- `case_finish` 对 MCP/agent 为单次调用
